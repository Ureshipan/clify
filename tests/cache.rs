use clify::Client;
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

#[test]
fn track_is_downloaded_once_and_then_served_from_disk() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0; 4096];
        let size = socket.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..size]).starts_with("GET /rest/stream.view?"));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nmy-audio").unwrap();
    });
    let root = std::env::temp_dir().join(format!("clify-download-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let path = client.cached_audio("track").unwrap();
    server.join().unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"my-audio");
    assert_eq!(client.cached_audio("track").unwrap(), path);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn truncated_response_is_not_cached() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 99\r\nConnection: close\r\n\r\nshort").unwrap();
    });
    let root = std::env::temp_dir().join(format!("clify-truncated-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert!(client.cached_audio("track").is_err());
    server.join().unwrap();
    assert!(client.cached_audio("track").is_err());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn playlist_metadata_is_reused_and_invalidated_by_edits() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for index in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]);
            if index == 1 {
                assert!(request.starts_with("POST /rest/updatePlaylist.view"));
            }
            let body = if index == 1 {
                r#"{"subsonic-response":{"status":"ok"}}"#
            } else if index == 0 {
                r#"{"subsonic-response":{"status":"ok","playlist":{"entry":[{"id":"1","title":"Before"}]}}}"#
            } else {
                r#"{"subsonic-response":{"status":"ok","playlist":{"entry":[{"id":"2","title":"After"}]}}}"#
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let root = std::env::temp_dir().join(format!("clify-playlist-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert_eq!(client.playlist("p").unwrap()[0].title, "Before");
    assert_eq!(client.playlist("p").unwrap()[0].title, "Before");
    client.add_to_playlist("p", "2").unwrap();
    assert_eq!(client.playlist("p").unwrap()[0].title, "After");
    server.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn genre_wave_uses_navidrome_genre_names() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for index in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            let request = String::from_utf8_lossy(&buffer[..n]);
            let body = if index == 0 {
                assert!(request.starts_with("POST /rest/getGenres.view"));
                r#"{"subsonic-response":{"status":"ok","genres":{"genre":[{"value":"Jazz","songCount":3}]}}}"#
            } else {
                assert!(request.starts_with("POST /rest/getRandomSongs.view"));
                r#"{"subsonic-response":{"status":"ok","randomSongs":{"song":[{"id":"1","title":"Blue"}]}}}"#
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        }
    });
    let root = std::env::temp_dir().join(format!("clify-genre-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert_eq!(client.genres().unwrap(), vec!["Jazz"]);
    assert_eq!(client.genre_songs("Jazz", 20).unwrap()[0].title, "Blue");
    server.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn concurrent_requests_for_same_track_share_one_download() {
    use std::sync::{Arc, Barrier};
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        thread::sleep(Duration::from_millis(200));
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nmy-audio").unwrap();
        listener.set_nonblocking(true).unwrap();
        thread::sleep(Duration::from_millis(250));
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "unexpected second download"
        );
    });
    let root =
        std::env::temp_dir().join(format!("clify-concurrent-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let other = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for c in [client, other] {
        let gate = barrier.clone();
        workers.push(thread::spawn(move || {
            gate.wait();
            c.acquire_audio("same").unwrap()
        }));
    }
    barrier.wait();
    let first = workers.remove(0).join().unwrap();
    let second = workers.remove(0).join().unwrap();
    assert_eq!(first.path(), second.path());
    assert_eq!(std::fs::read(first.path()).unwrap(), b"my-audio");
    server.join().unwrap();
    drop(first);
    drop(second);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cancelled_download_discards_partial_file() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (ready, wait) = mpsc::channel();
    let (proceed, resume) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart").unwrap();
        ready.send(()).unwrap();
        resume.recv().unwrap();
        let _ = socket.write_all(b"rest");
    });
    let root = std::env::temp_dir().join(format!("clify-cancel-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let cancelled = Arc::new(AtomicBool::new(false));
    let c = client.clone();
    let flag = cancelled.clone();
    let worker = thread::spawn(move || {
        c.acquire_audio_with_cancel("song", &|| flag.load(Ordering::Relaxed))
            .map(|_| ())
    });
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    cancelled.store(true, Ordering::Relaxed);
    proceed.send(()).unwrap();
    assert!(worker.join().unwrap().is_err());
    server.join().unwrap();
    assert!(!client.has_cached_audio("song"));
    assert_eq!(client.cache_stats().unwrap().audio_files, 0);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn prefetch_download_is_reused_when_track_becomes_current() {
    use clify::prefetch::Prefetcher;
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (ready, wait) = mpsc::channel();
    let (proceed, resume) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let n = socket.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..n]).contains("id=next"));
        ready.send(()).unwrap();
        resume.recv().unwrap();
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nmy-audio").unwrap();
        listener.set_nonblocking(true).unwrap();
        thread::sleep(Duration::from_millis(250));
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "download was repeated"
        );
    });
    let root = std::env::temp_dir().join(format!("clify-prefetch-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let prefetch = Prefetcher::new();
    prefetch.schedule(client.clone(), "current", &["next".into()]);
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    prefetch.schedule(client.clone(), "next", &[]);
    let foreground = {
        let c = client.clone();
        thread::spawn(move || c.acquire_audio("next").unwrap())
    };
    proceed.send(()).unwrap();
    let audio = foreground.join().unwrap();
    assert_eq!(std::fs::read(audio.path()).unwrap(), b"my-audio");
    server.join().unwrap();
    drop(audio);
    drop(prefetch);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn old_playlist_response_cannot_repopulate_cache_after_edit() {
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let mut buf = [0; 4096];
        let _ = first.read(&mut buf).unwrap();
        ready_tx.send(()).unwrap();
        let (mut update, _) = listener.accept().unwrap();
        let n = update.read(&mut buf).unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("POST /rest/updatePlaylist.view"));
        let ok = r#"{"subsonic-response":{"status":"ok"}}"#;
        write!(
            update,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            ok.len(),
            ok
        )
        .unwrap();
        release_rx.recv().unwrap();
        let old = r#"{"subsonic-response":{"status":"ok","playlist":{"entry":[{"id":"1","title":"Old"}]}}}"#;
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            old.len(),
            old
        )
        .unwrap();
        let (mut fresh, _) = listener.accept().unwrap();
        let _ = fresh.read(&mut buf).unwrap();
        let new = r#"{"subsonic-response":{"status":"ok","playlist":{"entry":[{"id":"2","title":"New"}]}}}"#;
        write!(
            fresh,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            new.len(),
            new
        )
        .unwrap();
    });
    let root = std::env::temp_dir().join(format!("clify-metadata-race-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let before = {
        let c = client.clone();
        thread::spawn(move || c.playlist("p").unwrap())
    };
    ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    client.add_to_playlist("p", "2").unwrap();
    release_tx.send(()).unwrap();
    assert_eq!(before.join().unwrap()[0].title, "New");
    assert_eq!(client.playlist("p").unwrap()[0].title, "New");
    server.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn retries_transient_server_failure_and_keeps_complete_file() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for attempt in 0..2 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer);
            if attempt == 0 {
                socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nmy-audio").unwrap();
            }
        }
    });
    let root = std::env::temp_dir().join(format!("clify-retry-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let audio = client.acquire_audio("song").unwrap();
    assert_eq!(std::fs::read(audio.path()).unwrap(), b"my-audio");
    server.join().unwrap();
    drop(audio);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn authentication_failure_is_not_retried() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket
            .write_all(
                b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        listener.set_nonblocking(true).unwrap();
        thread::sleep(Duration::from_millis(500));
        assert!(
            matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "authentication was retried"
        );
    });
    let root = std::env::temp_dir().join(format!("clify-no-retry-test-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert!(client.acquire_audio("song").is_err());
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn changing_queue_cancels_obsolete_prefetch_and_downloads_new_track() {
    use clify::prefetch::Prefetcher;
    use std::sync::mpsc;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (ready, wait) = mpsc::channel();
    let (proceed, resume) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut old, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let n = old.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..n]).contains("id=old"));
        old.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart").unwrap();
        ready.send(()).unwrap();
        resume.recv().unwrap();
        let _ = old.write_all(b"rest");
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (mut new, _) = loop {
            match listener.accept() {
                Ok(pair) => break pair,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(e) => panic!("new prefetch did not arrive: {e}"),
            }
        };
        new.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let n = new.read(&mut buffer).unwrap();
        assert!(String::from_utf8_lossy(&buffer[..n]).contains("id=new"));
        new.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnew-data").unwrap();
    });
    let root =
        std::env::temp_dir().join(format!("clify-prefetch-change-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let prefetch = Prefetcher::new();
    prefetch.schedule(client.clone(), "current", &["old".into()]);
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    prefetch.schedule(client.clone(), "current", &["new".into()]);
    proceed.send(()).unwrap();
    server.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.has_cached_audio("new") && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(client.has_cached_audio("new"));
    assert!(!client.has_cached_audio("old"));
    drop(prefetch);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn upcoming_tracks_stay_pinned_until_queue_changes_even_over_limit() {
    use clify::prefetch::Prefetcher;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for id in ["current", "next", "after"] {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let n = socket.read(&mut buffer).unwrap();
            assert!(String::from_utf8_lossy(&buffer[..n]).contains(&format!("id={id}")));
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata").unwrap();
        }
    });
    let root = std::env::temp_dir().join(format!("clify-prefetch-pins-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    client.set_audio_cache_limit(8).unwrap();
    let current = client.acquire_audio("current").unwrap();
    let prefetch = Prefetcher::new();
    prefetch.schedule(client.clone(), "current", &["next".into(), "after".into()]);
    server.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !(client.has_cached_audio("next") && client.has_cached_audio("after"))
        && std::time::Instant::now() < deadline
    {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(client.has_cached_audio("next") && client.has_cached_audio("after"));
    assert_eq!(client.cache_stats().unwrap().audio_bytes, 12);
    prefetch.clear();
    assert!(current.path().exists());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while client.cache_stats().unwrap().audio_bytes > 8 && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(client.cache_stats().unwrap().audio_bytes <= 8);
    drop(current);
    drop(prefetch);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn changing_account_cancels_same_id_from_old_server() {
    use clify::prefetch::Prefetcher;
    use std::sync::mpsc;
    let old_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let old_addr = old_listener.local_addr().unwrap();
    let new_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let new_addr = new_listener.local_addr().unwrap();
    let (ready, wait) = mpsc::channel();
    let (proceed, resume) = mpsc::channel();
    let old_server = thread::spawn(move || {
        let (mut socket, _) = old_listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\npart").unwrap();
        ready.send(()).unwrap();
        resume.recv().unwrap();
        let _ = socket.write_all(b"rest");
    });
    let new_server = thread::spawn(move || {
        new_listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let (mut socket, _) = loop {
            match new_listener.accept() {
                Ok(pair) => break pair,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(e) => panic!("new account not prefetched: {e}"),
            }
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nContent-Length: 8\r\nConnection: close\r\n\r\nnew-data").unwrap();
    });
    let root = std::env::temp_dir().join(format!("clify-account-change-{}", rand::random::<u64>()));
    let old = Client::new(&format!("http://{old_addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let new = Client::new(&format!("http://{new_addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    let prefetch = Prefetcher::new();
    prefetch.schedule(old.clone(), "current", &["same".into()]);
    wait.recv_timeout(Duration::from_secs(5)).unwrap();
    prefetch.schedule(new.clone(), "current", &["same".into()]);
    proceed.send(()).unwrap();
    old_server.join().unwrap();
    new_server.join().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !new.has_cached_audio("same") && std::time::Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(new.has_cached_audio("same"));
    assert!(!old.has_cached_audio("same"));
    drop(prefetch);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn partial_http_response_is_not_cached_as_full_track() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer);
        socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mpeg\r\nContent-Range: bytes 0-3/8\r\nContent-Length: 4\r\nConnection: close\r\n\r\npart").unwrap();
    });
    let root =
        std::env::temp_dir().join(format!("clify-partial-response-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert!(client.acquire_audio("song").is_err());
    server.join().unwrap();
    assert!(!client.has_cached_audio("song"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_metadata_does_not_hide_authorization_errors_but_survives_server_outage() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        for index in 0..3 {
            let (mut socket, _) = listener.accept().unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).unwrap();
            if index == 0 {
                let body = r#"{"subsonic-response":{"status":"ok","playlist":{"entry":[{"id":"1","title":"Cached"}]}}}"#;
                write!(
                    socket,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            } else if index == 1 {
                socket.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else {
                socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            }
        }
    });
    let root = std::env::temp_dir().join(format!("clify-stale-auth-{}", rand::random::<u64>()));
    let client = Client::new(&format!("http://{addr}"), "user", "password")
        .unwrap()
        .with_cache_root(&root);
    assert_eq!(client.playlist("p").unwrap()[0].title, "Cached");
    let account_dir = std::fs::read_dir(&root)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let metadata = std::fs::read_dir(account_dir.join("metadata"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::File::open(metadata)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::UNIX_EPOCH))
        .unwrap();
    assert!(client.playlist("p").unwrap_err().contains("401"));
    assert_eq!(client.playlist("p").unwrap()[0].title, "Cached");
    server.join().unwrap();
    std::fs::remove_dir_all(root).unwrap();
}
