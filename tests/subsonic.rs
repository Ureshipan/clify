use clify::{Client, Song};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

fn server(response: &'static str) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let n = socket.read(&mut buffer).unwrap();
            if n == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        tx.send(String::from_utf8(bytes).unwrap()).unwrap();
        write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
    });
    (format!("http://{addr}/music"), rx)
}

#[test]
fn creates_playlist_with_repeated_song_parameters_and_no_plaintext_password() {
    let (url, rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "secret-password").unwrap();
    let songs = vec![
        Song {
            id: "a&1".into(),
            title: "A".into(),
            ..Default::default()
        },
        Song {
            id: "b".into(),
            title: "B".into(),
            ..Default::default()
        },
    ];
    client.create_playlist("My mix", &songs).unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /music/rest/createPlaylist.view "));
    assert!(!request.contains("secret-password"));
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let params: Vec<_> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert!(params.contains(&("name".into(), "My mix".into())));
    assert_eq!(
        params
            .iter()
            .filter(|(key, _)| key == "songId")
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>(),
        vec!["a&1", "b"]
    );
}

#[test]
fn surfaces_subsonic_error_even_when_http_succeeds() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"failed","error":{"code":40,"message":"Wrong username or password"}}}"#,
    );
    let client = Client::new(&url, "listener", "wrong").unwrap();
    assert_eq!(client.ping().unwrap_err(), "Wrong username or password");
}

#[test]
fn removes_playlist_entry_by_index() {
    let (url, rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    client.remove_from_playlist("playlist & 1", 3).unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /music/rest/updatePlaylist.view "));
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(params["playlistId"], "playlist & 1");
    assert_eq!(params["songIndexToRemove"], "3");
}

#[test]
fn sends_now_playing_without_incrementing_play_count() {
    let (url, rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    client.scrobble("song-1", false).unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /music/rest/scrobble.view "));
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(params["submission"], "false");
}

#[test]
fn saves_queue_with_repeated_ids_and_millisecond_position() {
    let (url, rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    let songs = vec![
        Song {
            id: "first".into(),
            title: "A".into(),
            ..Default::default()
        },
        Song {
            id: "next".into(),
            title: "B".into(),
            ..Default::default()
        },
    ];
    client.save_queue(&songs, Some("next"), 12345).unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /music/rest/savePlayQueue.view "));
    let params: Vec<_> =
        url::form_urlencoded::parse(request.split("\r\n\r\n").nth(1).unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(
        params
            .iter()
            .filter(|(k, _)| k == "id")
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "next"]
    );
    assert!(params.contains(&("current".into(), "next".into())));
    assert!(params.contains(&("position".into(), "12345".into())));
}

#[test]
fn restores_saved_queue_with_position() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","playQueue":{"current":"2","position":3456,"entry":[{"id":"1","title":"A"},{"id":"2","title":"B"}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    let queue = client.saved_queue().unwrap().unwrap();
    assert_eq!(queue.current.as_deref(), Some("2"));
    assert_eq!(queue.position, 3456);
    assert_eq!(queue.songs.len(), 2);
}

#[test]
fn reads_structured_lyrics_with_offsets() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[{"lang":"eng","synced":true,"offset":200,"line":[{"start":1000,"value":"first"},{"start":2000,"value":"second"}]}]}}}"#,
    );
    let root = std::env::temp_dir().join(format!("clify-lyrics-test-{}", rand::random::<u64>()));
    let client = Client::new(&url, "listener", "password")
        .unwrap()
        .with_cache_root(&root);
    let lyrics = client.lyrics("song").unwrap();
    assert_eq!(lyrics[0].active_line(800), Some(0));
    assert_eq!(lyrics[0].seek_position(1), Some(1.8));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn browses_indexed_artists() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","artists":{"index":[{"name":"A","artist":[{"id":"1","name":"Artist","albumCount":2}]},{"name":"B","artist":[{"id":"2","name":"Band"}]}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    let artists = client.artists().unwrap();
    assert_eq!(artists.len(), 2);
    assert_eq!(artists[0].album_count, 2);
    assert_eq!(artists[1].name, "Band");
}
#[test]
fn browses_album_tracks() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","album":{"id":"a","name":"Album","song":[{"id":"s","title":"Song"}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    assert_eq!(client.album_songs("a").unwrap()[0].title, "Song");
}

#[test]
fn saves_rating_and_rejects_out_of_range_values() {
    let (url, rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    assert!(client.rate_song("song", 6).is_err());
    client.rate_song("song", 4).unwrap();
    let request = rx.recv().unwrap();
    assert!(request.starts_with("POST /music/rest/setRating.view "));
    let params: std::collections::HashMap<_, _> =
        url::form_urlencoded::parse(request.split("\r\n\r\n").nth(1).unwrap().as_bytes())
            .into_owned()
            .collect();
    assert_eq!(params["id"], "song");
    assert_eq!(params["rating"], "4");
}

#[test]
fn accepts_null_optional_metadata() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","album":{"song":[{"id":"s","title":"Track","artist":null,"album":null,"duration":null}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    let songs = client.album_songs("a").unwrap();
    assert_eq!(songs[0].artist, "");
    assert_eq!(songs[0].duration, 0);
}

#[test]
fn null_empty_track_collection_is_treated_as_empty() {
    let (url, _rx) = server(r#"{"subsonic-response":{"status":"ok","album":{"song":null}}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    assert!(client.album_songs("a").unwrap().is_empty());
}

#[test]
fn album_pagination_encodes_offset_and_bounded_page_size() {
    let (url, rx) = server(
        r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"1","name":"Album","artist":"Artist","songCount":7}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    let albums = client.albums(50, 1000).unwrap();
    assert_eq!(albums[0].name, "Album");
    assert_eq!(albums[0].song_count, 7);
    let request = rx.recv().unwrap();
    let body = request.split("\r\n\r\n").nth(1).unwrap();
    let params: std::collections::HashMap<_, _> = url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect();
    assert_eq!(params["offset"], "50");
    assert_eq!(params["size"], "500");
    assert_eq!(params["type"], "alphabeticalByName");
}

#[test]
fn no_saved_queue_is_distinct_from_error() {
    let (url, _rx) = server(r#"{"subsonic-response":{"status":"ok"}}"#);
    let client = Client::new(&url, "listener", "password").unwrap();
    assert!(client.saved_queue().unwrap().is_none());
}

#[test]
fn nullable_lyrics_fields_are_safe_to_display() {
    let (url, _rx) = server(
        r#"{"subsonic-response":{"status":"ok","lyricsList":{"structuredLyrics":[{"lang":null,"synced":false,"offset":null,"line":null}]}}}"#,
    );
    let client = Client::new(&url, "listener", "password").unwrap();
    let lyrics = client.lyrics("song").unwrap();
    assert_eq!(lyrics.len(), 1);
    assert!(lyrics[0].lines.is_empty());
    assert!(lyrics[0].active_line(0).is_none());
}
