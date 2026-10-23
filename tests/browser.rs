#![cfg(unix)]
use browser_use::{
    Browser, Options,
    wire::{read_frame, write_frame},
};
use serde_json::json;
use std::{
    io::Read,
    os::unix::net::UnixListener,
    thread,
    time::{Duration, Instant},
};

#[test]
fn load_wait_obeys_deadline_even_when_cdp_is_slow() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join("slow.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let attach = read_frame(&mut s).unwrap().unwrap();
        assert_eq!(attach["method"], "attach");
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":attach["id"],"result":{}}),
        )
        .unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        assert_eq!(req["params"]["method"], "Runtime.evaluate");
        thread::sleep(Duration::from_millis(250));
        let _ = write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":req["id"],"result":{"result":{"value":false}}}),
        );
        let mut b = [0];
        let _ = s.read(&mut b);
    });
    let mut browser = Browser::connect(Options {
        socket: Some(path),
        timeout: Duration::from_secs(1),
        ..Default::default()
    })
    .unwrap();
    let start = Instant::now();
    assert!(
        browser
            .tab(42)
            .wait_for_load("load", Duration::from_millis(50))
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_millis(200));
    drop(browser);
    server.join().unwrap();
}

#[test]
fn sdk_preserves_arbitrary_json_results_and_surfaces_javascript_exceptions() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join("eval.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        for result in [
            json!({"__rpc_error":{"message":"ordinary user data"}}),
            json!({"exceptionDetails":{"text":"ReferenceError"}}),
        ] {
            let mut req = read_frame(&mut s).unwrap().unwrap();
            if req["method"] == "attach" {
                write_frame(&mut s, &json!({"jsonrpc":"2.0","id":req["id"],"result":{}})).unwrap();
                req = read_frame(&mut s).unwrap().unwrap();
            }
            write_frame(
                &mut s,
                &json!({"jsonrpc":"2.0","id":req["id"],"result":result}),
            )
            .unwrap();
        }
    });
    let mut browser = Browser::connect(Options {
        socket: Some(path),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        browser.client.request("custom", json!({})).unwrap()["__rpc_error"]["message"],
        "ordinary user data"
    );
    let err = browser.tab(42).evaluate("missingVariable").unwrap_err();
    assert!(err.to_string().contains("ReferenceError"));
    server.join().unwrap();
}

#[test]
fn cdp_helper_attaches_the_session_tab_before_sending_a_command() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let path = dir.path().join("attach.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        if req["method"] != "attach" {
            write_frame(&mut s,&json!({"jsonrpc":"2.0","id":req["id"],"error":{"code":-32000,"message":"Debugger unattached"}})).unwrap();
            return;
        }
        assert_eq!(req["params"]["tabId"], 42);
        write_frame(&mut s, &json!({"jsonrpc":"2.0","id":req["id"],"result":{}})).unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        assert_eq!(req["method"], "executeCdp");
        assert_eq!(req["params"]["target"]["tabId"], 42);
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":req["id"],"result":{"frameId":"main"}}),
        )
        .unwrap();
    });
    let mut browser = Browser::connect(Options {
        socket: Some(path),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(
        browser.tab(42).goto("https://example.com").unwrap()["frameId"],
        "main"
    );
    server.join().unwrap();
}
