//! Per-account disk cache. Concurrent downloads share a lock and pinned audio is not evicted.
use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, Weak},
    time::{Duration, SystemTime},
};

pub const DEFAULT_AUDIO_LIMIT: u64 = 2 * 1024 * 1024 * 1024;
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheStats {
    pub audio_bytes: u64,
    pub audio_files: usize,
    pub cover_bytes: u64,
    pub metadata_bytes: u64,
    pub audio_limit_bytes: u64,
    pub audio_over_limit: bool,
}
#[derive(Clone)]
pub struct Cache {
    root: PathBuf,
    state: Arc<Mutex<State>>,
}
struct State {
    pins: HashMap<PathBuf, usize>,
    locks: HashMap<PathBuf, Weak<Mutex<()>>>,
    audio_limit: u64,
    writes: usize,
    over_limit: bool,
    metadata_generation: u64,
}
impl Default for State {
    fn default() -> Self {
        Self {
            pins: HashMap::new(),
            locks: HashMap::new(),
            audio_limit: DEFAULT_AUDIO_LIMIT,
            writes: 0,
            over_limit: false,
            metadata_generation: 0,
        }
    }
}
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}
static CACHE_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<Mutex<State>>>>> = OnceLock::new();
fn shared_state(path: &Path) -> Arc<Mutex<State>> {
    let registry = CACHE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let mut registry = lock(registry);
    if let Some(existing) = registry.get(path).and_then(Weak::upgrade) {
        return existing;
    }
    registry.retain(|_, state| state.strong_count() > 0);
    let state = Arc::new(Mutex::new(State::default()));
    registry.insert(path.to_path_buf(), Arc::downgrade(&state));
    state
}
fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

/// Pins an audio file, including while it is being downloaded.
pub struct CacheLease {
    cache: Cache,
    path: PathBuf,
}
impl CacheLease {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for CacheLease {
    fn drop(&mut self) {
        let mut state = lock(&self.cache.state);
        if let Some(count) = state.pins.get_mut(&self.path) {
            *count -= 1;
            if *count == 0 {
                state.pins.remove(&self.path);
            }
        }
        if state.over_limit {
            let _ = self.cache.trim_locked(&mut state);
        }
    }
}
struct WriteGuard(Cache);
impl Drop for WriteGuard {
    fn drop(&mut self) {
        lock(&self.0.state).writes -= 1;
    }
}
impl Cache {
    pub fn for_account(base: &str, username: &str) -> Option<Self> {
        let dirs = directories::ProjectDirs::from("org", "clify", "clify")?;
        Some(Self::at(dirs.cache_dir(), base, username))
    }
    pub fn at(root: &Path, base: &str, username: &str) -> Self {
        let root = root.join(hash(&format!("{}\0{}", base, username)));
        let state = shared_state(&root);
        Self { root, state }
    }
    fn path(&self, kind: &str, key: &str) -> PathBuf {
        self.root.join(kind).join(hash(key))
    }
    pub(crate) fn scope(&self) -> &Path {
        &self.root
    }
    pub fn pin_audio(&self, key: &str) -> CacheLease {
        let path = self.path("audio", key);
        *lock(&self.state).pins.entry(path.clone()).or_default() += 1;
        CacheLease {
            cache: self.clone(),
            path,
        }
    }
    pub fn download_lock(&self, key: &str) -> Arc<Mutex<()>> {
        let path = self.path("audio", key);
        let mut state = lock(&self.state);
        if let Some(existing) = state.locks.get(&path).and_then(Weak::upgrade) {
            return existing;
        }
        state.locks.retain(|_, value| value.strong_count() > 0);
        let next = Arc::new(Mutex::new(()));
        state.locks.insert(path, Arc::downgrade(&next));
        next
    }
    pub fn get(&self, kind: &str, key: &str, max_age: Option<Duration>) -> Option<PathBuf> {
        let path = self.path(kind, key);
        let metadata = fs::metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() == 0 {
            return None;
        }
        if let Some(age) = max_age {
            if SystemTime::now()
                .duration_since(metadata.modified().ok()?)
                .ok()?
                > age
            {
                return None;
            }
        }
        if kind == "audio" {
            if let Ok(file) = File::open(&path) {
                let _ = file.set_times(fs::FileTimes::new().set_accessed(SystemTime::now()));
            }
        }
        Some(path)
    }
    pub fn read(&self, kind: &str, key: &str, max_age: Option<Duration>) -> Option<Vec<u8>> {
        fs::read(self.get(kind, key, max_age)?).ok()
    }
    pub fn metadata_generation(&self) -> u64 {
        lock(&self.state).metadata_generation
    }
    pub fn write_metadata_if_current(
        &self,
        key: &str,
        bytes: &[u8],
        generation: u64,
    ) -> io::Result<PathBuf> {
        self.write_inner("metadata", key, bytes, 10 * 1024 * 1024, Some(generation))
    }
    pub fn write(
        &self,
        kind: &str,
        key: &str,
        reader: impl Read,
        max_bytes: u64,
    ) -> io::Result<PathBuf> {
        self.write_inner(kind, key, reader, max_bytes, None)
    }
    fn write_inner(
        &self,
        kind: &str,
        key: &str,
        reader: impl Read,
        max_bytes: u64,
        generation: Option<u64>,
    ) -> io::Result<PathBuf> {
        {
            lock(&self.state).writes += 1;
        }
        let _guard = WriteGuard(self.clone());
        let final_path = self.path(kind, key);
        let directory = final_path.parent().unwrap();
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.root, fs::Permissions::from_mode(0o700))?;
        }
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(20)
            .map(char::from)
            .collect();
        let temporary = directory.join(format!(".partial-{suffix}"));
        let result = (|| {
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temporary)?;
            let mut limited = reader.take(max_bytes.saturating_add(1));
            let size = io::copy(&mut limited, &mut file)?;
            if size == 0 || size > max_bytes {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Кэшируемый файл пуст или слишком велик",
                ));
            }
            file.flush()?;
            file.sync_all()?;
            drop(file);
            let state = lock(&self.state);
            if generation.is_some_and(|value| value != state.metadata_generation) {
                return Err(io::Error::other("Данные кэша устарели"));
            }
            #[cfg(windows)]
            if final_path.exists() {
                fs::remove_file(&final_path)?;
            }
            fs::rename(&temporary, &final_path)?;
            drop(state);
            Ok(final_path.clone())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        if result.is_ok() && kind == "audio" {
            let mut state = lock(&self.state);
            let _ = self.trim_locked(&mut state);
        }
        result
    }
    fn trim_locked(&self, state: &mut State) -> io::Result<()> {
        let directory = self.root.join("audio");
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                state.over_limit = false;
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        let mut files = Vec::new();
        let mut total = 0u64;
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if let Ok(metadata) = entry.metadata() {
                if metadata.is_file() {
                    total = total.saturating_add(metadata.len());
                    files.push((
                        entry.path(),
                        metadata.len(),
                        metadata
                            .accessed()
                            .or_else(|_| metadata.modified())
                            .unwrap_or(SystemTime::UNIX_EPOCH),
                    ));
                }
            }
        }
        files.sort_by_key(|(_, _, time)| *time);
        for (path, size, _) in files {
            if total <= state.audio_limit {
                break;
            }
            if !state.pins.contains_key(&path) && fs::remove_file(path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
        state.over_limit = total > state.audio_limit;
        Ok(())
    }
    /// Remove abandoned temporary files; recent files can still belong to a live download.
    pub fn cleanup_partials(&self, age: Duration) -> io::Result<()> {
        for kind in ["audio", "covers", "metadata"] {
            let entries = match fs::read_dir(self.root.join(kind)) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for entry in entries.flatten() {
                if !entry.file_name().to_string_lossy().starts_with(".partial-") {
                    continue;
                }
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata
                    .modified()
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .is_some_and(|elapsed| elapsed >= age)
                {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        Ok(())
    }
    pub fn set_audio_limit(&self, bytes: u64) -> io::Result<()> {
        self.cleanup_partials(Duration::from_secs(24 * 60 * 60))?;
        let mut state = lock(&self.state);
        state.audio_limit = bytes;
        self.trim_locked(&mut state)
    }
    pub fn stats(&self) -> io::Result<CacheStats> {
        let mut stats = CacheStats::default();
        for kind in ["audio", "covers", "metadata"] {
            let entries = match fs::read_dir(self.root.join(kind)) {
                Ok(v) => v,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let metadata = entry.metadata()?;
                if !metadata.is_file() {
                    continue;
                }
                match kind {
                    "audio" => {
                        stats.audio_bytes = stats.audio_bytes.saturating_add(metadata.len());
                        stats.audio_files += 1;
                    }
                    "covers" => {
                        stats.cover_bytes = stats.cover_bytes.saturating_add(metadata.len())
                    }
                    _ => stats.metadata_bytes = stats.metadata_bytes.saturating_add(metadata.len()),
                }
            }
        }
        stats.audio_limit_bytes = lock(&self.state).audio_limit;
        stats.audio_over_limit = stats.audio_bytes > stats.audio_limit_bytes;
        Ok(stats)
    }
    pub fn invalidate(&self, kind: &str) {
        let mut state = lock(&self.state);
        if kind == "metadata" {
            state.metadata_generation = state.metadata_generation.wrapping_add(1);
        }
        let _ = fs::remove_dir_all(self.root.join(kind));
    }
    pub fn clear(&self) -> io::Result<()> {
        let mut state = lock(&self.state);
        if !state.pins.is_empty() || state.writes > 0 {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Кэш используется плеером или загрузкой",
            ));
        }
        match fs::remove_dir_all(&self.root) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
            Ok(()) => {}
        }
        state.metadata_generation = state.metadata_generation.wrapping_add(1);
        Ok(())
    }
    pub fn path_for(&self, kind: &str, key: &str) -> PathBuf {
        self.path(kind, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cache_separates_accounts_and_never_keeps_partial_files() {
        let root = std::env::temp_dir().join(format!("clify-cache-test-{}", rand::random::<u64>()));
        let first = Cache::at(&root, "https://server/", "a");
        let second = Cache::at(&root, "https://server/", "b");
        first.write("audio", "track", &b"one"[..], 10).unwrap();
        assert_eq!(first.read("audio", "track", None).unwrap(), b"one");
        assert!(second.read("audio", "track", None).is_none());
        assert!(first.write("audio", "broken", &b"too long"[..], 3).is_err());
        assert!(first.get("audio", "broken", None).is_none());
        assert_eq!(fs::read_dir(first.root.join("audio")).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn eviction_uses_recent_access() {
        let root = std::env::temp_dir().join(format!("clify-lru-test-{}", rand::random::<u64>()));
        let cache = Cache::at(&root, "server", "user");
        cache.set_audio_limit(100).unwrap();
        cache.write("audio", "old", &b"1111"[..], 10).unwrap();
        cache.write("audio", "new", &b"2222"[..], 10).unwrap();
        let old = cache.path_for("audio", "old");
        let new = cache.path_for("audio", "new");
        let ancient = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        File::open(&old)
            .unwrap()
            .set_times(fs::FileTimes::new().set_accessed(ancient))
            .unwrap();
        File::open(&new)
            .unwrap()
            .set_times(fs::FileTimes::new().set_accessed(ancient + Duration::from_secs(100)))
            .unwrap();
        cache.get("audio", "old", None).unwrap();
        cache.set_audio_limit(4).unwrap();
        assert!(old.exists());
        assert!(!new.exists());
        cache.clear().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn removes_only_abandoned_partial_files() {
        let root =
            std::env::temp_dir().join(format!("clify-partials-test-{}", rand::random::<u64>()));
        let cache = Cache::at(&root, "server", "user");
        fs::create_dir_all(cache.root.join("audio")).unwrap();
        let old = cache.root.join("audio/.partial-old");
        let recent = cache.root.join("audio/.partial-recent");
        let normal = cache.root.join("audio/data");
        fs::write(&old, b"old").unwrap();
        fs::write(&recent, b"recent").unwrap();
        fs::write(&normal, b"data").unwrap();
        File::open(&old)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH))
            .unwrap();
        cache.cleanup_partials(Duration::from_secs(3600)).unwrap();
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(normal.exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn eviction_never_removes_pinned_audio_and_clear_is_blocked() {
        let root =
            std::env::temp_dir().join(format!("clify-cache-limit-test-{}", rand::random::<u64>()));
        let cache = Cache::at(&root, "server", "user");
        cache.set_audio_limit(5).unwrap();
        let first = cache.pin_audio("first");
        cache.write("audio", "first", &b"1234"[..], 10).unwrap();
        let second = cache.pin_audio("second");
        cache.write("audio", "second", &b"5678"[..], 10).unwrap();
        assert!(first.path().exists() && second.path().exists());
        assert!(cache.clear().is_err());
        drop(first);
        assert!(!cache.path_for("audio", "first").exists());
        assert!(second.path().exists());
        drop(second);
        assert!(cache.stats().unwrap().audio_bytes <= 5);
        cache.clear().unwrap();
        fs::remove_dir_all(root).unwrap();
    }
}
