//! Small, UI-independent Subsonic client. Passwords only live in memory.
pub mod cache;
pub mod lyrics;
pub mod prefetch;
pub mod queue;
pub mod sleep_timer;
use cache::{Cache, CacheLease, CacheStats};
use md5::{Digest, Md5};
use rand::{distributions::Alphanumeric, seq::SliceRandom, Rng};
use serde::Deserialize;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

pub(crate) fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// A cached audio file protected against eviction while it is in use.
pub struct CachedAudio {
    lease: CacheLease,
}
impl CachedAudio {
    pub fn path(&self) -> &Path {
        self.lease.path()
    }
}

struct CancellableReader<'a, R> {
    inner: R,
    cancelled: &'a dyn Fn() -> bool,
}
impl<R: Read> Read for CancellableReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if (self.cancelled)() {
            return Err(io::Error::other("Загрузка отменена"));
        }
        let result = self.inner.read(buf)?;
        if (self.cancelled)() {
            return Err(io::Error::other("Загрузка отменена"));
        }
        Ok(result)
    }
}

struct ApiFailure {
    message: String,
    may_use_stale: bool,
}

#[derive(Clone)]
pub struct Client {
    base: Url,
    username: String,
    password: String,
    http: reqwest::blocking::Client,
    cache: Option<Cache>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Song {
    pub id: String,
    pub title: String,
    #[serde(default, deserialize_with = "null_default")]
    pub artist: String,
    #[serde(default, deserialize_with = "null_default")]
    pub album: String,
    #[serde(default, deserialize_with = "null_default")]
    pub duration: u64,
    #[serde(default)]
    pub cover_art: Option<String>,
    #[serde(default)]
    pub user_rating: Option<u8>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Album {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub artist: String,
    #[serde(default, deserialize_with = "null_default")]
    pub song_count: u64,
    #[serde(default)]
    pub cover_art: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artist {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub album_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default, deserialize_with = "null_default")]
    pub song_count: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQueue {
    #[serde(default, rename = "entry", deserialize_with = "null_default")]
    pub songs: Vec<Song>,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default, deserialize_with = "null_default")]
    pub position: u64,
}

impl Client {
    pub fn new(base: &str, username: &str, password: &str) -> Result<Self, String> {
        let mut url =
            Url::parse(base.trim()).map_err(|_| "Некорректный адрес сервера".to_string())?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("Укажите HTTP(S)-адрес сервера без логина, параметров и фрагмента".into());
        }
        if username.trim().is_empty() || password.is_empty() {
            return Err("Введите логин и пароль".into());
        }
        let path = format!("{}/", url.path().trim_end_matches('/'));
        url.set_path(&path);
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(10))
            .build()
            .map_err(|e| e.to_string())?;
        let cache = Cache::for_account(url.as_str(), username);
        Ok(Self {
            base: url,
            username: username.into(),
            password: password.into(),
            http,
            cache,
        })
    }
    /// Set the cache root, useful for platform integrations and isolated tests.
    pub fn with_cache_root(mut self, root: &std::path::Path) -> Self {
        self.cache = Some(Cache::at(root, self.base.as_str(), &self.username));
        self
    }
    fn auth(&self) -> Vec<(String, String)> {
        let salt: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(16)
            .map(char::from)
            .collect();
        let token = format!(
            "{:x}",
            Md5::digest(format!("{}{}", self.password, salt).as_bytes())
        );
        vec![
            ("u".into(), self.username.clone()),
            ("t".into(), token),
            ("s".into(), salt),
            ("v".into(), "1.16.1".into()),
            ("c".into(), "clify".into()),
            ("f".into(), "json".into()),
        ]
    }
    fn endpoint(&self, method: &str) -> Result<Url, String> {
        self.base
            .join(&format!("rest/{method}.view"))
            .map_err(|_| "Некорректный адрес API".into())
    }
    fn call(&self, method: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, String> {
        self.call_detailed(method, params).map_err(|e| e.message)
    }
    fn call_detailed(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Value, ApiFailure> {
        let mut form = self.auth();
        form.extend(params.iter().map(|(k, v)| (k.to_string(), v.to_string())));
        let endpoint = self.endpoint(method).map_err(|message| ApiFailure {
            message,
            may_use_stale: false,
        })?;
        let response = self.http.post(endpoint).form(&form).send().map_err(|e| {
            let may_use_stale = e.is_timeout() || e.is_connect();
            ApiFailure {
                message: e.without_url().to_string(),
                may_use_stale,
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(ApiFailure {
                message: format!("Сервер вернул HTTP {status}"),
                may_use_stale: status.is_server_error()
                    || status == reqwest::StatusCode::REQUEST_TIMEOUT,
            });
        }
        let json: serde_json::Value = response.json().map_err(|e| ApiFailure {
            message: e.without_url().to_string(),
            may_use_stale: false,
        })?;
        let body = json.get("subsonic-response").ok_or_else(|| ApiFailure {
            message: "Неверный ответ Subsonic API".into(),
            may_use_stale: false,
        })?;
        if body.get("status").and_then(|v| v.as_str()) != Some("ok") {
            return Err(ApiFailure {
                message: body
                    .pointer("/error/message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Ошибка Subsonic API")
                    .into(),
                may_use_stale: false,
            });
        }
        Ok(body.clone())
    }
    fn cached_call(
        &self,
        method: &str,
        params: &[(&str, &str)],
    ) -> Result<serde_json::Value, String> {
        let key = serde_json::to_string(&(method, params)).map_err(|e| e.to_string())?;
        if let Some(cache) = &self.cache {
            if let Some(value) = cache
                .read("metadata", &key, Some(Duration::from_secs(300)))
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            {
                return Ok(value);
            }
        }
        for attempt in 0..2 {
            let generation = self.cache.as_ref().map(Cache::metadata_generation);
            match self.call_detailed(method, params) {
                Ok(value) => {
                    if let (Some(cache), Some(generation), Ok(bytes)) =
                        (&self.cache, generation, serde_json::to_vec(&value))
                    {
                        let _ = cache.write_metadata_if_current(&key, &bytes, generation);
                        if cache.metadata_generation() != generation {
                            if attempt == 0 {
                                continue;
                            }
                            return Err(
                                "Данные изменились во время загрузки; повторите запрос".into()
                            );
                        }
                    }
                    return Ok(value);
                }
                Err(error) => {
                    if error.may_use_stale {
                        if let Some(value) = self
                            .cache
                            .as_ref()
                            .and_then(|c| c.read("metadata", &key, None))
                            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
                        {
                            return Ok(value);
                        }
                    }
                    return Err(error.message);
                }
            }
        }
        Err("Не удалось обновить данные".into())
    }
    pub fn invalidate_metadata(&self) {
        if let Some(cache) = &self.cache {
            cache.invalidate("metadata");
        }
    }
    pub fn clear_cache(&self) -> Result<(), String> {
        self.cache
            .as_ref()
            .ok_or("Каталог кэша недоступен")?
            .clear()
            .map_err(|e| format!("Не удалось очистить кэш: {e}"))
    }
    pub fn set_audio_cache_limit(&self, bytes: u64) -> Result<(), String> {
        self.cache
            .as_ref()
            .ok_or("Каталог кэша недоступен")?
            .set_audio_limit(bytes)
            .map_err(|e| format!("Не удалось ограничить кэш: {e}"))
    }
    pub fn cache_stats(&self) -> Result<CacheStats, String> {
        self.cache
            .as_ref()
            .ok_or("Каталог кэша недоступен")?
            .stats()
            .map_err(|e| format!("Не удалось прочитать размер кэша: {e}"))
    }
    pub(crate) fn cache_scope(&self) -> Option<PathBuf> {
        Some(self.cache.as_ref()?.scope().to_path_buf())
    }
    pub fn has_cached_audio(&self, id: &str) -> bool {
        self.cache
            .as_ref()
            .and_then(|cache| cache.get("audio", id, None))
            .is_some()
    }
    /// Return the current cache path. Use `acquire_audio` when the file must remain available.
    pub fn cached_audio(&self, id: &str) -> Result<PathBuf, String> {
        Ok(self.acquire_audio(id)?.path().to_path_buf())
    }
    pub fn acquire_audio(&self, id: &str) -> Result<CachedAudio, String> {
        self.acquire_audio_with_cancel(id, &|| false)
    }
    pub fn acquire_audio_with_cancel(
        &self,
        id: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<CachedAudio, String> {
        let cache = self.cache.as_ref().ok_or("Каталог кэша недоступен")?;
        if cancelled() {
            return Err("Загрузка отменена".into());
        }
        let lease = cache.pin_audio(id);
        let lock = cache.download_lock(id);
        let _download = loop {
            if cancelled() {
                return Err("Загрузка отменена".into());
            }
            match lock.try_lock() {
                Ok(guard) => break guard,
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(Duration::from_millis(40))
                }
                Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
            }
        };
        if cache.get("audio", id, None).is_some() {
            return Ok(CachedAudio { lease });
        }
        let mut last_error = String::new();
        for attempt in 0..3 {
            if cancelled() {
                return Err("Загрузка отменена".into());
            }
            if attempt > 0 {
                let delay = Duration::from_millis(200 * (1 << (attempt - 1)));
                let started = std::time::Instant::now();
                while started.elapsed() < delay {
                    if cancelled() {
                        return Err("Загрузка отменена".into());
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            let response = match self
                .http
                .get(self.media_url("stream", id)?)
                .timeout(Duration::from_secs(600))
                .send()
            {
                Ok(response) => response,
                Err(error) => {
                    let retry = error.is_timeout() || error.is_connect();
                    last_error = error.without_url().to_string();
                    if retry && attempt < 2 {
                        continue;
                    }
                    return Err(last_error);
                }
            };
            let status = response.status();
            if !status.is_success() {
                last_error = format!("Сервер вернул HTTP {status}");
                // Long Retry-After values must not be bypassed by our short backoff.
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok());
                let may_retry =
                    status.is_server_error() || status == reqwest::StatusCode::REQUEST_TIMEOUT;
                if may_retry && retry_after.is_none() && attempt < 2 {
                    continue;
                }
                return Err(last_error);
            }
            if status != reqwest::StatusCode::OK {
                return Err(format!("Сервер вернул неполный ответ: HTTP {status}"));
            }
            if response
                .content_length()
                .is_some_and(|size| size > 1024 * 1024 * 1024)
            {
                return Err("Трек превышает максимальный размер кэша (1 ГиБ)".into());
            }
            let kind = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if kind.contains("json") || kind.contains("xml") || kind.contains("text/html") {
                return Err("Сервер вернул ошибку вместо аудио".into());
            }
            let reader = CancellableReader {
                inner: response,
                cancelled,
            };
            match cache.write("audio", id, reader, 1024 * 1024 * 1024) {
                Ok(_) => return Ok(CachedAudio { lease }),
                Err(error) => {
                    if cancelled() {
                        return Err("Загрузка отменена".into());
                    }
                    let retry = matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::BrokenPipe
                    );
                    last_error = format!("Не удалось сохранить трек в кэш: {error}");
                    if !retry || attempt == 2 {
                        return Err(last_error);
                    }
                }
            }
        }
        Err(last_error)
    }
    pub fn cached_cover_path(&self, id: &str) -> Option<std::path::PathBuf> {
        self.cache.as_ref()?.get("covers", id, None)
    }
    pub fn ping(&self) -> Result<(), String> {
        self.call("ping", &[]).map(|_| ())
    }
    fn songs_at(value: &serde_json::Value, path: &str) -> Result<Vec<Song>, String> {
        serde_json::from_value(
            value
                .pointer(path)
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|e| format!("Ошибка чтения треков: {e}"))
    }
    pub fn albums(&self, offset: usize, count: usize) -> Result<Vec<Album>, String> {
        let response = self.cached_call(
            "getAlbumList2",
            &[
                ("type", "alphabeticalByName"),
                ("offset", &offset.to_string()),
                ("size", &count.clamp(1, 500).to_string()),
            ],
        )?;
        serde_json::from_value(
            response
                .pointer("/albumList2/album")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|e| format!("Ошибка чтения альбомов: {e}"))
    }
    pub fn artists(&self) -> Result<Vec<Artist>, String> {
        let response = self.cached_call("getArtists", &[])?;
        let indices = response
            .pointer("/artists/index")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut result = Vec::new();
        for index in indices {
            let artists: Vec<Artist> = serde_json::from_value(
                index
                    .get("artist")
                    .filter(|value| !value.is_null())
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!([])),
            )
            .map_err(|e| format!("Ошибка чтения исполнителей: {e}"))?;
            result.extend(artists);
        }
        Ok(result)
    }
    pub fn artist_albums(&self, id: &str) -> Result<Vec<Album>, String> {
        let response = self.cached_call("getArtist", &[("id", id)])?;
        serde_json::from_value(
            response
                .pointer("/artist/album")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|e| format!("Ошибка чтения альбомов: {e}"))
    }
    pub fn album_songs(&self, id: &str) -> Result<Vec<Song>, String> {
        Self::songs_at(&self.cached_call("getAlbum", &[("id", id)])?, "/album/song")
    }
    pub fn lyrics(&self, id: &str) -> Result<Vec<lyrics::Lyrics>, String> {
        let response = self.cached_call("getLyricsBySongId", &[("id", id)])?;
        let values = response
            .pointer("/lyricsList/structuredLyrics")
            .filter(|value| !value.is_null())
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        serde_json::from_value(values).map_err(|e| format!("Ошибка чтения текста песни: {e}"))
    }
    pub fn song(&self, id: &str) -> Result<Song, String> {
        let value = self.cached_call("getSong", &[("id", id)])?;
        serde_json::from_value(value.get("song").cloned().ok_or("Сервер не вернул трек")?)
            .map_err(|e| format!("Ошибка чтения трека: {e}"))
    }
    pub fn scrobble(&self, id: &str, submission: bool) -> Result<(), String> {
        self.call(
            "scrobble",
            &[
                ("id", id),
                ("submission", if submission { "true" } else { "false" }),
            ],
        )
        .map(|_| ())
    }
    pub fn random_songs(&self) -> Result<Vec<Song>, String> {
        Self::songs_at(
            &self.call("getRandomSongs", &[("size", "50")])?,
            "/randomSongs/song",
        )
    }
    pub fn genres(&self) -> Result<Vec<String>, String> {
        let value = self.cached_call("getGenres", &[])?;
        let values = value
            .pointer("/genres/genre")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(values
            .iter()
            .filter_map(|v| {
                v.get("value")
                    .or_else(|| v.get("$"))?
                    .as_str()
                    .map(str::to_owned)
            })
            .collect())
    }
    pub fn genre_songs(&self, genre: &str, count: usize) -> Result<Vec<Song>, String> {
        if genre.trim().is_empty() {
            return Err("Выберите жанр".into());
        }
        Self::songs_at(
            &self.call(
                "getRandomSongs",
                &[("size", &count.clamp(1, 500).to_string()), ("genre", genre)],
            )?,
            "/randomSongs/song",
        )
    }
    pub fn search(&self, query: &str) -> Result<Vec<Song>, String> {
        Self::songs_at(
            &self.call(
                "search3",
                &[
                    ("query", query),
                    ("songCount", "100"),
                    ("albumCount", "0"),
                    ("artistCount", "0"),
                ],
            )?,
            "/searchResult3/song",
        )
    }
    pub fn similar(&self, id: &str, count: usize) -> Result<Vec<Song>, String> {
        Self::songs_at(
            &self.call(
                "getSimilarSongs2",
                &[("id", id), ("count", &count.clamp(1, 100).to_string())],
            )?,
            "/similarSongs2/song",
        )
    }
    pub fn favorite_songs(&self) -> Result<Vec<Song>, String> {
        Self::songs_at(&self.cached_call("getStarred2", &[])?, "/starred2/song")
    }
    pub fn rate_song(&self, song_id: &str, rating: u8) -> Result<(), String> {
        if rating > 5 {
            return Err("Рейтинг должен быть от 0 до 5".into());
        }
        self.call(
            "setRating",
            &[("id", song_id), ("rating", &rating.to_string())],
        )?;
        self.invalidate_metadata();
        Ok(())
    }
    /// Build a finite personalized queue from favorite tracks and their neighbors.
    pub fn favorite_wave(&self, count: usize) -> Result<Vec<Song>, String> {
        let mut favorites = self.favorite_songs()?;
        if favorites.is_empty() {
            return Err("Добавьте треки в избранное Navidrome, чтобы создать волну".into());
        }
        favorites.shuffle(&mut rand::thread_rng());
        favorites.truncate(5);
        let mut groups = Vec::new();
        let mut last_error = None;
        for seed in favorites {
            match self.similar(&seed.id, count.clamp(1, 100)) {
                Ok(neighbors) if !neighbors.is_empty() => {
                    let mut group = vec![seed];
                    group.extend(neighbors);
                    groups.push(group);
                }
                Ok(_) => (),
                Err(error) => last_error = Some(error),
            }
        }
        if groups.is_empty() {
            return Err(
                last_error.unwrap_or_else(|| "Сервер не нашёл похожие треки для избранного".into())
            );
        }
        Ok(interleave_unique(groups, count.clamp(1, 100)))
    }
    /// Fetch the queue explicitly saved on the server; this is never overwritten automatically.
    pub fn saved_queue(&self) -> Result<Option<SavedQueue>, String> {
        let body = self.call("getPlayQueue", &[])?;
        body.get("playQueue")
            .filter(|value| !value.is_null())
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| format!("Ошибка чтения очереди: {e}"))
    }
    pub fn save_queue(
        &self,
        songs: &[Song],
        current: Option<&str>,
        position_ms: u64,
    ) -> Result<(), String> {
        if songs.is_empty() {
            return Err("Очередь пуста".into());
        }
        if current.is_some_and(|id| !songs.iter().any(|song| song.id == id)) {
            return Err("Текущий трек отсутствует в очереди".into());
        }
        let mut params: Vec<(&str, &str)> =
            songs.iter().map(|song| ("id", song.id.as_str())).collect();
        let position = position_ms.to_string();
        if let Some(id) = current {
            params.push(("current", id));
            params.push(("position", &position));
        }
        self.call("savePlayQueue", &params).map(|_| ())
    }
    pub fn playlists(&self) -> Result<Vec<Playlist>, String> {
        let value = self.cached_call("getPlaylists", &[])?;
        serde_json::from_value(
            value
                .pointer("/playlists/playlist")
                .filter(|value| !value.is_null())
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|e| e.to_string())
    }
    pub fn playlist(&self, id: &str) -> Result<Vec<Song>, String> {
        Self::songs_at(
            &self.cached_call("getPlaylist", &[("id", id)])?,
            "/playlist/entry",
        )
    }
    pub fn create_playlist(&self, name: &str, songs: &[Song]) -> Result<(), String> {
        if name.trim().is_empty() {
            return Err("Введите название плейлиста".into());
        }
        if songs.is_empty() {
            return Err("Список треков пуст".into());
        }
        let mut params = vec![("name", name.trim())];
        params.extend(songs.iter().map(|s| ("songId", s.id.as_str())));
        self.call("createPlaylist", &params)?;
        self.invalidate_metadata();
        Ok(())
    }
    pub fn set_favorite(&self, song_id: &str, favorite: bool) -> Result<(), String> {
        self.call(if favorite { "star" } else { "unstar" }, &[("id", song_id)])?;
        self.invalidate_metadata();
        Ok(())
    }
    pub fn add_to_playlist(&self, playlist_id: &str, song_id: &str) -> Result<(), String> {
        self.call(
            "updatePlaylist",
            &[("playlistId", playlist_id), ("songIdToAdd", song_id)],
        )?;
        self.invalidate_metadata();
        Ok(())
    }
    pub fn remove_from_playlist(&self, playlist_id: &str, index: usize) -> Result<(), String> {
        self.call(
            "updatePlaylist",
            &[
                ("playlistId", playlist_id),
                ("songIndexToRemove", &index.to_string()),
            ],
        )?;
        self.invalidate_metadata();
        Ok(())
    }
    pub fn media_url(&self, method: &str, id: &str) -> Result<Url, String> {
        if !matches!(method, "stream" | "getCoverArt") {
            return Err("Неизвестный тип медиа".into());
        }
        let mut url = self.endpoint(method)?;
        url.query_pairs_mut()
            .extend_pairs(self.auth())
            .append_pair("id", id);
        Ok(url)
    }
    pub fn cover(&self, id: &str) -> Result<Vec<u8>, String> {
        if let Some(bytes) = self
            .cache
            .as_ref()
            .and_then(|cache| cache.read("covers", id, None))
        {
            return Ok(bytes);
        }
        let response = self
            .http
            .get(self.media_url("getCoverArt", id)?)
            .send()
            .map_err(|e| e.without_url().to_string())?
            .error_for_status()
            .map_err(|e| e.without_url().to_string())?;
        let mut bytes = Vec::new();
        response
            .take(20 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Не удалось загрузить обложку: {e}"))?;
        if bytes.len() > 20 * 1024 * 1024 {
            return Err("Обложка слишком велика".into());
        }
        let valid = bytes.starts_with(&[0xff, 0xd8, 0xff])
            || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
            || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"));
        if !valid {
            return Err("Сервер вернул неподдерживаемую обложку".into());
        }
        if let Some(cache) = &self.cache {
            let _ = cache.write("covers", id, bytes.as_slice(), 20 * 1024 * 1024);
        }
        Ok(bytes)
    }
}

/// Avoid recent repeats; when a source is exhausted, allow older tracks again.
pub fn wave_candidates(queue: &[Song], candidates: &[Song]) -> Vec<Song> {
    fn select(queue: &[Song], candidates: &[Song], history: usize) -> Vec<Song> {
        let recent: std::collections::HashSet<_> =
            queue.iter().rev().take(history).map(|s| &s.id).collect();
        let mut seen = std::collections::HashSet::new();
        candidates
            .iter()
            .filter(|song| !recent.contains(&song.id) && seen.insert(song.id.clone()))
            .cloned()
            .collect()
    }
    let result = select(queue, candidates, 200);
    if result.is_empty() {
        select(queue, candidates, 5)
    } else {
        result
    }
}

fn interleave_unique(groups: Vec<Vec<Song>>, count: usize) -> Vec<Song> {
    let mut seen = std::collections::HashSet::new();
    let mut iterators: Vec<_> = groups.into_iter().map(Vec::into_iter).collect();
    let mut result = Vec::new();
    loop {
        let mut progress = false;
        for iterator in &mut iterators {
            for song in iterator.by_ref() {
                if seen.insert(song.id.clone()) {
                    result.push(song);
                    progress = true;
                    break;
                }
            }
            if result.len() >= count {
                return result;
            }
        }
        if !progress {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wave_skips_recent_tracks_and_can_reuse_older_tracks() {
        let song = |id: &str| Song {
            id: id.into(),
            title: id.into(),
            ..Default::default()
        };
        let history: Vec<_> = (0..8).map(|i| song(&i.to_string())).collect();
        let result = wave_candidates(&history, &[song("1"), song("1"), song("7")]);
        assert_eq!(
            result.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["1"]
        );
        assert!(wave_candidates(&history, &[song("7")]).is_empty());
    }
    #[test]
    fn wave_interleaves_and_deduplicates() {
        let song = |id: &str| Song {
            id: id.into(),
            title: id.into(),
            ..Default::default()
        };
        let result = interleave_unique(
            vec![
                vec![song("a"), song("b"), song("c")],
                vec![song("b"), song("d"), song("a"), song("e")],
            ],
            4,
        );
        assert_eq!(
            result.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c", "d"]
        );
    }
    #[test]
    fn validates_server_address() {
        assert!(Client::new("ftp://example.org", "u", "p").is_err());
        assert!(Client::new("https://u:p@example.org", "u", "p").is_err());
        assert!(Client::new("https://example.org?token=x", "u", "p").is_err());
    }
    #[test]
    fn subpath_and_query_are_encoded() {
        let c = Client::new("https://example.org/music", "a b", "secret").unwrap();
        let url = c.media_url("stream", "one&two").unwrap();
        assert_eq!(url.path(), "/music/rest/stream.view");
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(pairs["u"], "a b");
        assert_eq!(pairs["id"], "one&two");
        assert!(!url.as_str().contains("secret"));
    }
    #[test]
    fn response_parsing_handles_empty_and_populated_songs() {
        let json = serde_json::json!({"searchResult3": {"song": [{"id":"1", "title":"Track"}]}});
        assert_eq!(
            Client::songs_at(&json, "/searchResult3/song").unwrap()[0].title,
            "Track"
        );
        assert!(Client::songs_at(&json, "/randomSongs/song")
            .unwrap()
            .is_empty());
    }
}

pub mod audiomuse;
