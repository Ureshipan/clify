use clify::audiomuse::{AudioMuse, Authentication, SimilarOptions};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
    time::Duration,
};

fn server(replies: Vec<&'static str>) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for response in replies {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 4096];
            loop {
                let n = socket.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
                if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&data[..end]);
                    let len: usize = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if data.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            tx.send(String::from_utf8(data).unwrap()).unwrap();
            socket.write_all(response.as_bytes()).unwrap();
        }
    });
    (format!("http://{addr}/muse"), rx)
}
fn response(body: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body)
}

#[test]
fn login_session_is_used_for_mood_mix() {
    let auth = "HTTP/1.1 200 OK\r\nSet-Cookie: audiomuse_jwt=session-value; HttpOnly; Path=/\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}";
    let moods: &'static str = Box::leak(
        response(r#"{"happy":[{"index":2,"top_tags":["bright"],"n_songs":9}]}"#).into_boxed_str(),
    );
    let tracks: &'static str =
        Box::leak(response(r#"[{"item_id":"song-1","title":"A","author":"B"}]"#).into_boxed_str());
    let (url, requests) = server(vec![auth, moods, tracks]);
    let (muse, options) = AudioMuse::connect(
        &url,
        Authentication::Login {
            username: "name".into(),
            password: "password".into(),
        },
        "main",
    )
    .unwrap();
    assert_eq!(options["happy"][0].index, 2);
    let songs = muse
        .similar(
            "",
            Some(("happy", 2)),
            SimilarOptions {
                count: 30,
                artist_limit: true,
                radius: false,
                mood_similarity: false,
            },
        )
        .unwrap();
    assert_eq!(songs[0].id, "song-1");
    assert_eq!(songs[0].artist, "B");
    let login = requests.recv().unwrap();
    assert!(login.starts_with("POST /muse/auth "));
    let mood_req = requests.recv().unwrap();
    assert!(mood_req
        .to_lowercase()
        .contains("cookie: audiomuse_jwt=session-value"));
    let request = requests.recv().unwrap();
    assert!(request.contains("mood=happy"));
    assert!(request.contains("centroid_index=2"));
    assert!(request.contains("server=main"));
    assert!(!request.contains("password"));
}

#[test]
fn artist_alchemy_uses_bearer_and_server_in_body() {
    let moods: &'static str = Box::leak(response("{}").into_boxed_str());
    let result: &'static str = Box::leak(
        response(r#"{"results":[{"item_id":"42","title":"Title","author":"Artist"}]}"#)
            .into_boxed_str(),
    );
    let (url, requests) = server(vec![moods, result]);
    let (muse, _) =
        AudioMuse::connect(&url, Authentication::Token("s3cr3t".into()), "primary").unwrap();
    let songs = muse.artist_mix("An Artist", 25, 1.5).unwrap();
    assert_eq!(songs[0].id, "42");
    let _ = requests.recv().unwrap();
    let request = requests.recv().unwrap();
    assert!(request.starts_with("POST /muse/api/alchemy "));
    assert!(request
        .to_lowercase()
        .contains("authorization: bearer s3cr3t"));
    let body: serde_json::Value =
        serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["items"][0]["type"], "artist");
    assert_eq!(body["items"][0]["id"], "An Artist");
    assert_eq!(body["server"], "primary");
}
