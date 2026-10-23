#![cfg(unix)]
use browser_use::wire::{read_frame, write_frame};
use browser_use::{Client, Options};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    os::unix::net::{UnixListener, UnixStream},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Host {
    child: Child,
    dir: tempfile::TempDir,
}
impl Host {
    fn start() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("bu-")
            .tempdir_in("/tmp")
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_browser-use"))
            .args(["host", "--socket-dir"])
            .arg(dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut h = Self { child, dir };
        let deadline = Instant::now() + Duration::from_secs(5);
        while !h.dir.path().join("active.json").exists() {
            assert!(
                h.child.try_wait().unwrap().is_none(),
                "host exited before registering"
            );
            assert!(Instant::now() < deadline, "host startup timed out");
            thread::sleep(Duration::from_millis(10));
        }
        h
    }
    fn socket(&self) -> UnixStream {
        let registry: Value =
            serde_json::from_slice(&fs::read(self.dir.path().join("active.json")).unwrap())
                .unwrap();
        let stream = UnixStream::connect(registry["socketPath"].as_str().unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn host_routes_colliding_ids_out_of_order_and_broadcasts_notifications() {
    let mut h = Host::start();
    let mut a = h.socket();
    let mut b = h.socket();
    write_frame(&mut a, &json!({"jsonrpc":"2.0","id":1,"method":"alpha"})).unwrap();
    write_frame(&mut b, &json!({"jsonrpc":"2.0","id":1,"method":"beta"})).unwrap();
    let stdout = h.child.stdout.as_mut().unwrap();
    let first = read_frame(stdout).unwrap().unwrap();
    let second = read_frame(stdout).unwrap().unwrap();
    assert_ne!(first["id"], second["id"]);
    let stdin = h.child.stdin.as_mut().unwrap();
    write_frame(stdin, &json!({"jsonrpc":"2.0","method":"heartbeat"})).unwrap();
    for req in [second, first] {
        write_frame(
            stdin,
            &json!({"jsonrpc":"2.0","id":req["id"],"result":req["method"]}),
        )
        .unwrap();
    }
    for (stream, expected) in [(&mut a, "alpha"), (&mut b, "beta")] {
        assert_eq!(read_frame(stream).unwrap().unwrap()["method"], "heartbeat");
        assert_eq!(
            read_frame(stream).unwrap(),
            Some(json!({"jsonrpc":"2.0","id":1,"result":expected}))
        );
    }
    drop(h.child.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(3);
    while h.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(!h.dir.path().join("active.json").exists());
    assert!(
        !fs::read_dir(h.dir.path()).unwrap().any(|e| e
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "sock"))
    );
}

#[test]
fn sdk_preserves_scope_dispatches_notifications_and_returns_remote_error() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join("sdk.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        assert_eq!(req["params"]["session_id"], "test-session");
        assert_eq!(req["params"]["turn_id"], "test-turn");
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","method":"heartbeat","params":{}}),
        )
        .unwrap();
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":req["id"],"result":"pong"}),
        )
        .unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":"not owned"}}),
        )
        .unwrap();
    });
    let mut c = Client::connect(Options {
        socket: Some(path),
        session_id: "test-session".into(),
        turn_id: "test-turn".into(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(c.request("ping", json!({})).unwrap(), json!("pong"));
    assert_eq!(c.drain_notifications()[0]["method"], "heartbeat");
    let err = c.request("claimUserTab", json!({"tabId":7})).unwrap_err();
    assert!(err.to_string().contains("not owned"), "{err:#}");
    server.join().unwrap();
}

#[test]
fn sdk_times_out_without_replaying_mutating_requests() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join("slow.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        read_frame(&mut s).unwrap();
        thread::sleep(Duration::from_millis(250));
        let mut b = [0];
        assert_eq!(s.read(&mut b).unwrap(), 0);
    });
    let mut c = Client::connect(Options {
        socket: Some(path),
        timeout: Duration::from_millis(50),
        ..Default::default()
    })
    .unwrap();
    let start = Instant::now();
    assert!(c.request("createTab", json!({})).is_err());
    assert!(start.elapsed() < Duration::from_millis(200));
    let err = c.request("ping", json!({})).unwrap_err();
    assert!(err.to_string().contains("closed"), "{err:#}");
    server.join().unwrap();
}

#[test]
fn discovery_rejects_symlinked_socket_directories() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let target = dir.path().join("real");
    std::fs::create_dir(&target).unwrap();
    let listener = UnixListener::bind(target.join("fake.sock")).unwrap();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();
    let result = Client::connect(Options {
        socket_dir: link,
        ..Default::default()
    });
    assert!(
        result.is_err(),
        "discovery connected through a symlink directory"
    );
    drop(listener);
}
