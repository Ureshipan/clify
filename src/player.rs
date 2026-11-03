//! Desktop playback through mpv's JSON IPC, with an ffplay fallback.
use rand::{distributions::Alphanumeric, Rng};
use serde_json::{json, Value};
use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReplayGain {
    #[default]
    Off,
    Track,
    Album,
}
impl ReplayGain {
    fn argument(self) -> &'static str {
        match self {
            Self::Off => "--replaygain=no",
            Self::Track => "--replaygain=track",
            Self::Album => "--replaygain=album",
        }
    }
}

pub struct Player {
    child: Child,
    ipc: Option<String>,
    temp: Option<PathBuf>,
    pub is_mpv: bool,
}
impl Player {
    pub fn start(
        source: &std::path::Path,
        volume: u8,
        replay_gain: ReplayGain,
    ) -> Result<Self, String> {
        let random: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(24)
            .map(char::from)
            .collect();
        #[cfg(unix)]
        let (ipc, temp) = {
            use std::os::unix::fs::DirBuilderExt;
            let directory = std::env::temp_dir().join(format!("clify-{random}"));
            std::fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .map_err(|e| format!("Не удалось создать каталог плеера: {e}"))?;
            (
                directory.join("mpv.sock").to_string_lossy().into_owned(),
                Some(directory),
            )
        };
        #[cfg(windows)]
        let (ipc, temp) = (format!(r"\\.\pipe\clify-{random}"), None);
        #[cfg(not(any(unix, windows)))]
        let (ipc, temp) = (String::new(), None);
        let result = Command::new("mpv")
            .args([
                "--no-config",
                "--no-terminal",
                "--audio-display=no",
                "--no-video",
                "--idle=no",
                &format!("--volume={volume}"),
                replay_gain.argument(),
                &format!("--input-ipc-server={ipc}"),
                "--",
            ])
            .arg(source)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match result {
            Ok(child) => Ok(Self {
                child,
                ipc: Some(ipc),
                temp,
                is_mpv: true,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if let Some(path) = &temp {
                    let _ = std::fs::remove_dir_all(path);
                }
                let child = Command::new("ffplay")
                    .args([
                        "-nodisp",
                        "-autoexit",
                        "-loglevel",
                        "quiet",
                        "-volume",
                        &volume.to_string(),
                    ])
                    .arg(source)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()
                    .map_err(|_| {
                        "Установите mpv или FFmpeg с ffplay и добавьте программу в PATH".to_string()
                    })?;
                Ok(Self {
                    child,
                    ipc: None,
                    temp: None,
                    is_mpv: false,
                })
            }
            Err(e) => {
                if let Some(path) = &temp {
                    let _ = std::fs::remove_dir_all(path);
                }
                Err(format!("Не удалось запустить mpv: {e}"))
            }
        }
    }
    pub fn ipc(&self) -> Option<String> {
        self.ipc.clone()
    }
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.stop();
        if let Some(path) = &self.temp {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

pub fn command(ipc: &str, command: Value) -> Result<Value, String> {
    #[cfg(unix)]
    let stream = {
        use std::os::unix::net::UnixStream;
        let mut result = None;
        for _ in 0..20 {
            match UnixStream::connect(ipc) {
                Ok(s) => {
                    result = Some(s);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
        let stream = result.ok_or("mpv не открыл канал управления")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|e| e.to_string())?;
        stream
    };
    #[cfg(windows)]
    let stream = {
        let mut result = None;
        for _ in 0..20 {
            match std::fs::OpenOptions::new().read(true).write(true).open(ipc) {
                Ok(s) => {
                    result = Some(s);
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(50)),
            }
        }
        result.ok_or("mpv не открыл канал управления")?
    };
    #[cfg(not(any(unix, windows)))]
    return Err("Управление mpv недоступно на этой платформе".into());
    #[cfg(any(unix, windows))]
    {
        let mut stream = stream;
        writeln!(stream, "{}", json!({"command":command, "request_id":1}))
            .map_err(|e| format!("Ошибка управления mpv: {e}"))?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut line = String::new();
            if reader
                .read_line(&mut line)
                .map_err(|e| format!("Ошибка ответа mpv: {e}"))?
                == 0
            {
                return Err("mpv закрыл канал управления".into());
            }
            let response: Value =
                serde_json::from_str(&line).map_err(|_| "Некорректный ответ mpv")?;
            if response.get("request_id").and_then(Value::as_u64) != Some(1) {
                continue;
            }
            if response.get("error").and_then(Value::as_str) != Some("success") {
                return Err(format!(
                    "Ошибка mpv: {}",
                    response
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("неизвестная ошибка")
                ));
            }
            return Ok(response.get("data").cloned().unwrap_or(Value::Null));
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{os::unix::net::UnixListener, sync::mpsc};
    #[test]
    fn reads_ipc_response_after_unsolicited_event() {
        let path = std::env::temp_dir().join(format!(
            "clify-test-{}-{}.sock",
            std::process::id(),
            rand::random::<u64>()
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            tx.send(line).unwrap();
            stream.write_all(b"{\"event\":\"start-file\"}\n{\"request_id\":1,\"error\":\"success\",\"data\":42.5}\n").unwrap();
        });
        let result = command(path.to_str().unwrap(), json!(["get_property", "time-pos"])).unwrap();
        assert_eq!(result.as_f64(), Some(42.5));
        let request: Value = serde_json::from_str(&rx.recv().unwrap()).unwrap();
        assert_eq!(request["command"], json!(["get_property", "time-pos"]));
        worker.join().unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
