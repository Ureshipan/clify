//! MPRIS bridge; D-Bus callbacks are forwarded to the GUI thread.
use clify::queue::RepeatMode;
use mpris_server::{LoopStatus, Metadata, PlaybackStatus, Player, Time, TrackId};
use std::sync::mpsc::Sender;

#[derive(Clone, Debug)]
pub enum Control {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek(f64),
    SetPosition(String, f64),
    Volume(f64),
    Repeat(RepeatMode),
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub art_url: Option<String>,
    pub duration: f64,
    pub position: f64,
    pub volume: f64,
    pub repeat: RepeatMode,
    pub playing: bool,
    pub paused: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_previous: bool,
    pub can_next: bool,
}
fn send(tx: &Sender<super::Message>, ctx: &eframe::egui::Context, command: Control) {
    let _ = tx.send(super::Message::Mpris(command));
    ctx.request_repaint();
}
pub fn start(
    tx: Sender<super::Message>,
    ctx: eframe::egui::Context,
) -> tokio::sync::mpsc::UnboundedSender<State> {
    let (state_tx, mut state_rx) = tokio::sync::mpsc::unbounded_channel::<State>();
    std::thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return;
        };
        runtime.block_on(async move {
            let Ok(player) = Player::builder("clify").identity("CLIFY").can_control(true).build().await else { return; };
            macro_rules! on { ($method:ident, $action:expr) => {{ let tx = tx.clone(); let ctx = ctx.clone(); player.$method(move |_| send(&tx, &ctx, $action)); }}; }
            on!(connect_play, Control::Play); on!(connect_pause, Control::Pause); on!(connect_play_pause, Control::PlayPause);
            on!(connect_stop, Control::Stop); on!(connect_next, Control::Next); on!(connect_previous, Control::Previous);
            { let tx = tx.clone(); let ctx = ctx.clone(); player.connect_seek(move |_, t| send(&tx, &ctx, Control::Seek(t.as_micros() as f64 / 1_000_000.0))); }
            { let tx = tx.clone(); let ctx = ctx.clone(); player.connect_set_position(move |_, id, t| send(&tx, &ctx, Control::SetPosition(id.to_string(), t.as_micros() as f64 / 1_000_000.0))); }
            { let tx = tx.clone(); let ctx = ctx.clone(); player.connect_set_volume(move |_, v| send(&tx, &ctx, Control::Volume(v))); }
            { let tx = tx.clone(); let ctx = ctx.clone(); player.connect_set_loop_status(move |_, v| send(&tx, &ctx, Control::Repeat(match v { LoopStatus::None => RepeatMode::Off, LoopStatus::Track => RepeatMode::One, LoopStatus::Playlist => RepeatMode::All }))); }
            let run = player.run(); tokio::pin!(run); let mut previous: Option<State> = None;
            loop {
                tokio::select! {
                    _ = &mut run => break,
                    state = state_rx.recv() => {
                        let Some(state) = state else { break; };
                        let old = previous.as_ref();
                        if old.is_none_or(|o| o.id != state.id || o.title != state.title || o.artist != state.artist || o.album != state.album || o.art_url != state.art_url || o.duration != state.duration) {
                            let mut metadata = Metadata::builder().title(&state.title).artist([state.artist.clone()]).album(&state.album).length(Time::from_micros((state.duration.max(0.0) * 1_000_000.0) as i64));
                            if let Ok(id) = TrackId::try_from(state.id.clone()) { metadata = metadata.trackid(id); }
                            if let Some(url) = &state.art_url { metadata = metadata.art_url(url.clone()); }
                            let _ = player.set_metadata(metadata.build()).await;
                        }
                        if old.is_none_or(|o| o.playing != state.playing || o.paused != state.paused) {
                            let status = if !state.playing { PlaybackStatus::Stopped } else if state.paused { PlaybackStatus::Paused } else { PlaybackStatus::Playing };
                            let _ = player.set_playback_status(status).await;
                        }
                        if old.is_none_or(|o| o.repeat != state.repeat) { let _ = player.set_loop_status(match state.repeat { RepeatMode::Off => LoopStatus::None, RepeatMode::One => LoopStatus::Track, RepeatMode::All => LoopStatus::Playlist }).await; }
                        if old.is_none_or(|o| o.volume != state.volume) { let _ = player.set_volume(state.volume).await; }
                        if old.is_none_or(|o| o.can_play != state.can_play) { let _ = player.set_can_play(state.can_play).await; }
                        if old.is_none_or(|o| o.can_pause != state.can_pause) { let _ = player.set_can_pause(state.can_pause).await; let _ = player.set_can_seek(state.can_pause).await; }
                        if old.is_none_or(|o| o.can_next != state.can_next) { let _ = player.set_can_go_next(state.can_next).await; }
                        if old.is_none_or(|o| o.can_previous != state.can_previous) { let _ = player.set_can_go_previous(state.can_previous).await; }
                        player.set_position(Time::from_micros((state.position.max(0.0) * 1_000_000.0) as i64));
                        previous = Some(state);
                    }
                }
            }
        });
    });
    state_tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{process::Command, thread, time::Duration};
    #[test]
    fn exports_metadata_and_forwards_next_on_session_bus() {
        if std::env::var_os("CLIFY_TEST_DBUS").is_none()
            || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none()
            || Command::new("gdbus").arg("--version").output().is_err()
        {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let state = start(tx, eframe::egui::Context::default());
        state
            .send(State {
                id: "/org/clify/track/t7".into(),
                title: "Test title".into(),
                artist: "Test artist".into(),
                album: "Album".into(),
                art_url: Some("file:///tmp/cover.jpg".into()),
                duration: 123.0,
                playing: true,
                can_play: true,
                can_next: true,
                ..Default::default()
            })
            .unwrap();
        let base = [
            "call",
            "--session",
            "--dest",
            "org.mpris.MediaPlayer2.clify",
            "--object-path",
            "/org/mpris/MediaPlayer2",
        ];
        let mut result = String::new();
        for _ in 0..50 {
            if let Ok(output) = Command::new("gdbus")
                .args(base)
                .args([
                    "--method",
                    "org.freedesktop.DBus.Properties.Get",
                    "org.mpris.MediaPlayer2.Player",
                    "Metadata",
                ])
                .output()
            {
                result = String::from_utf8_lossy(&output.stdout).into_owned();
                if output.status.success() && result.contains("Test title") {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        assert!(result.contains("Test title"), "metadata: {result}");
        assert!(result.contains("Test artist"));
        assert!(result.contains("file:///tmp/cover.jpg"));
        let result = Command::new("gdbus")
            .args(base)
            .args(["--method", "org.mpris.MediaPlayer2.Player.Next"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(3)).unwrap(),
            super::super::Message::Mpris(Control::Next)
        ));
    }
}
