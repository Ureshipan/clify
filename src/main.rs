#[cfg(target_os = "linux")]
mod mpris;
mod player;
mod ui;
use clify::{
    audiomuse::{AudioMuse, Authentication, Mood, SimilarOptions},
    cache::CacheStats,
    lyrics::Lyrics,
    prefetch::Prefetcher,
    queue::{self, RepeatMode},
    sleep_timer::SleepTimer,
    Album, Artist, CachedAudio, Client, Playlist, SavedQueue, Song,
};
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, RichText, TextureHandle};
use std::collections::BTreeMap;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    time::{Duration, Instant},
};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 760.0])
            .with_min_inner_size([720.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("CLIFY", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Library,
    Mix,
    Playlists,
    Settings,
    Queue,
    Lyrics,
}
#[derive(Clone, Copy, PartialEq)]
enum LibraryView {
    Songs,
    Albums,
    Artists,
}
#[derive(Clone, Copy, PartialEq)]
enum MixMode {
    Navidrome,
    Wave,
    Track,
    Mood,
    Artist,
    Genre,
}
#[derive(Clone)]
enum WaveSource {
    Genre(String),
    Navidrome(String),
    Track(String, SimilarOptions),
    Mood(String, usize, SimilarOptions),
}
#[derive(Clone, Copy, PartialEq)]
enum AuthMode {
    Login,
    Token,
    None,
}

enum Message {
    Connected(Result<Client, String>),
    Songs(Result<Vec<Song>, String>),
    Albums(usize, Option<String>, Result<Vec<Album>, String>),
    Artists(Result<Vec<Artist>, String>),
    Playlists(Result<Vec<Playlist>, String>),
    Created(Result<(), String>),
    FavoriteUpdated(Result<(), String>, bool),
    Rated(String, u8, Result<(), String>),
    PlaylistUpdated(Result<Vec<Song>, String>),
    PlaylistLoaded(String, Result<Vec<Song>, String>),
    QueueSaved(Result<(), String>),
    QueueLoaded(Result<Option<SavedQueue>, String>),
    Cover(u64, String, Result<Vec<u8>, String>),
    AudioReady(u64, usize, Result<CachedAudio, String>),
    Genres(Result<Vec<String>, String>),
    Lyrics(u64, String, Result<Vec<Lyrics>, String>),
    CacheInfo(Result<CacheStats, String>),
    CacheChanged(Result<(), String>),
    #[cfg(target_os = "linux")]
    Mpris(mpris::Control),
    WaveRefill(u64, Result<Vec<Song>, String>),
    SongDetails(u64, String, Result<Song, String>),
    MuseConnected(Result<(AudioMuse, BTreeMap<String, Vec<Mood>>), String>),
    PlayerResult(u64, PlayerAction, Result<serde_json::Value, String>),
    Position(u64, Result<(f64, f64), String>),
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Preferences {
    palette: usize,
    volume: u8,
    #[serde(default = "default_cache_gib")]
    cache_gib: u8,
    #[serde(default = "default_prefetch")]
    prefetch: bool,
    #[serde(default)]
    repeat: RepeatMode,
    #[serde(default)]
    replay_gain: player::ReplayGain,
}
fn default_cache_gib() -> u8 {
    2
}
fn default_prefetch() -> bool {
    true
}

#[derive(Clone, Copy)]
enum PlayerAction {
    Pause(bool),
    Volume,
    Seek,
}

struct App {
    #[cfg(target_os = "linux")]
    mpris: tokio::sync::mpsc::UnboundedSender<mpris::State>,
    #[cfg(target_os = "linux")]
    last_mpris: Option<mpris::State>,
    tx: Sender<Message>,
    rx: Receiver<Message>,
    client: Option<Client>,
    session_revision: u64,
    server: String,
    username: String,
    password: String,
    page: Page,
    busy: bool,
    status: String,
    query: String,
    library_view: LibraryView,
    albums: Vec<Album>,
    artists: Vec<Artist>,
    album_offset: usize,
    album_artist: Option<String>,
    artist_filter: String,
    songs: Vec<Song>,
    playlists: Vec<Playlist>,
    selected: Option<Song>,
    cover: Option<TextureHandle>,
    lyrics_song: Option<String>,
    lyrics: Vec<Lyrics>,
    lyrics_index: usize,
    playlist_name: String,
    open_playlist: Option<String>,
    mix_count: usize,
    palette: usize,
    player: Option<player::Player>,
    playing_audio: Option<CachedAudio>,
    foreground_cancel: Option<Arc<AtomicBool>>,
    prefetcher: Prefetcher,
    prefetch_enabled: bool,
    cache_gib: u8,
    cache_info: Option<CacheStats>,
    queue: Vec<Song>,
    repeat: RepeatMode,
    replay_gain: player::ReplayGain,
    queue_index: Option<usize>,
    resume_position: Option<(usize, f64)>,
    player_generation: u64,
    paused: bool,
    volume: u8,
    position: f64,
    seek_preview: Option<f64>,
    duration: f64,
    position_in_flight: bool,
    loading_audio: bool,
    last_position_request: Instant,
    last_listened_position: Option<f64>,
    listened: f64,
    scrobbled: bool,
    sleep_timer: SleepTimer,
    wave: Option<WaveSource>,
    wave_revision: u64,
    wave_loading: bool,
    genres: Vec<String>,
    genre: String,
    muse: Option<AudioMuse>,
    muse_url: String,
    muse_server: String,
    muse_user: String,
    muse_password: String,
    muse_token: String,
    auth_mode: AuthMode,
    moods: BTreeMap<String, Vec<Mood>>,
    mood_index: usize,
    mood_name: String,
    mix_mode: MixMode,
    artist: String,
    mix_radius: bool,
    mix_artist_limit: bool,
    mix_mood_filter: bool,
    temperature: f32,
}
impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = FontDefinitions::default();
        fonts.font_data.insert(
            "cyrillic".into(),
            FontData::from_static(include_bytes!("../assets/DejaVuSans.ttf")).into(),
        );
        fonts
            .families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "cyrillic".into());
        fonts
            .families
            .get_mut(&FontFamily::Monospace)
            .unwrap()
            .insert(0, "cyrillic".into());
        cc.egui_ctx.set_fonts(fonts);
        let (tx, rx) = mpsc::channel();
        #[cfg(target_os = "linux")]
        let mpris = mpris::start(tx.clone(), cc.egui_ctx.clone());
        let preferences = cc
            .storage
            .and_then(|s| s.get_string("clify.preferences"))
            .and_then(|s| serde_json::from_str::<Preferences>(&s).ok());
        let palette = preferences.as_ref().map(|p| p.palette.min(3)).unwrap_or(0);
        let volume = preferences
            .as_ref()
            .map(|p| p.volume.min(100))
            .unwrap_or(80);
        let cache_gib = preferences
            .as_ref()
            .map(|p| p.cache_gib.clamp(1, 20))
            .unwrap_or(2);
        let prefetch_enabled = preferences.as_ref().map(|p| p.prefetch).unwrap_or(true);
        let repeat = preferences.as_ref().map(|p| p.repeat).unwrap_or_default();
        let replay_gain = preferences
            .as_ref()
            .map(|p| p.replay_gain)
            .unwrap_or_default();
        Self {
            #[cfg(target_os = "linux")]
            mpris,
            #[cfg(target_os = "linux")]
            last_mpris: None,
            tx,
            rx,
            client: None,
            session_revision: 0,
            server: "http://localhost:4533".into(),
            username: String::new(),
            password: String::new(),
            page: Page::Library,
            busy: false,
            status: "Подключитесь к Navidrome, чтобы начать".into(),
            query: String::new(),
            library_view: LibraryView::Songs,
            albums: vec![],
            artists: vec![],
            album_offset: 0,
            album_artist: None,
            artist_filter: String::new(),
            songs: vec![],
            playlists: vec![],
            selected: None,
            cover: None,
            lyrics_song: None,
            lyrics: vec![],
            lyrics_index: 0,
            playlist_name: String::new(),
            open_playlist: None,
            mix_count: 30,
            palette,
            player: None,
            playing_audio: None,
            foreground_cancel: None,
            prefetcher: Prefetcher::new(),
            prefetch_enabled,
            cache_gib,
            cache_info: None,
            queue: vec![],
            repeat,
            replay_gain,
            queue_index: None,
            resume_position: None,
            player_generation: 0,
            paused: false,
            volume,
            position: 0.0,
            seek_preview: None,
            duration: 0.0,
            position_in_flight: false,
            loading_audio: false,
            last_position_request: Instant::now(),
            last_listened_position: None,
            listened: 0.0,
            scrobbled: false,
            sleep_timer: SleepTimer::Off,
            wave: None,
            wave_revision: 0,
            wave_loading: false,
            genres: vec![],
            genre: String::new(),
            muse: None,
            muse_url: "http://localhost:8000".into(),
            muse_server: String::new(),
            muse_user: String::new(),
            muse_password: String::new(),
            muse_token: String::new(),
            auth_mode: AuthMode::Login,
            moods: BTreeMap::new(),
            mood_index: 0,
            mood_name: String::new(),
            mix_mode: MixMode::Navidrome,
            artist: String::new(),
            mix_radius: true,
            mix_artist_limit: true,
            mix_mood_filter: false,
            temperature: 1.0,
        }
    }
    fn accent(&self) -> Color32 {
        [
            Color32::from_rgb(152, 237, 181),
            Color32::from_rgb(121, 193, 255),
            Color32::from_rgb(225, 162, 255),
            Color32::from_rgb(255, 183, 119),
        ][self.palette]
    }
    fn spawn(&mut self, ctx: &egui::Context, work: impl FnOnce() -> Message + Send + 'static) {
        self.busy = true;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(work());
            ctx.request_repaint();
        });
    }
    fn connect(&mut self, ctx: &egui::Context) {
        let (server, user, pass) = (
            self.server.clone(),
            self.username.clone(),
            self.password.clone(),
        );
        let cache_limit = self.cache_gib as u64 * 1024 * 1024 * 1024;
        self.spawn(ctx, move || {
            Message::Connected(Client::new(&server, &user, &pass).and_then(|c| {
                c.ping()?;
                c.set_audio_cache_limit(cache_limit)?;
                Ok(c)
            }))
        });
    }
    fn load_songs(&mut self, ctx: &egui::Context, query: Option<String>) {
        if let Some(c) = self.client.clone() {
            self.open_playlist = None;
            self.spawn(ctx, move || {
                Message::Songs(match query {
                    Some(q) => c.search(&q),
                    None => c.random_songs(),
                })
            });
        }
    }
    fn select(&mut self, song: Song, ctx: &egui::Context) {
        self.selected = Some(song.clone());
        self.cover = None;
        if let Some(id) = song.cover_art {
            self.load_cover(id, ctx);
        } else if let Some(c) = self.client.clone() {
            let id = song.id;
            let revision = self.session_revision;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = c.song(&id);
                let _ = tx.send(Message::SongDetails(revision, id, result));
                ctx.request_repaint();
            });
        }
    }
    fn load_cover(&self, id: String, ctx: &egui::Context) {
        if let Some(c) = self.client.clone() {
            let revision = self.session_revision;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let result = c.cover(&id);
                let _ = tx.send(Message::Cover(revision, id, result));
                ctx.request_repaint();
            });
        }
    }
    fn play(&mut self, ctx: &egui::Context) {
        self.end_wave();
        if let Some((index, _)) = self.resume_position {
            if self
                .queue
                .get(index)
                .zip(self.selected.as_ref())
                .is_some_and(|(queued, selected)| queued.id == selected.id)
            {
                self.play_index(index, ctx);
                return;
            }
            self.resume_position = None;
        }
        if let Some(song) = &self.selected {
            if let Some(index) = self.songs.iter().position(|s| s.id == song.id) {
                self.queue = self.songs.clone();
                self.play_index(index, ctx);
                return;
            }
            self.queue = vec![song.clone()];
            self.play_index(0, ctx);
        }
    }
    fn end_wave(&mut self) {
        self.wave = None;
        self.wave_loading = false;
        self.wave_revision = self.wave_revision.wrapping_add(1);
    }
    fn start_wave(&mut self, source: WaveSource, ctx: &egui::Context) {
        self.end_wave();
        self.prefetcher.clear();
        self.stop();
        self.queue.clear();
        self.queue_index = None;
        self.resume_position = None;
        self.wave = Some(source);
        self.status = "Подбираем треки для волны…".into();
        self.refill_wave(ctx);
    }
    fn refill_wave(&mut self, ctx: &egui::Context) {
        if self.wave_loading {
            return;
        }
        let (Some(source), Some(client)) = (self.wave.clone(), self.client.clone()) else {
            return;
        };
        let muse = self.muse.clone();
        let last_id = self.queue.last().map(|s| s.id.clone());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        let revision = self.wave_revision;
        self.wave_loading = true;
        std::thread::spawn(move || {
            let result = match source {
                WaveSource::Genre(genre) => client.genre_songs(&genre, 100),
                WaveSource::Navidrome(id) => client.similar(last_id.as_deref().unwrap_or(&id), 100),
                WaveSource::Track(id, options) => muse
                    .ok_or("AudioMuse-AI отключён".to_string())
                    .and_then(|m| m.similar(last_id.as_deref().unwrap_or(&id), None, options)),
                WaveSource::Mood(mood, index, options) => muse
                    .ok_or("AudioMuse-AI отключён".to_string())
                    .and_then(|m| m.similar("", Some((&mood, index)), options)),
            };
            let _ = tx.send(Message::WaveRefill(revision, result));
            ctx.request_repaint();
        });
    }
    fn schedule_prefetch(&self) {
        if !self.prefetch_enabled {
            self.prefetcher.clear();
            return;
        }
        if let (Some(client), Some(index)) = (self.client.clone(), self.queue_index) {
            if let Some(current) = self.queue.get(index) {
                let mut upcoming: Vec<_> = self
                    .queue
                    .iter()
                    .skip(index + 1)
                    .take(20)
                    .map(|s| s.id.clone())
                    .collect();
                if self.wave.is_none() && self.repeat == RepeatMode::All && upcoming.len() < 2 {
                    upcoming.extend(self.queue.iter().take(index).map(|s| s.id.clone()));
                }
                self.prefetcher.schedule(client, &current.id, &upcoming);
            }
        }
    }
    fn play_index(&mut self, index: usize, ctx: &egui::Context) {
        let (Some(c), Some(song)) = (self.client.clone(), self.queue.get(index).cloned()) else {
            return;
        };
        self.stop();
        if self
            .resume_position
            .is_some_and(|(saved, _)| saved != index)
        {
            self.resume_position = None;
        }
        self.queue_index = Some(index);
        self.loading_audio = true;
        self.status = format!("Подготовка трека: {}", song.title);
        self.select(song.clone(), ctx);
        let cancelled = Arc::new(AtomicBool::new(false));
        self.foreground_cancel = Some(cancelled.clone());
        self.schedule_prefetch();
        let generation = self.player_generation;
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result =
                c.acquire_audio_with_cancel(&song.id, &|| cancelled.load(Ordering::Relaxed));
            let _ = tx.send(Message::AudioReady(generation, index, result));
            ctx.request_repaint();
        });
    }
    fn start_cached(&mut self, index: usize, audio: CachedAudio) {
        let (Some(c), Some(song)) = (self.client.clone(), self.queue.get(index).cloned()) else {
            return;
        };
        match player::Player::start(audio.path(), self.volume, self.replay_gain) {
            Ok(player) => {
                let mpv = player.is_mpv;
                self.player = Some(player);
                self.playing_audio = Some(audio);
                self.queue_index = Some(index);
                self.paused = false;
                self.last_listened_position = None;
                self.listened = 0.0;
                self.scrobbled = false;
                let song_id = song.id.clone();
                std::thread::spawn(move || {
                    let _ = c.scrobble(&song_id, false);
                });
                self.position = 0.0;
                self.duration = song.duration as f64;
                self.last_position_request = Instant::now() - Duration::from_secs(2);
                self.status = if mpv {
                    format!("Воспроизведение: {}", song.title)
                } else if self
                    .resume_position
                    .take()
                    .is_some_and(|(_, position)| position > 0.0)
                {
                    "ffplay не умеет восстанавливать позицию; трек запущен с начала".into()
                } else {
                    format!(
                        "Воспроизведение через ffplay: {} (для паузы и перемотки установите mpv)",
                        song.title
                    )
                };
            }
            Err(e) => self.status = e,
        }
    }
    fn stop(&mut self) {
        self.player_generation = self.player_generation.wrapping_add(1);
        if let Some(cancelled) = self.foreground_cancel.take() {
            cancelled.store(true, Ordering::Relaxed);
        }
        if let Some(mut p) = self.player.take() {
            p.stop();
            self.status = "Воспроизведение остановлено".into();
        }
        self.playing_audio = None;
        self.paused = false;
        self.loading_audio = false;
        self.position_in_flight = false;
        self.position = 0.0;
        self.seek_preview = None;
    }
    fn player_command(
        &self,
        ctx: &egui::Context,
        action: PlayerAction,
        command: serde_json::Value,
    ) {
        if let Some(ipc) = self.player.as_ref().and_then(|p| p.ipc()) {
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let generation = self.player_generation;
            std::thread::spawn(move || {
                let result = player::command(&ipc, command);
                let _ = tx.send(Message::PlayerResult(generation, action, result));
                ctx.request_repaint();
            });
        }
    }
    fn request_position(&mut self, ctx: &egui::Context) {
        if self.position_in_flight || self.last_position_request.elapsed() < Duration::from_secs(1)
        {
            return;
        }
        if let Some(ipc) = self.player.as_ref().and_then(|p| p.ipc()) {
            self.position_in_flight = true;
            self.last_position_request = Instant::now();
            let generation = self.player_generation;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let position =
                    player::command(&ipc, serde_json::json!(["get_property", "time-pos"]));
                let result = position.and_then(|p| {
                    let position = p.as_f64().ok_or("mpv ещё загружает трек")?;
                    let duration =
                        player::command(&ipc, serde_json::json!(["get_property", "duration"]))
                            .ok()
                            .and_then(|d| d.as_f64())
                            .unwrap_or(0.0);
                    Ok((position, duration))
                });
                let _ = tx.send(Message::Position(generation, result));
                ctx.request_repaint();
            });
        }
    }
    fn step(&mut self, offset: isize, ctx: &egui::Context) {
        let Some(current) = self.queue_index else {
            return;
        };
        let next = current
            .checked_add_signed(offset)
            .filter(|i| *i < self.queue.len())
            .or_else(|| {
                if self.wave.is_none() && self.repeat == RepeatMode::All && !self.queue.is_empty() {
                    if offset == 1 {
                        Some(0)
                    } else if offset == -1 {
                        Some(self.queue.len() - 1)
                    } else {
                        None
                    }
                } else {
                    None
                }
            });
        if let Some(index) = next {
            self.play_index(index, ctx);
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(msg) = self.rx.try_recv() {
            match msg {
                Message::PlayerResult(generation, action, result) => {
                    if generation == self.player_generation {
                        match result {
                            Ok(_) => {
                                if let PlayerAction::Pause(value) = action {
                                    self.paused = value;
                                }
                            }
                            Err(e) => self.status = e,
                        }
                    }
                }
                Message::Position(generation, result) => {
                    if generation == self.player_generation {
                        self.position_in_flight = false;
                        if let Ok((position, duration)) = result {
                            if let Some((index, target)) = self.resume_position.take() {
                                if self.queue_index == Some(index) && target > 0.0 {
                                    self.last_listened_position = None;
                                    self.player_command(
                                        ctx,
                                        PlayerAction::Seek,
                                        serde_json::json!(["seek", target, "absolute+exact"]),
                                    );
                                }
                            }
                            if !self.paused {
                                if let Some(previous) = self.last_listened_position {
                                    let delta = position - previous;
                                    if (0.0..=3.0).contains(&delta) {
                                        self.listened += delta;
                                    }
                                }
                            }
                            self.last_listened_position = Some(position);
                            self.position = position;
                            if duration > 0.0 {
                                self.duration = duration;
                            }
                        }
                    }
                }
                #[cfg(target_os = "linux")]
                Message::Mpris(command) => {
                    self.mpris_control(command, ctx);
                }
                Message::CacheInfo(result) => {
                    self.busy = false;
                    match result {
                        Ok(info) => {
                            self.cache_info = Some(info);
                            self.status = "Размер кэша обновлён".into();
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::CacheChanged(result) => {
                    self.busy = false;
                    self.cache_info = None;
                    self.status = match result {
                        Ok(()) => "Кэш обновлён".into(),
                        Err(e) => e,
                    };
                }
                Message::Lyrics(revision, id, result) => {
                    if revision != self.session_revision {
                        continue;
                    }
                    self.busy = false;
                    match result {
                        Ok(lyrics) if self.selected.as_ref().is_some_and(|song| song.id == id) => {
                            self.lyrics_song = Some(id);
                            self.lyrics_index = 0;
                            self.status = if lyrics.is_empty() {
                                "Для трека нет текста".into()
                            } else {
                                "Текст песни загружен".into()
                            };
                            self.lyrics = lyrics;
                        }
                        Ok(_) => self.status = "Выбран другой трек".into(),
                        Err(e) => self.status = e,
                    }
                }
                Message::Genres(result) => {
                    self.busy = false;
                    match result {
                        Ok(genres) => {
                            self.genre = genres.first().cloned().unwrap_or_default();
                            self.status = format!("Жанров: {}", genres.len());
                            self.genres = genres;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::WaveRefill(revision, result) => {
                    if revision == self.wave_revision && self.wave.is_some() {
                        self.wave_loading = false;
                        match result {
                            Ok(songs) => {
                                let added = clify::wave_candidates(&self.queue, &songs);
                                if added.is_empty() {
                                    self.end_wave();
                                    self.status = "Для волны больше нет доступных треков".into();
                                } else {
                                    let next = self.queue_index.map(|i| i + 1).unwrap_or(0);
                                    self.queue.extend(added);
                                    self.schedule_prefetch();
                                    if self.player.is_none()
                                        && !self.loading_audio
                                        && next < self.queue.len()
                                    {
                                        self.play_index(next, ctx);
                                    }
                                }
                            }
                            Err(e) => {
                                self.end_wave();
                                self.status = format!("Волна остановлена: {e}");
                            }
                        }
                    }
                }
                Message::AudioReady(generation, index, result) => {
                    if generation == self.player_generation {
                        self.loading_audio = false;
                        self.foreground_cancel = None;
                        match result {
                            Ok(audio) => self.start_cached(index, audio),
                            Err(e) => self.status = e,
                        }
                    }
                }
                Message::SongDetails(revision, id, result) => {
                    if revision != self.session_revision {
                        continue;
                    }
                    if let Ok(song) = result {
                        if self.selected.as_ref().is_some_and(|s| s.id == id) {
                            self.selected = Some(song.clone());
                            if let Some(cover) = song.cover_art.clone() {
                                self.load_cover(cover, ctx);
                            }
                        }
                        if let Some(index) = self
                            .queue_index
                            .filter(|i| self.queue.get(*i).is_some_and(|s| s.id == id))
                        {
                            if song.duration > 0 && self.duration == 0.0 {
                                self.duration = song.duration as f64;
                            }
                            self.queue[index] = song;
                        }
                    }
                }
                Message::Cover(revision, id, result) => {
                    if revision != self.session_revision {
                        continue;
                    }
                    if self.selected.as_ref().and_then(|s| s.cover_art.as_ref()) == Some(&id) {
                        if let Ok(bytes) = result {
                            if let Ok(img) = image::load_from_memory(&bytes) {
                                let image = img.to_rgba8();
                                let size = [image.width() as usize, image.height() as usize];
                                self.cover = Some(ctx.load_texture(
                                    "cover",
                                    egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()),
                                    Default::default(),
                                ));
                            }
                        }
                    }
                }
                Message::MuseConnected(res) => {
                    self.busy = false;
                    match res {
                        Ok((client, moods)) => {
                            self.muse = Some(client);
                            self.mood_name = moods
                                .iter()
                                .find(|(_, v)| !v.is_empty())
                                .map(|(k, _)| k.clone())
                                .unwrap_or_default();
                            self.mood_index = 0;
                            self.moods = moods;
                            self.muse_password.clear();
                            self.muse_token.clear();
                            self.status = "AudioMuse-AI подключён".into();
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::Connected(res) => {
                    self.busy = false;
                    match res {
                        Ok(c) => {
                            self.session_revision = self.session_revision.wrapping_add(1);
                            self.client = Some(c);
                            self.password.clear();
                            self.status = "Подключено к Navidrome".into();
                            self.load_songs(ctx, None);
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::Albums(offset, artist, result) => {
                    self.busy = false;
                    match result {
                        Ok(albums) => {
                            self.album_offset = offset;
                            self.album_artist = artist;
                            self.status = format!("Альбомов: {}", albums.len());
                            self.albums = albums;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::Artists(result) => {
                    self.busy = false;
                    match result {
                        Ok(artists) => {
                            self.status = format!("Исполнителей: {}", artists.len());
                            self.artists = artists;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::Songs(res) => {
                    self.busy = false;
                    match res {
                        Ok(s) => {
                            self.status = format!("Загружено треков: {}", s.len());
                            self.songs = s;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::Playlists(res) => {
                    self.busy = false;
                    match res {
                        Ok(p) => {
                            self.status = format!("Плейлистов: {}", p.len());
                            self.playlists = p;
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::QueueSaved(result) => {
                    self.busy = false;
                    self.status = match result {
                        Ok(()) => "Очередь сохранена на сервере".into(),
                        Err(e) => e,
                    };
                }
                Message::QueueLoaded(result) => {
                    self.busy = false;
                    match result {
                        Ok(Some(saved)) if !saved.songs.is_empty() => {
                            self.end_wave();
                            self.prefetcher.clear();
                            self.stop();
                            self.queue = saved.songs;
                            let index = saved
                                .current
                                .as_ref()
                                .and_then(|id| self.queue.iter().position(|song| &song.id == id))
                                .unwrap_or(0);
                            self.queue_index = Some(index);
                            self.resume_position = Some((index, saved.position as f64 / 1000.0));
                            self.select(self.queue[index].clone(), ctx);
                            self.status = "Очередь восстановлена; нажмите PLAY".into();
                        }
                        Ok(_) => self.status = "На сервере нет сохранённой очереди".into(),
                        Err(e) => self.status = e,
                    }
                }
                Message::PlaylistLoaded(id, res) => {
                    self.busy = false;
                    match res {
                        Ok(songs) => {
                            self.status = format!("Загружено треков: {}", songs.len());
                            self.songs = songs;
                            self.open_playlist = Some(id);
                        }
                        Err(e) => {
                            self.open_playlist = None;
                            self.status = e;
                        }
                    }
                }
                Message::PlaylistUpdated(res) => {
                    self.busy = false;
                    match res {
                        Ok(songs) => {
                            self.songs = songs;
                            self.status = "Плейлист обновлён".into();
                        }
                        Err(e) => {
                            self.open_playlist = None;
                            self.status = e;
                        }
                    }
                }
                Message::Rated(id, rating, result) => {
                    self.busy = false;
                    match result {
                        Ok(()) => {
                            if let Some(song) = self.selected.as_mut().filter(|song| song.id == id)
                            {
                                song.user_rating = Some(rating);
                            }
                            for song in self
                                .songs
                                .iter_mut()
                                .chain(self.queue.iter_mut())
                                .filter(|song| song.id == id)
                            {
                                song.user_rating = Some(rating);
                            }
                            self.status = if rating == 0 {
                                "Оценка удалена".into()
                            } else {
                                format!("Оценка: {rating} из 5")
                            };
                        }
                        Err(e) => self.status = e,
                    }
                }
                Message::FavoriteUpdated(res, favorite) => {
                    self.busy = false;
                    self.status = match res {
                        Ok(()) if favorite => "Трек добавлен в избранное".into(),
                        Ok(()) => "Трек удалён из избранного".into(),
                        Err(e) => e,
                    };
                }
                Message::Created(res) => {
                    self.busy = false;
                    self.status = match res {
                        Ok(()) => "Плейлист сохранён на сервере".into(),
                        Err(e) => e,
                    };
                }
            }
        }
        if self.sleep_timer.expired(Instant::now()) {
            self.sleep_timer = SleepTimer::Off;
            self.resume_position = None;
            self.end_wave();
            self.prefetcher.clear();
            self.stop();
            self.status = "Таймер сна остановил воспроизведение".into();
        }
        if !self.scrobbled
            && self.duration > 30.0
            && self.listened >= (self.duration / 2.0).min(240.0)
        {
            if let (Some(client), Some(song)) = (
                self.client.clone(),
                self.queue_index.and_then(|i| self.queue.get(i)),
            ) {
                let id = song.id.clone();
                self.scrobbled = true;
                std::thread::spawn(move || {
                    let _ = client.scrobble(&id, true);
                });
            }
        }
        let outcome = self
            .player
            .as_mut()
            .and_then(|p| p.try_wait().ok().flatten());
        if let Some(status) = outcome {
            self.player = None;
            self.playing_audio = None;
            self.player_generation = self.player_generation.wrapping_add(1);
            self.position_in_flight = false;
            if status.success() && self.sleep_timer == SleepTimer::EndOfTrack {
                self.sleep_timer = SleepTimer::Off;
                self.end_wave();
                self.prefetcher.clear();
                self.status = "Таймер сна остановил воспроизведение".into();
            } else if status.success() {
                let mode = if self.wave.is_some() {
                    RepeatMode::Off
                } else {
                    self.repeat
                };
                if let Some(next) = self
                    .queue_index
                    .and_then(|i| queue::next_after_completion(i, self.queue.len(), mode))
                {
                    self.play_index(next, ctx);
                } else if self.wave.is_some() {
                    self.status = "Загружаем продолжение волны…".into();
                    self.refill_wave(ctx);
                } else {
                    self.prefetcher.clear();
                    self.status = "Очередь воспроизведена".into();
                }
            } else {
                self.status = "Проигрыватель завершился с ошибкой".into();
            }
        }
        if self.wave.is_some() && !self.loading_audio {
            let removed = queue::trim_history(&mut self.queue, &mut self.queue_index, 200);
            if removed > 0 {
                self.resume_position = self.resume_position.and_then(|(index, position)| {
                    index.checked_sub(removed).map(|index| (index, position))
                });
            }
        }
        if self.wave.is_some()
            && self
                .queue_index
                .is_some_and(|i| self.queue.len().saturating_sub(i + 1) < 5)
        {
            self.refill_wave(ctx);
        }
        self.request_position(ctx);
        #[cfg(target_os = "linux")]
        self.publish_mpris();
    }
    #[cfg(target_os = "linux")]
    fn track_path(&self) -> String {
        format!("/org/clify/track/t{}", self.player_generation)
    }
    #[cfg(target_os = "linux")]
    fn mpris_control(&mut self, control: mpris::Control, ctx: &egui::Context) {
        use mpris::Control;
        let mpv = self.player.as_ref().is_some_and(|p| p.is_mpv);
        match control {
            Control::Play if mpv && self.paused => self.player_command(
                ctx,
                PlayerAction::Pause(false),
                serde_json::json!(["set_property", "pause", false]),
            ),
            Control::Play if self.player.is_none() && !self.loading_audio => {
                if let Some(i) = self.queue_index {
                    self.play_index(i, ctx);
                } else {
                    self.play(ctx);
                }
            }
            Control::Pause if mpv && !self.paused => self.player_command(
                ctx,
                PlayerAction::Pause(true),
                serde_json::json!(["set_property", "pause", true]),
            ),
            Control::PlayPause if mpv => {
                let value = !self.paused;
                self.player_command(
                    ctx,
                    PlayerAction::Pause(value),
                    serde_json::json!(["set_property", "pause", value]),
                );
            }
            Control::PlayPause if self.player.is_none() && !self.loading_audio => {
                if let Some(i) = self.queue_index {
                    self.play_index(i, ctx);
                } else {
                    self.play(ctx);
                }
            }
            Control::Stop => {
                self.resume_position = None;
                self.end_wave();
                self.prefetcher.clear();
                self.stop();
            }
            Control::Next => self.step(1, ctx),
            Control::Previous => self.step(-1, ctx),
            Control::Seek(offset) if mpv => {
                self.last_listened_position = None;
                self.player_command(
                    ctx,
                    PlayerAction::Seek,
                    serde_json::json!(["seek", offset, "relative+exact"]),
                );
            }
            Control::SetPosition(id, position) if mpv && id == self.track_path() => {
                self.last_listened_position = None;
                self.player_command(
                    ctx,
                    PlayerAction::Seek,
                    serde_json::json!(["seek", position.max(0.0), "absolute+exact"]),
                );
            }
            Control::Repeat(repeat) => {
                self.repeat = repeat;
                self.schedule_prefetch();
            }
            Control::Volume(volume) if mpv && volume.is_finite() => {
                self.volume = (volume.clamp(0.0, 1.0) * 100.0).round() as u8;
                self.player_command(
                    ctx,
                    PlayerAction::Volume,
                    serde_json::json!(["set_property", "volume", self.volume]),
                );
            }
            _ => (),
        }
    }
    #[cfg(target_os = "linux")]
    fn publish_mpris(&mut self) {
        let current = self
            .queue_index
            .and_then(|i| self.queue.get(i))
            .filter(|_| self.player.is_some());
        let cover = current
            .and_then(|song| song.cover_art.as_ref())
            .and_then(|id| self.client.as_ref()?.cached_cover_path(id))
            .and_then(|path| url::Url::from_file_path(path).ok())
            .map(|url| url.to_string());
        let mpv = self.player.as_ref().is_some_and(|p| p.is_mpv);
        let state = mpris::State {
            id: self.track_path(),
            title: current.map(|s| s.title.clone()).unwrap_or_default(),
            artist: current.map(|s| s.artist.clone()).unwrap_or_default(),
            album: current.map(|s| s.album.clone()).unwrap_or_default(),
            art_url: cover,
            duration: self.duration,
            position: self.position,
            volume: self.volume as f64 / 100.0,
            repeat: if self.wave.is_some() {
                RepeatMode::Off
            } else {
                self.repeat
            },
            playing: self.player.is_some(),
            paused: self.paused,
            can_play: self.client.is_some() && (!self.queue.is_empty() || self.selected.is_some()),
            can_pause: mpv,
            can_previous: self.queue_index.is_some_and(|i| {
                i > 0
                    || (self.wave.is_none()
                        && self.repeat == RepeatMode::All
                        && self.queue.len() > 1)
            }),
            can_next: self.queue_index.is_some_and(|i| {
                i + 1 < self.queue.len()
                    || (self.wave.is_none()
                        && self.repeat == RepeatMode::All
                        && self.queue.len() > 1)
            }),
        };
        if self.last_mpris.as_ref() != Some(&state) {
            let _ = self.mpris.send(state.clone());
            self.last_mpris = Some(state);
        }
    }
    fn heading(ui: &mut egui::Ui, small: &str, title: &str, subtitle: &str, accent: Color32) {
        ui.label(RichText::new(small).size(11.0).color(accent).monospace());
        ui.add_space(5.0);
        ui.heading(RichText::new(title).size(29.0).strong());
        ui.label(RichText::new(subtitle).color(Color32::from_gray(145)));
        ui.add_space(16.0);
    }
}
impl Drop for App {
    fn drop(&mut self) {
        self.stop();
    }
}
impl eframe::App for App {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        if let Ok(json) = serde_json::to_string(&Preferences {
            palette: self.palette,
            volume: self.volume,
            cache_gib: self.cache_gib,
            prefetch: self.prefetch_enabled,
            repeat: self.repeat,
            replay_gain: self.replay_gain,
        }) {
            storage.set_string("clify.preferences", json);
        }
    }
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.poll(ctx);
        ctx.request_repaint_after(Duration::from_millis(500));
        let accent = self.accent();
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(13, 17, 19);
        visuals.window_fill = Color32::from_rgb(20, 26, 28);
        visuals.override_text_color = Some(Color32::from_rgb(225, 235, 229));
        visuals.selection.bg_fill = accent.linear_multiply(0.32);
        visuals.extreme_bg_color = Color32::from_rgb(18, 24, 25);
        visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(23, 31, 30);
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(23, 31, 30);
        visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 60, 54));
        visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(36, 51, 44);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(36, 51, 44);
        visuals.widgets.hovered.bg_stroke =
            egui::Stroke::new(1.0_f32, accent.linear_multiply(0.65));
        visuals.widgets.active.weak_bg_fill = accent.linear_multiply(0.27);
        visuals.widgets.active.bg_fill = accent.linear_multiply(0.35);
        visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0_f32, accent);
        for widget in [
            &mut visuals.widgets.inactive,
            &mut visuals.widgets.hovered,
            &mut visuals.widgets.active,
        ] {
            widget.corner_radius = egui::CornerRadius::same(5);
        }
        ctx.set_visuals(visuals);
        ctx.style_mut(|style| {
            style.spacing.item_spacing = egui::vec2(8.0, 7.0);
            style.spacing.button_padding = egui::vec2(10.0, 6.0);
            style.spacing.interact_size.y = 30.0;
        });
        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(
                    RichText::new("◈  CLIFY")
                        .size(20.0)
                        .strong()
                        .color(accent)
                        .monospace(),
                );
                ui.label(
                    RichText::new("/  YOUR MUSIC, YOUR SIGNAL")
                        .size(10.0)
                        .color(Color32::GRAY)
                        .monospace(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add_space(15.0);
                    ui.label(
                        RichText::new(if self.client.is_some() {
                            "● ONLINE"
                        } else {
                            "○ OFFLINE"
                        })
                        .size(11.0)
                        .color(accent)
                        .monospace(),
                    );
                });
            });
            ui.add_space(8.0);
        });
        egui::TopBottomPanel::bottom("player").show(ctx, |ui| {
            ui.separator();
            ui.add_space(7.0);
            ui.horizontal_wrapped(|ui| {
                ui.add_space(12.0);
                if ui.button("⏮").clicked() {
                    self.step(-1, ctx);
                }
                let mpv = self.player.as_ref().is_some_and(|p| p.is_mpv);
                if mpv {
                    if ui.button(if self.paused { "▶" } else { "⏸" }).clicked() {
                        let pause = !self.paused;
                        self.player_command(
                            ctx,
                            PlayerAction::Pause(pause),
                            serde_json::json!(["set_property", "pause", pause]),
                        );
                    }
                } else if ui.button("▶ PLAY").clicked() {
                    self.play(ctx);
                }
                if ui.button("■").clicked() {
                    self.resume_position = None;
                    self.end_wave();
                    self.prefetcher.clear();
                    self.stop();
                }
                if ui.button("⏭").clicked() {
                    self.step(1, ctx);
                }
                if ui
                    .add_enabled(
                        !self.loading_audio && self.queue_index.is_some(),
                        egui::Button::new("🔀"),
                    )
                    .on_hover_text("Перемешать следующие треки")
                    .clicked()
                {
                    if let Some(index) = self.queue_index {
                        queue::shuffle_upcoming(&mut self.queue, index, &mut rand::thread_rng());
                        self.schedule_prefetch();
                    }
                }
                let repeat_label = if self.wave.is_some() {
                    "Волна · повтор выкл"
                } else {
                    self.repeat.label()
                };
                if ui
                    .button(repeat_label)
                    .on_hover_text("Режим повтора действует для обычной очереди")
                    .clicked()
                {
                    self.repeat = self.repeat.next();
                    self.schedule_prefetch();
                }
                ui.add_space(8.0);
                let text = self
                    .queue_index
                    .and_then(|i| self.queue.get(i))
                    .filter(|_| self.player.is_some())
                    .map(|s| format!("{} — {}", s.artist, s.title))
                    .unwrap_or_else(|| "Выберите трек".into());
                ui.label(text);
            });
            if self.player.as_ref().is_some_and(|p| p.is_mpv) {
                ui.horizontal(|ui| {
                    ui.add_space(12.0);
                    let mut position = self
                        .seek_preview
                        .unwrap_or(self.position)
                        .min(self.duration.max(0.0));
                    let seek = ui.add_enabled(
                        self.duration > 0.0,
                        egui::Slider::new(&mut position, 0.0..=self.duration.max(1.0))
                            .show_value(false)
                            .text(format!(
                                "{}:{:02} / {}:{:02}",
                                self.position as u64 / 60,
                                self.position as u64 % 60,
                                self.duration as u64 / 60,
                                self.duration as u64 % 60
                            )),
                    );
                    if seek.dragged() {
                        self.seek_preview = Some(position);
                    }
                    if seek.drag_stopped() || (seek.changed() && !seek.dragged()) {
                        self.seek_preview = None;
                        self.last_listened_position = None;
                        self.player_command(
                            ctx,
                            PlayerAction::Seek,
                            serde_json::json!(["seek", position, "absolute+exact"]),
                        );
                        self.position = position;
                    }
                    ui.label("Громкость");
                    let volume = ui.add(egui::Slider::new(&mut self.volume, 0..=100));
                    if volume.drag_stopped() || (volume.changed() && !volume.dragged()) {
                        self.player_command(
                            ctx,
                            PlayerAction::Volume,
                            serde_json::json!(["set_property", "volume", self.volume]),
                        );
                    }
                });
            }
            ui.horizontal(|ui| {
                ui.add_space(14.0);
                ui.label(RichText::new(&self.status).size(11.0).color(Color32::GRAY));
            });
            ui.add_space(7.0);
        });
        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(185.0)
            .show(ctx, |ui| {
                ui.add_space(28.0);
                ui.label(
                    RichText::new("  НАВИГАЦИЯ")
                        .size(10.0)
                        .color(Color32::GRAY)
                        .monospace(),
                );
                ui.add_space(12.0);
                for (page, label) in [
                    (Page::Library, "  ◫   Медиатека"),
                    (Page::Mix, "  ◉   Микс"),
                    (Page::Playlists, "  ≡   Плейлисты"),
                    (Page::Queue, "  ▷   Очередь"),
                    (Page::Lyrics, "  ♫   Текст песни"),
                    (Page::Settings, "  ⚙   Настройки"),
                ] {
                    if ui.selectable_label(self.page == page, label).clicked() {
                        self.page = page;
                    }
                    ui.add_space(8.0);
                }
                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.label(
                        RichText::new("  CLIFY  /  0.1.0")
                            .size(10.0)
                            .color(Color32::DARK_GRAY)
                            .monospace(),
                    );
                    ui.add_space(15.0);
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(22.0);
                ui.horizontal(|ui| {
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        if self.client.is_none() {
                            self.login_ui(ui, ctx, accent);
                        } else {
                            match self.page {
                                Page::Library => self.library_ui(ui, ctx, accent),
                                Page::Mix => self.mix_ui(ui, ctx, accent),
                                Page::Playlists => self.playlists_ui(ui, ctx, accent),
                                Page::Queue => self.queue_ui(ui, ctx, accent),
                                Page::Lyrics => self.lyrics_ui(ui, ctx, accent),
                                Page::Settings => self.settings_ui(ui, ctx, accent),
                            }
                        }
                    });
                });
            });
        });
    }
}
impl App {
    fn song_list(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(cover) = &self.cover {
            ui.add(egui::Image::new(cover).fit_to_exact_size(egui::vec2(100.0, 100.0)));
            ui.add_space(10.0);
        }
        if self.songs.is_empty() {
            ui.label(RichText::new("Здесь пока нет треков").color(Color32::GRAY));
        }
        for (index, song) in self.songs.clone().into_iter().enumerate() {
            let selected = self.selected.as_ref().map(|s| &s.id) == Some(&song.id);
            let text = format!(
                "{:02}   {}  —  {}     {}:{:02}",
                index + 1,
                song.artist,
                song.title,
                song.duration / 60,
                song.duration % 60
            );
            let response = ui.selectable_label(selected, text);
            if response.clicked() {
                self.select(song.clone(), ctx);
            }
            if response.double_clicked() {
                self.resume_position = None;
                self.end_wave();
                self.queue = self.songs.clone();
                self.play_index(index, ctx);
            }
        }
    }
}
