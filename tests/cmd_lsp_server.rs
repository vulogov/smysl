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

/// LS-P2's gate: hover, definition, references and the outline through the binary, each answer
/// asserted whole.
#[test]
fn navigation_through_the_binary_answers_exactly() {
    let text = "@doc smysl/0.1 { id: v/t }\n\n\
                @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n\
                ~ Latency rose.\n\n\
                @claim c/b { status: derived, grounds: [e/a] }\n\
                ~ It regressed.\n";
    let out = smysl::surface::parse_surface(text).unwrap();
    let uid_a = out.uid_of(&"e/a".parse().unwrap()).unwrap();
    let col =
        |line: usize, needle: &str| text.lines().nth(line).unwrap().find(needle).unwrap() as u64;
    let r = |line: u64, a: u64, b: u64| json!({ "start": { "line": line, "character": a }, "end": { "line": line, "character": b } });

    let mut child = Command::new(env!("CARGO_BIN_EXE_smysl"))
        .arg("lsp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    let caps = receive(&mut stdout)["result"]["capabilities"].clone();
    for c in [
        "hoverProvider",
        "definitionProvider",
        "referencesProvider",
        "documentSymbolProvider",
    ] {
        assert_eq!(caps[c], true, "{c}");
    }
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": {
            "uri": "file:///w/t.smy", "languageId": "smysl", "version": 1, "text": text } } }),
    );
    let _published = receive(&mut stdout);
    let doc = json!({ "uri": "file:///w/t.smy" });
    let ref_col = col(5, "e/a");
    let decl_col = col(2, "e/a");

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover", "params": {
            "textDocument": doc, "position": { "line": 5, "character": ref_col + 1 } } }),
    );
    let hover = receive(&mut stdout)["result"].clone();
    let tokens = smysl::tokens("Latency rose.");
    assert_eq!(
        hover,
        json!({
            "contents": { "kind": "markdown", "value": format!(
                "**e/a** · `@evidence` · measured\n\n`{uid_a}`\n\n> Latency rose.\n\n\
                 gist: {tokens} tokens of `l0_max` 30\n\nsource: metric `p95`") },
            "range": r(5, ref_col, ref_col + 3),
        })
    );

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/definition", "params": {
            "textDocument": doc, "position": { "line": 5, "character": ref_col } } }),
    );
    assert_eq!(
        receive(&mut stdout)["result"],
        json!({ "uri": "file:///w/t.smy", "range": r(2, decl_col, decl_col + 3) })
    );

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "textDocument/references", "params": {
            "textDocument": doc, "position": { "line": 2, "character": decl_col },
            "context": { "includeDeclaration": true } } }),
    );
    assert_eq!(
        receive(&mut stdout)["result"],
        json!([
            { "uri": "file:///w/t.smy", "range": r(2, decl_col, decl_col + 3) },
            { "uri": "file:///w/t.smy", "range": r(5, ref_col, ref_col + 3) },
        ])
    );

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "textDocument/documentSymbol", "params": {
            "textDocument": doc } }),
    );
    let line_len = |l: usize| text.lines().nth(l).unwrap().len() as u64;
    let range2 = |a: u64, b: u64, c: u64| json!({ "start": { "line": a, "character": 0 }, "end": { "line": b, "character": c } });
    assert_eq!(
        receive(&mut stdout)["result"],
        json!([
            { "name": "v/t", "detail": "smysl/0.1", "kind": 1,
              "range": r(0, 0, line_len(0)), "selectionRange": r(0, 0, line_len(0)) },
            { "name": "e/a", "detail": "@evidence · measured — Latency rose.", "kind": 19,
              "range": range2(2, 4, 0), "selectionRange": r(2, decl_col, decl_col + 3) },
            { "name": "c/b", "detail": "@claim · derived — It regressed.", "kind": 19,
              "range": range2(5, 6, line_len(6)), "selectionRange": r(5, col(5, "c/b"), col(5, "c/b") + 3) },
        ])
    );

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "shutdown" }),
    );
    let _ = receive(&mut stdout);
    send(&mut stdin, json!({ "jsonrpc": "2.0", "method": "exit" }));
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

/// LS-P3 through the binary: completion, formatting and its refusal, and a quick fix that
/// lowers a status.
#[test]
fn completion_formatting_and_a_quick_fix_through_the_binary() {
    let text = "@doc smysl/0.1 { id: v/t }\n\n\
                @evidence e/a { status: speculative }\n~ A guess.\n\n\
                @claim c/b   { status: derived, grounds: [e/a] }\n~ Over-claimed.\n";
    let mut child = Command::new(env!("CARGO_BIN_EXE_smysl"))
        .arg("lsp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }),
    );
    let caps = receive(&mut stdout)["result"]["capabilities"].clone();
    assert_eq!(caps["documentFormattingProvider"], true);
    assert_eq!(
        caps["codeActionProvider"]["codeActionKinds"],
        json!(["quickfix"])
    );
    let open = |uri: &str, text: &str| {
        json!({ "jsonrpc": "2.0", "method": "textDocument/didOpen", "params": { "textDocument": {
            "uri": uri, "languageId": "smysl", "version": 1, "text": text } } })
    };
    send(&mut stdin, open("file:///w/t.smy", text));
    let _ = receive(&mut stdout);
    let doc = json!({ "uri": "file:///w/t.smy" });

    // `status: ` — the five a writer may use.
    let col = text.lines().nth(5).unwrap().find("derived").unwrap() as u64;
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 2, "method": "textDocument/completion", "params": {
            "textDocument": doc, "position": { "line": 5, "character": col } } }),
    );
    let labels: Vec<String> = receive(&mut stdout)["result"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        labels,
        ["cited", "derived", "inferred", "measured", "speculative"]
    );

    // The fix for Rule M lowers `derived` to `speculative`, and only that.
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 3, "method": "textDocument/codeAction", "params": {
            "textDocument": doc,
            "range": { "start": { "line": 5, "character": 0 }, "end": { "line": 5, "character": 0 } },
            "context": { "diagnostics": [] } } }),
    );
    assert_eq!(
        receive(&mut stdout)["result"],
        json!([{
            "title": "Lower status to `speculative`, its weakest ground's",
            "kind": "quickfix",
            "edit": { "changes": { "file:///w/t.smy": [{
                "range": { "start": { "line": 5, "character": col },
                           "end": { "line": 5, "character": col + 7 } },
                "newText": "speculative" }] } },
        }])
    );

    // A clean document is formatted as one whole-text edit.
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 4, "method": "textDocument/formatting", "params": {
            "textDocument": doc, "options": { "tabSize": 2, "insertSpaces": true } } }),
    );
    let edits = receive(&mut stdout)["result"].clone();
    assert_eq!(edits.as_array().unwrap().len(), 1);
    assert!(edits[0]["newText"]
        .as_str()
        .unwrap()
        .contains("@claim c/b {"));

    // With a comment it is refused, and the editor is told why.
    send(
        &mut stdin,
        open("file:///w/c.smy", &format!("{text}# keep me\n")),
    );
    let _ = receive(&mut stdout);
    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 5, "method": "textDocument/formatting", "params": {
            "textDocument": { "uri": "file:///w/c.smy" }, "options": {} } }),
    );
    assert_eq!(receive(&mut stdout)["result"], Value::Null);
    let shown = receive(&mut stdout);
    assert_eq!(shown["method"], "window/showMessage");
    assert!(shown["params"]["message"]
        .as_str()
        .unwrap()
        .contains("comment"));

    send(
        &mut stdin,
        json!({ "jsonrpc": "2.0", "id": 6, "method": "shutdown" }),
    );
    let _ = receive(&mut stdout);
    send(&mut stdin, json!({ "jsonrpc": "2.0", "method": "exit" }));
    assert_eq!(child.wait().unwrap().code(), Some(0));
}
