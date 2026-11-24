//! The protocol over in-memory streams (RFC SMYSL-6 §8, LS-P1): framing, the handshake, a
//! document's diagnostics, debouncing, and the order of `shutdown` and `exit`.

use std::io::{BufReader, Cursor};

use serde_json::{json, Value};
use smysl_lsp::{serve, LspConfig, LspError};

fn frame(v: Value) -> Vec<u8> {
    let body = serde_json::to_vec(&v).unwrap();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend(body);
    out
}

/// Every frame the server wrote.
fn frames(mut bytes: &[u8]) -> Vec<Value> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let head_end = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a header");
        let head = std::str::from_utf8(&bytes[..head_end]).unwrap();
        let n: usize = head
            .strip_prefix("Content-Length: ")
            .expect("Content-Length first")
            .parse()
            .unwrap();
        let body = &bytes[head_end + 4..head_end + 4 + n];
        out.push(serde_json::from_slice(body).unwrap());
        bytes = &bytes[head_end + 4 + n..];
    }
    out
}

fn run(messages: Vec<Value>, config: LspConfig) -> (Result<(), LspError>, Vec<Value>) {
    let input: Vec<u8> = messages.into_iter().flat_map(frame).collect();
    let mut output = Vec::new();
    let r = serve(BufReader::new(Cursor::new(input)), &mut output, config);
    (r, frames(&output))
}

fn initialize(encodings: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "capabilities": { "general": { "positionEncodings": encodings } } } })
}

const DOC: &str = "@doc smysl/0.1 { id: v/t }\n\n\
    @evidence e/a { status: speculative }\n~ A 語 guess.\n\n\
    @claim c/b { status: derived, grounds: [e/a] }\n~ On 😀 a guess.\n";

fn open(text: &str) -> Value {
    json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": {
        "uri": "file:///t.smy", "languageId": "smysl", "version": 1, "text": text } } })
}

fn end() -> Vec<Value> {
    vec![
        json!({ "jsonrpc": "2.0", "id": 9, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ]
}

#[test]
fn the_handshake_a_documents_diagnostics_and_a_clean_exit() {
    let mut msgs = vec![
        initialize(json!(["utf-16"])),
        json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
        open(DOC),
    ];
    msgs.extend(end());
    let (r, out) = run(msgs, LspConfig::default());
    assert!(r.is_ok(), "{r:?}");
    assert_eq!(out[0]["id"], 1);
    assert_eq!(
        out[0]["result"]["capabilities"]["positionEncoding"],
        "utf-16"
    );
    assert_eq!(out[0]["result"]["serverInfo"]["name"], "smysl");
    let published = &out[1];
    assert_eq!(published["method"], "textDocument/publishDiagnostics");
    assert_eq!(published["params"]["uri"], "file:///t.smy");
    assert_eq!(published["params"]["version"], 1);
    let ds = published["params"]["diagnostics"].as_array().unwrap();
    let e030 = ds.iter().find(|d| d["code"] == "SMY-E030").expect("Rule M");
    assert_eq!(e030["range"]["start"], json!({ "line": 5, "character": 0 }));
    assert_eq!(e030["severity"], 1);
    assert_eq!(e030["source"], "smysl");
    assert_eq!(out[2], json!({ "jsonrpc": "2.0", "id": 9, "result": null }));
    assert_eq!(out.len(), 3);
}

#[test]
fn utf8_is_taken_when_offered() {
    let (_, out) = run(
        [vec![initialize(json!(["utf-8", "utf-16"]))], end()].concat(),
        LspConfig::default(),
    );
    assert_eq!(
        out[0]["result"]["capabilities"]["positionEncoding"],
        "utf-8"
    );
}

#[test]
fn a_burst_of_changes_is_checked_once_after_the_debounce() {
    let change = |v: i64, text: &str| {
        json!({ "jsonrpc": "2.0", "method": "textDocument/didChange", "params": {
            "textDocument": { "uri": "file:///t.smy", "version": v },
            "contentChanges": [ { "text": text } ] } })
    };
    // The client's stream stays open past the burst, so the deadline can pass: the reader
    // thread only sees `shutdown` after a pause.
    let mut first: Vec<u8> = [
        initialize(json!([])),
        open(DOC),
        change(2, "@doc smysl/0.1 { id: v/t }\n"),
        change(3, DOC),
    ]
    .into_iter()
    .flat_map(frame)
    .collect();
    let rest: Vec<u8> = end().into_iter().flat_map(frame).collect();
    struct Slow {
        a: Cursor<Vec<u8>>,
        b: Cursor<Vec<u8>>,
        paused: bool,
    }
    impl std::io::Read for Slow {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.a.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            if !self.paused {
                self.paused = true;
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            self.b.read(buf)
        }
    }
    let input = Slow {
        a: Cursor::new(std::mem::take(&mut first)),
        b: Cursor::new(rest),
        paused: false,
    };
    let mut output = Vec::new();
    let r = serve(
        BufReader::new(input),
        &mut output,
        LspConfig::default().with_check_debounce_ms(20),
    );
    assert!(r.is_ok(), "{r:?}");
    let out = frames(&output);
    let published: Vec<&Value> = out
        .iter()
        .filter(|m| m["method"] == "textDocument/publishDiagnostics")
        .collect();
    // One on open, one for the burst of two changes, at the last version.
    assert_eq!(published.len(), 2, "{out:?}");
    assert_eq!(published[1]["params"]["version"], 3);
}

#[test]
fn requests_after_shutdown_and_unknown_methods_are_errors() {
    let msgs = vec![
        initialize(json!([])),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/semanticTokens/full" }),
        json!({ "jsonrpc": "2.0", "id": 3, "method": "shutdown" }),
        json!({ "jsonrpc": "2.0", "id": 4, "method": "initialize", "params": {} }),
        json!({ "jsonrpc": "2.0", "method": "exit" }),
    ];
    let (r, out) = run(msgs, LspConfig::default());
    assert!(r.is_ok());
    assert_eq!(out[1]["error"]["code"], -32601);
    assert_eq!(out[3]["error"]["code"], -32600);
}

#[test]
fn exit_without_shutdown_and_a_closed_stream_are_errors() {
    let (r, _) = run(
        vec![
            initialize(json!([])),
            json!({ "jsonrpc": "2.0", "method": "exit" }),
        ],
        LspConfig::default(),
    );
    assert!(matches!(r, Err(LspError::Transport(_))));
    let (r, _) = run(vec![initialize(json!([]))], LspConfig::default());
    assert!(matches!(r, Err(LspError::Transport(_))));
}

#[test]
fn closing_a_document_clears_its_diagnostics() {
    let msgs = [
        vec![
            initialize(json!([])),
            open(DOC),
            json!({ "jsonrpc": "2.0", "method": "textDocument/didClose",
                    "params": { "textDocument": { "uri": "file:///t.smy" } } }),
        ],
        end(),
    ]
    .concat();
    let (_, out) = run(msgs, LspConfig::default());
    assert_eq!(out[2]["params"]["diagnostics"], json!([]));
}

#[test]
fn a_position_request_without_a_position_is_invalid_params() {
    let msgs = [
        vec![
            initialize(json!([])),
            open(DOC),
            json!({ "jsonrpc": "2.0", "id": 5, "method": "textDocument/hover",
                    "params": { "textDocument": { "uri": "file:///t.smy" } } }),
            json!({ "jsonrpc": "2.0", "id": 6, "method": "textDocument/definition",
                    "params": { "textDocument": { "uri": "file:///t.smy" },
                                "position": { "line": 0, "character": 0 } } }),
        ],
        end(),
    ]
    .concat();
    let (_, out) = run(msgs, LspConfig::default());
    assert_eq!(out[2]["error"]["code"], -32602);
    // Nothing under the cursor is `null`, not an error.
    assert_eq!(out[3]["result"], Value::Null);
}
