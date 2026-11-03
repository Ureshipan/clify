//! One background worker keeps a small part of the playback queue cached.
use crate::{CachedAudio, Client};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{mpsc, Arc, Mutex},
    thread,
};

struct Policy {
    allowed: HashSet<String>,
    pinned: HashMap<String, CachedAudio>,
    scope: Option<PathBuf>,
    revision: u64,
    shutdown: bool,
}
enum Command {
    Update(Box<Client>, Vec<String>, u64),
    Wake,
}
/// Cancellation is cooperative: an in-progress blocking network read ends before it is observed.
pub struct Prefetcher {
    tx: mpsc::Sender<Command>,
    policy: Arc<Mutex<Policy>>,
}
impl Default for Prefetcher {
    fn default() -> Self {
        Self::new()
    }
}
impl Prefetcher {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let policy = Arc::new(Mutex::new(Policy {
            allowed: HashSet::new(),
            pinned: HashMap::new(),
            scope: None,
            revision: 0,
            shutdown: false,
        }));
        let shared = policy.clone();
        thread::spawn(move || {
            while let Ok(command) = rx.recv() {
                let Command::Update(client, ids, revision) = command else {
                    continue;
                };
                let scope = client.cache_scope();
                for id in ids {
                    let is_current = || {
                        let state = shared.lock().unwrap_or_else(|e| e.into_inner());
                        !state.shutdown && state.scope == scope && state.allowed.contains(&id)
                    };
                    {
                        let state = shared.lock().unwrap_or_else(|e| e.into_inner());
                        if state.shutdown {
                            return;
                        }
                        if state.revision != revision {
                            break;
                        }
                    }
                    if let Ok(audio) = client.acquire_audio_with_cancel(&id, &|| !is_current()) {
                        let mut state = shared.lock().unwrap_or_else(|e| e.into_inner());
                        if !state.shutdown && state.scope == scope && state.allowed.contains(&id) {
                            state.pinned.insert(id, audio);
                        }
                    }
                }
            }
        });
        Self { tx, policy }
    }
    /// Retain the current track so an in-flight prefetch can finish when playback reaches it.
    pub fn schedule(&self, client: Client, current: &str, upcoming: &[String]) {
        let mut seen = HashSet::new();
        let ids: Vec<_> = upcoming
            .iter()
            .filter(|id| id.as_str() != current && seen.insert((*id).clone()))
            .take(2)
            .cloned()
            .collect();
        let mut policy = self.policy.lock().unwrap_or_else(|e| e.into_inner());
        policy.revision = policy.revision.wrapping_add(1);
        let scope = client.cache_scope();
        if policy.scope != scope {
            policy.pinned.clear();
        }
        policy.scope = scope;
        policy.allowed = ids.iter().cloned().collect();
        policy.allowed.insert(current.to_owned());
        let allowed = policy.allowed.clone();
        policy.pinned.retain(|id, _| allowed.contains(id));
        let revision = policy.revision;
        drop(policy);
        let _ = self
            .tx
            .send(Command::Update(Box::new(client), ids, revision));
    }
    pub fn clear(&self) {
        let mut state = self.policy.lock().unwrap_or_else(|e| e.into_inner());
        state.revision = state.revision.wrapping_add(1);
        state.allowed.clear();
        state.scope = None;
        state.pinned.clear();
        drop(state);
        let _ = self.tx.send(Command::Wake);
    }
}
impl Drop for Prefetcher {
    fn drop(&mut self) {
        let mut state = self.policy.lock().unwrap_or_else(|e| e.into_inner());
        state.shutdown = true;
        state.allowed.clear();
        state.scope = None;
        state.pinned.clear();
        drop(state);
        let _ = self.tx.send(Command::Wake);
    }
}
