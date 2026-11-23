//! The protocol (RFC SMYSL-6 §3.2): JSON-RPC 2.0 framed by `Content-Length`, over any byte
//! streams. Everything that touches I/O, a thread, a clock or JSON is here, and nothing in
//! `analysis` does.
//!
//! One thread reads frames and sends them down a channel; the loop answers them in order. A
//! document's diagnostics are published when it opens and `check_debounce_ms` after the last
//! of a burst of changes, so a run of keystrokes is checked once.

use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::analysis::{
    AnalysisHost, Change, CompletionKind, Diagnostic, Encoding, Language, Position, Range,
    Severity, SymbolKind, TextEdit,
};
use crate::{LspConfig, LspError};

/// JSON-RPC's error codes, as LSP 3.17 uses them.
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;

/// Read one frame; `None` at the end of the stream.
pub(crate) fn read_frame(r: &mut impl BufRead) -> Result<Option<Value>, LspError> {
    let mut length: Option<usize> = None;
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let l = line.trim_end_matches(['\r', '\n']);
        if l.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((k, v)) = l.split_once(':') {
            if k.trim().eq_ignore_ascii_case("content-length") {
                length = Some(v.trim().parse().map_err(|_| {
                    LspError::Transport(format!("a Content-Length that is not a number: {v}"))
                })?);
            }
        }
    }
    let n = length.expect("set before the loop breaks");
    let mut body = vec![0; n];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|e| LspError::Transport(format!("a frame that is not JSON: {e}")))
}

/// Write one frame.
pub(crate) fn write_frame(w: &mut impl Write, v: &Value) -> Result<(), LspError> {
    let body = serde_json::to_vec(v).map_err(|e| LspError::Transport(e.to_string()))?;
    write!(w, "Content-Length: {}\r\n\r\n", body.len())?;
    w.write_all(&body)?;
    w.flush()?;
    Ok(())
}

fn position(p: Position) -> Value {
    json!({ "line": p.line, "character": p.character })
}

fn range(r: Range) -> Value {
    json!({ "start": position(r.start), "end": position(r.end) })
}

fn parse_position(v: &Value) -> Option<Position> {
    Some(Position::new(
        u32::try_from(v.get("line")?.as_u64()?).ok()?,
        u32::try_from(v.get("character")?.as_u64()?).ok()?,
    ))
}

fn parse_range(v: &Value) -> Option<Range> {
    Some(Range::new(
        parse_position(v.get("start")?)?,
        parse_position(v.get("end")?)?,
    ))
}

/// A text document position request's document and position.
fn at(params: &Value) -> Result<(String, Position), (i64, String)> {
    let uri = params
        .pointer("/textDocument/uri")
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, "no `textDocument.uri`".to_string()))?;
    let pos = params
        .get("position")
        .and_then(parse_position)
        .ok_or((INVALID_PARAMS, "no `position`".to_string()))?;
    Ok((uri.to_string(), pos))
}

fn text_edit(e: &TextEdit) -> Value {
    json!({ "range": range(e.range), "newText": e.text })
}

/// LSP 3.17's `CompletionItemKind` for each: Class, Property, EnumMember, Reference, Value,
/// Keyword.
fn completion_kind(k: CompletionKind) -> u32 {
    match k {
        CompletionKind::Type => 7,
        CompletionKind::Key => 10,
        CompletionKind::Status => 20,
        CompletionKind::Label => 18,
        CompletionKind::Kind => 12,
        CompletionKind::Keyword => 14,
    }
}

/// LSP 3.17's `SymbolKind` for each: File, Namespace, Object, Operator, Event.
fn symbol_kind(k: SymbolKind) -> u32 {
    match k {
        SymbolKind::Doc => 1,
        SymbolKind::Schema => 3,
        SymbolKind::Unit => 19,
        SymbolKind::Relation => 25,
        SymbolKind::Thread => 24,
    }
}

fn diagnostic(d: &Diagnostic) -> Value {
    json!({
        "range": range(d.range),
        "severity": match d.severity { Severity::Error => 1, Severity::Warning => 2 },
        "code": d.code,
        "source": "smysl",
        "message": d.message,
    })
}

/// The log: a file when configured, else stderr. stdout is the protocol's.
struct Log(Option<std::fs::File>);

impl Log {
    fn line(&mut self, s: &str) {
        match &mut self.0 {
            Some(f) => {
                let _ = writeln!(f, "{s}");
            }
            None => eprintln!("smysl lsp-server: {s}"),
        }
    }
}

/// The server's state between messages.
struct Server<W: Write> {
    out: W,
    host: AnalysisHost,
    shut_down: bool,
    /// Documents changed since their diagnostics were last published.
    dirty: BTreeSet<String>,
    deadline: Option<Instant>,
    debounce: Duration,
    log: Log,
    /// Messages for `window/showMessage`, sent after the reply that produced them.
    notices: Vec<String>,
}

/// What handling one message decided.
enum Next {
    Continue,
    Exit(Result<(), LspError>),
}

impl<W: Write> Server<W> {
    fn send(&mut self, v: Value) -> Result<(), LspError> {
        write_frame(&mut self.out, &v)
    }

    fn reply(&mut self, id: &Value, result: Value) -> Result<(), LspError> {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
    }

    fn fail(&mut self, id: &Value, code: i64, message: &str) -> Result<(), LspError> {
        self.send(json!({
            "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message }
        }))
    }

    fn publish(&mut self, uri: &str) -> Result<(), LspError> {
        let host = &self.host;
        let analysed =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.diagnostics(uri)));
        let ds = match analysed {
            Ok(ds) => ds,
            Err(_) => {
                // Leave the document's diagnostics as they were (§7).
                self.log.line(&format!("analysis panicked on {uri}"));
                return Ok(());
            }
        };
        let version = self.host.document(uri).map(|d| d.version);
        let mut params = json!({
            "uri": uri,
            "diagnostics": ds.iter().map(diagnostic).collect::<Vec<_>>(),
        });
        if let Some(v) = version {
            params["version"] = json!(v);
        }
        self.send(json!({
            "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": params
        }))
    }

    fn flush_dirty(&mut self) -> Result<(), LspError> {
        for uri in std::mem::take(&mut self.dirty) {
            if self.host.document(&uri).is_some() {
                self.publish(&uri)?;
            }
        }
        self.deadline = None;
        Ok(())
    }

    fn handle(&mut self, msg: Value) -> Result<Next, LspError> {
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method.as_deref(), id) {
            // A response to a request of ours; the server sends none yet.
            (None, _) => Ok(Next::Continue),
            (Some(m), Some(id)) => {
                if self.shut_down {
                    self.fail(&id, INVALID_REQUEST, "the server is shut down")?;
                    return Ok(Next::Continue);
                }
                let answered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.request(m, &params)
                }));
                match answered {
                    Ok(Ok(result)) => {
                        self.reply(&id, result)?;
                        for message in std::mem::take(&mut self.notices) {
                            self.send(json!({
                                "jsonrpc": "2.0", "method": "window/showMessage",
                                "params": { "type": 3, "message": message }
                            }))?;
                        }
                    }
                    Ok(Err((code, message))) => self.fail(&id, code, &message)?,
                    Err(_) => {
                        self.log.line(&format!("{m} panicked"));
                        self.fail(&id, INTERNAL_ERROR, "the server failed on this request")?;
                    }
                }
                Ok(Next::Continue)
            }
            (Some(m), None) => self.notification(m, &params),
        }
    }

    /// A request's result, or a JSON-RPC error.
    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let offered = params
                    .pointer("/capabilities/general/positionEncodings")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let enc = if offered.iter().any(|e| e.as_str() == Some("utf-8")) {
                    Encoding::Utf8
                } else {
                    Encoding::Utf16
                };
                self.host = AnalysisHost::new(enc);
                Ok(json!({
                    "capabilities": {
                        "positionEncoding": enc.as_str(),
                        // Open and close, and incremental changes.
                        "textDocumentSync": { "openClose": true, "change": 2 },
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "documentSymbolProvider": true,
                        "completionProvider": { "triggerCharacters": ["@", ":", "[", ",", "-", ">"] },
                        "documentFormattingProvider": true,
                        "codeActionProvider": { "codeActionKinds": ["quickfix"] },
                    },
                    "serverInfo": { "name": "smysl", "version": env!("CARGO_PKG_VERSION") },
                }))
            }
            "shutdown" => {
                self.shut_down = true;
                Ok(Value::Null)
            }
            "textDocument/hover" => {
                let (uri, pos) = at(params)?;
                Ok(match self.host.hover(&uri, pos) {
                    Some(h) => json!({
                        "contents": { "kind": "markdown", "value": h.markdown },
                        "range": range(h.range),
                    }),
                    None => Value::Null,
                })
            }
            "textDocument/definition" => {
                let (uri, pos) = at(params)?;
                Ok(match self.host.definition(&uri, pos) {
                    Some(r) => json!({ "uri": uri, "range": range(r) }),
                    None => Value::Null,
                })
            }
            "textDocument/references" => {
                let (uri, pos) = at(params)?;
                let declaration = params
                    .pointer("/context/includeDeclaration")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                Ok(Value::Array(
                    self.host
                        .references(&uri, pos, declaration)
                        .into_iter()
                        .map(|r| json!({ "uri": uri, "range": range(r) }))
                        .collect(),
                ))
            }
            "textDocument/documentSymbol" => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "no `textDocument.uri`".to_string()))?;
                Ok(Value::Array(
                    self.host
                        .symbols(uri)
                        .into_iter()
                        .map(|s| {
                            json!({
                                "name": s.name,
                                "detail": s.detail,
                                "kind": symbol_kind(s.kind),
                                "range": range(s.range),
                                "selectionRange": range(s.selection),
                            })
                        })
                        .collect(),
                ))
            }
            "textDocument/completion" => {
                let (uri, pos) = at(params)?;
                Ok(Value::Array(
                    self.host
                        .completion(&uri, pos)
                        .into_iter()
                        .map(|c| {
                            json!({
                                "label": c.label,
                                "kind": completion_kind(c.kind),
                                "detail": c.detail,
                                "textEdit": { "range": range(c.replace), "newText": c.label },
                            })
                        })
                        .collect(),
                ))
            }
            "textDocument/formatting" => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "no `textDocument.uri`".to_string()))?
                    .to_string();
                match self.host.format(&uri) {
                    Ok(Some(e)) => Ok(json!([text_edit(&e)])),
                    Ok(None) => Ok(json!([])),
                    Err(why) => {
                        // Refused rather than lossy (§5.5): say why, and change nothing.
                        self.notices.push(why);
                        Ok(Value::Null)
                    }
                }
            }
            "textDocument/codeAction" => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "no `textDocument.uri`".to_string()))?
                    .to_string();
                let r = params
                    .get("range")
                    .and_then(parse_range)
                    .ok_or((INVALID_PARAMS, "no `range`".to_string()))?;
                Ok(Value::Array(
                    self.host
                        .actions(&uri, r)
                        .into_iter()
                        .map(|a| {
                            json!({
                                "title": a.title,
                                "kind": "quickfix",
                                "edit": { "changes": {
                                    uri.clone(): a.edits.iter().map(text_edit).collect::<Vec<_>>()
                                } },
                            })
                        })
                        .collect(),
                ))
            }
            _ => Err((METHOD_NOT_FOUND, format!("`{method}` is not served"))),
        }
    }

    fn notification(&mut self, method: &str, params: &Value) -> Result<Next, LspError> {
        match method {
            "exit" => {
                return Ok(Next::Exit(if self.shut_down {
                    Ok(())
                } else {
                    Err(LspError::Transport("`exit` before `shutdown`".into()))
                }))
            }
            "textDocument/didOpen" => {
                let Some(td) = params.get("textDocument") else {
                    return Ok(Next::Continue);
                };
                let uri = td.get("uri").and_then(Value::as_str).unwrap_or_default();
                let lang = td
                    .get("languageId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let Some(language) = Language::of(lang, uri) else {
                    return Ok(Next::Continue);
                };
                let version = td.get("version").and_then(Value::as_i64).unwrap_or(0);
                let text = td.get("text").and_then(Value::as_str).unwrap_or_default();
                self.host.open(uri, language, version, text.to_string());
                self.dirty.remove(uri);
                self.publish(uri)?;
            }
            "textDocument/didChange" => {
                let uri = params
                    .pointer("/textDocument/uri")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let version = params
                    .pointer("/textDocument/version")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                let changes: Vec<Change> = params
                    .get("contentChanges")
                    .and_then(Value::as_array)
                    .map(|cs| {
                        cs.iter()
                            .filter_map(|c| {
                                let text = c.get("text")?.as_str()?.to_string();
                                Some(match c.get("range").and_then(parse_range) {
                                    Some(range) => Change::Range { range, text },
                                    None => Change::Full(text),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if self.host.change(&uri, version, changes) {
                    self.dirty.insert(uri);
                    self.deadline = Some(Instant::now() + self.debounce);
                }
            }
            "textDocument/didClose" => {
                if let Some(uri) = params.pointer("/textDocument/uri").and_then(Value::as_str) {
                    let uri = uri.to_string();
                    self.host.close(&uri);
                    self.dirty.remove(&uri);
                    self.send(json!({
                        "jsonrpc": "2.0", "method": "textDocument/publishDiagnostics",
                        "params": { "uri": uri, "diagnostics": [] }
                    }))?;
                }
            }
            // `initialized`, `$/cancelRequest`, `$/setTrace` and anything else: nothing to do.
            _ => {}
        }
        Ok(Next::Continue)
    }
}

/// Serve over `input` and `output` until `exit` or the end of `input`.
pub(crate) fn serve<R, W>(input: R, output: W, config: LspConfig) -> Result<(), LspError>
where
    R: BufRead + Send + 'static,
    W: Write,
{
    let log = Log(match &config.log_file {
        Some(p) => Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)?,
        ),
        None => None,
    });
    let (tx, rx) = mpsc::channel::<Result<Option<Value>, LspError>>();
    std::thread::spawn(move || {
        let mut input = input;
        loop {
            let frame = read_frame(&mut input);
            let end = !matches!(frame, Ok(Some(_)));
            if tx.send(frame).is_err() || end {
                return;
            }
        }
    });
    let mut s = Server {
        out: output,
        host: AnalysisHost::new(Encoding::Utf16),
        shut_down: false,
        dirty: BTreeSet::new(),
        deadline: None,
        debounce: Duration::from_millis(config.check_debounce_ms),
        log,
        notices: Vec::new(),
    };
    loop {
        let next = match s.deadline {
            Some(d) => match rx.recv_timeout(d.saturating_duration_since(Instant::now())) {
                Ok(m) => Some(m),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    s.flush_dirty()?;
                    continue;
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => None,
            },
            None => rx.recv().ok(),
        };
        let msg = match next {
            Some(Ok(Some(m))) => m,
            Some(Err(e)) => {
                s.log.line(&e.to_string());
                return Err(e);
            }
            // The client closed the stream: as `exit`, clean only after `shutdown`.
            Some(Ok(None)) | None => {
                return if s.shut_down {
                    Ok(())
                } else {
                    Err(LspError::Transport("the client closed the stream".into()))
                }
            }
        };
        match s.handle(msg)? {
            Next::Continue => {}
            Next::Exit(r) => return r,
        }
    }
}
