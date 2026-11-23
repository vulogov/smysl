//! `smysl lsp-server` (RFC SMYSL-6, LS-P1): the real binary, spoken to as an editor would,
//! over its stdin and stdout.

#![cfg(all(feature = "cli", feature = "lsp"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

fn send(w: &mut impl Write, v: Value) {
    let body = serde_json::to_vec(&v).unwrap();
    write!(w, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
    w.write_all(&body).unwrap();
    w.flush().unwrap();
}

fn receive(r: &mut impl BufRead) -> Value {
    let mut n = 0usize;
    loop {
        let mut line = String::new();
        assert!(
            r.read_line(&mut line).unwrap() > 0,
            "the server closed stdout"
        );
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        if let Some(v) = l.strip_prefix("Content-Length: ") {
            n = v.parse().unwrap();
        }
    }
    let mut body = vec![0; n];
    r.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[test]
fn an_editor_session_through_the_binary() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_smysl"))
        .args(["lsp-server", "--stdio", "--debounce-ms", "10"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "capabilities": { "general": { "positionEncodings": ["utf-16"] } } } }),
    );
    let init = receive(&mut stdout);
    assert_eq!(init["result"]["capabilities"]["positionEncoding"], "utf-16");
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    );

    let text = "@doc smysl/0.1 { id: v/t }\n\n\
                @evidence e/a { status: speculative }\n~ Ελληνικά guess.\n\n\
                @claim c/b { status: derived, grounds: [e/a] }\n~ A claim.\n\n\
                @rel c/b --elaborates--> e/missing\n";
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": {
            "uri": "file:///w/t.smy", "languageId": "smysl", "version": 1, "text": text } } }),
    );
    let published = receive(&mut stdout);
    assert_eq!(published["method"], "textDocument/publishDiagnostics");
    let ds = published["params"]["diagnostics"].as_array().unwrap();
    let codes: Vec<&str> = ds.iter().map(|d| d["code"].as_str().unwrap()).collect();
    assert!(codes.contains(&"SMY-E030"), "{codes:?}");
    assert!(codes.contains(&"SMY-E060"), "{codes:?}");

    // Raise the ground's status the honest way, with a source, and the finding goes.
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": { "uri": "file:///w/t.smy", "version": 2 },
            "contentChanges": [ {
                "range": { "start": { "line": 2, "character": 24 },
                           "end": { "line": 2, "character": 35 } },
                "text": "measured, source: { kind: metric, ref: \"m\" }" } ] } }),
    );
    let published = receive(&mut stdout);
    assert_eq!(published["params"]["version"], 2);
    let codes: Vec<String> = published["params"]["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_string())
        .collect();
    assert!(!codes.contains(&"SMY-E030".to_string()), "{codes:?}");

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "shutdown" }),
    );
    assert_eq!(receive(&mut stdout)["result"], Value::Null);
    send(&mut stdin, json!({ "jsonrpc": "2.0", "method": "exit" }));
    let status = child.wait().unwrap();
    assert_eq!(status.code(), Some(0));
    let mut rest = String::new();
    stdout.read_to_string(&mut rest).unwrap();
    assert!(rest.is_empty(), "nothing after the protocol: {rest:?}");
}

#[test]
fn exit_without_shutdown_is_exit_1() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_smysl"))
        .arg("lsp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    send(&mut stdin, json!({ "jsonrpc": "2.0", "method": "exit" }));
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).contains("before `shutdown`"));
}
