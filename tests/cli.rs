#![cfg(unix)]
use browser_use::wire::{read_frame, write_frame};
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::net::UnixListener,
    process::{Command, Stdio},
    thread,
};
fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_browser-use"))
}

#[test]
fn cli_open_tab_navigates_and_action_plan_reuses_default_tab_and_session() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("cli.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        for (method, result) in [
            ("nameSession", json!({})),
            ("createTab", json!({"id":42})),
            ("executeCdp", json!({"frameId":"f"})),
            ("executeCdp", json!({"result":{"value":"Example"}})),
            ("finalizeTabs", json!({})),
        ] {
            let mut req = read_frame(&mut s).unwrap().unwrap();
            if req["method"] == "attach" {
                write_frame(&mut s, &json!({"jsonrpc":"2.0","id":req["id"],"result":{}})).unwrap();
                req = read_frame(&mut s).unwrap().unwrap();
            }
            assert_eq!(req["method"], method);
            assert_eq!(req["params"]["session_id"], "task-one");
            if req["params"]["method"] == "Page.navigate" {
                assert_eq!(req["params"]["target"]["tabId"], 42);
                assert_eq!(req["params"]["commandParams"]["url"], "https://example.com");
            }
            write_frame(
                &mut s,
                &json!({"jsonrpc":"2.0","id":req["id"],"result":result}),
            )
            .unwrap();
        }
    });
    let out=binary().args(["--socket",socket.to_str().unwrap(),"--session-id","task-one","run","-c","name-session 'Example task'\nopen-tab https://example.com\neval 'document.title'\nfinalize-tabs []"]).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let values: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(values.len(), 4);
    assert_eq!(values[2], "Example");
    server.join().unwrap();
}

#[test]
fn mcp_handles_handshake_tools_bad_arguments_and_browser_requests() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("mcp.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let req = read_frame(&mut s).unwrap().unwrap();
        assert_eq!(req["method"], "getUserTabs");
        write_frame(
            &mut s,
            &json!({"jsonrpc":"2.0","id":req["id"],"result":[{"id":9,"title":"Hello"}]}),
        )
        .unwrap();
    });
    let mut child = binary()
        .args(["--socket", socket.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"navigate","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"user_tabs","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":5,"method":"missing"}),
    ];
    for m in messages {
        writeln!(child.stdin.as_mut().unwrap(), "{m}").unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let responses: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-06-18");
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "user_tabs")
    );
    assert_eq!(responses[2]["result"]["isError"], true);
    assert!(
        responses[3]["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Hello")
    );
    assert_eq!(responses[4]["error"]["code"], -32601);
    server.join().unwrap();
}

#[test]
fn manifest_is_valid_and_setup_writes_only_requested_destination() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("host.json");
    let out = binary()
        .args([
            "setup",
            "--extension-id",
            "abcdefghijklmnopabcdefghijklmnop",
            "--output",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let manifest: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(manifest["name"], "com.gaetanjuvin.browser_use");
    assert_eq!(manifest["type"], "stdio");
    assert_eq!(
        manifest["allowed_origins"][0],
        "chrome-extension://abcdefghijklmnopabcdefghijklmnop/"
    );
    assert!(std::path::Path::new(manifest["path"].as_str().unwrap()).is_absolute());
    assert!(
        !binary()
            .args(["manifest", "--extension-id", "../../bad"])
            .output()
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn help_and_invalid_commands_work_without_a_browser() {
    assert!(binary().arg("--help").output().unwrap().status.success());
    let out = binary().args(["cdp", "--tab-id", "1"]).output().unwrap();
    assert!(!out.status.success());
    let out = binary()
        .args(["run", "-c", "open-tab 'unterminated"])
        .output()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn mcp_reconnects_on_next_explicit_action_without_replaying_failed_mutation() {
    let dir = tempfile::Builder::new().tempdir_in("/tmp").unwrap();
    let socket = dir.path().join("reconnect.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        let request = read_frame(&mut first).unwrap().unwrap();
        assert_eq!(request["method"], "createTab");
        drop(first);
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok((mut s, _)) => {
                    let request = read_frame(&mut s).unwrap().unwrap();
                    assert_eq!(request["method"], "ping", "failed mutation was replayed");
                    write_frame(
                        &mut s,
                        &json!({"jsonrpc":"2.0","id":request["id"],"result":"pong"}),
                    )
                    .unwrap();
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "MCP did not reconnect"
                    );
                    thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    let mut child = binary()
        .args(["--socket", socket.to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    for m in [
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"open_tab","arguments":{}}}),
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"ping","arguments":{}}}),
    ] {
        writeln!(child.stdin.as_mut().unwrap(), "{m}").unwrap();
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    let values: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(values[1]["result"]["isError"], true);
    assert_eq!(values[2]["result"]["isError"], false);
    server.join().unwrap();
}
