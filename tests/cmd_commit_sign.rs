//! RFC SMYSL-8 TR-P3: `--sign --key` at the commit, on `ingest` and `tel read`.
//!
//! The phase's gate for signing at the commit: `tel read --sign` gives a library that passes
//! `verify --mode strict`; `ingest --text --sign` signs every window it committed; and a key for
//! another agent exits 2 before any provider is called, asserted with a provider that counts its
//! connections.

#![cfg(all(
    feature = "cli",
    feature = "sign",
    feature = "ingest",
    feature = "text",
    feature = "tel"
))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-commit-sign-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join(".smysl")).expect("a scratch directory");
    dir
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary under test runs")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn expect(dir: &Path, code: i32, args: &[&str]) -> String {
    let o = run_in(dir, args);
    assert_eq!(
        o.status.code(),
        Some(code),
        "smysl {}\n{}",
        args.join(" "),
        text(&o)
    );
    text(&o)
}

/// Make a key for `agent` and a key file listing it.
fn enrol(dir: &Path, agent: &str, file: &str) {
    let entry = run_in(dir, &["keygen", "--agent", agent, "--out", file]);
    assert!(entry.status.success(), "{}", text(&entry));
    std::fs::write(
        dir.join(".smysl/keys.hjson"),
        format!(
            "{{ keys: [\n{}\n] }}\n",
            String::from_utf8_lossy(&entry.stdout).trim()
        ),
    )
    .unwrap();
}

fn read_request(s: &mut std::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_lowercase();
            let len = head
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() >= end + 4 + len {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// An Ollama stub that counts the connections it accepts, and its configuration.
fn stub(dir: &Path, answer: String) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let counted = calls.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            counted.fetch_add(1, Ordering::SeqCst);
            let request = read_request(&mut s);
            let body = if request.starts_with("POST /api/chat") {
                serde_json::json!({
                    "model": "llama3.2",
                    "message": { "role": "assistant", "content": answer },
                    "done": true,
                    "prompt_eval_count": 100,
                    "eval_count": 50,
                })
                .to_string()
            } else {
                r#"{"models":[{"name":"llama3.2"}]}"#.to_string()
            };
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body.as_bytes());
            let _ = s.flush();
        }
    });
    std::fs::write(
        dir.join(".smysl/config.hjson"),
        format!(
            "{{\n  providers: {{\n    nearby: {{ kind: ollama, endpoint: \"http://127.0.0.1:{port}\", \
             model: \"llama3.2\", context_window: 8192, max_output: 2048, structured: json-schema }}\n  }}\n  \
             routing: {{ content-ingest: nearby }}\n  fallback: [nearby]\n}}\n"
        ),
    )
    .expect("the configuration writes");
    calls
}

/// A library holding the fixture chat, and the stub answering one claim a message.
fn chat_library(dir: &Path) -> Arc<AtomicUsize> {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/library/readers/telegram.json");
    std::fs::copy(&fixture, dir.join("chat.json")).expect("the fixture copies");
    let quotes = [
        "I finished the first chapter last night.",
        "And what did you make of it?",
        "Of course. We meet on Thursday.",
    ];
    let units: Vec<String> = quotes
        .iter()
        .enumerate()
        .map(|(i, q)| {
            serde_json::json!({
                "type": "claim",
                "label": format!("c/m{i}"),
                "gist": format!("A reader said: {q}"),
                "status": "cited",
                "quote": q,
            })
            .to_string()
        })
        .collect();
    let calls = stub(dir, format!(r#"{{"units":[{}]}}"#, units.join(",")));
    expect(
        dir,
        0,
        &[
            "text",
            "add",
            "chat.json",
            "--reader",
            "telegram/1",
            "--alias",
            "group",
            "--lang",
            "en",
            "--licence",
            "CC0-1.0",
            "-s",
            "lib",
        ],
    );
    calls
}

#[test]
fn ingest_text_signs_what_it_committed() {
    let dir = scratch("text");
    let calls = chat_library(&dir);
    enrol(&dir, "tool:smysl-ingest", "ingest.key");
    let out = expect(
        &dir,
        0,
        &[
            "ingest",
            "--text",
            "group",
            "--at",
            "1705400000000",
            "--yes",
            "--sign",
            "--key",
            "ingest.key",
            "-s",
            "lib",
        ],
    );
    assert!(calls.load(Ordering::SeqCst) > 0, "the provider was called");
    assert!(out.contains("signed "), "{out}");
    assert!(!out.contains("signed 0 "), "{out}");
    // Every attestation the ingest made verifies; `text add`'s own are out of scope.
    let v = expect(
        &dir,
        0,
        &[
            "verify",
            "--mode",
            "selective",
            "--agent-prefix",
            "tool:smysl-ingest",
            "--keys",
            ".smysl/keys.hjson",
            "-s",
            "lib",
        ],
    );
    assert!(v.contains("passed"), "{v}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_key_for_another_agent_exits_before_any_provider_call() {
    let dir = scratch("mismatch");
    let calls = chat_library(&dir);
    let before = calls.load(Ordering::SeqCst);
    enrol(&dir, "human:alice", "alice.key");

    let out = expect(
        &dir,
        2,
        &[
            "ingest",
            "--text",
            "group",
            "--yes",
            "--sign",
            "--key",
            "alice.key",
            "-s",
            "lib",
        ],
    );
    assert!(out.contains("attests as tool:smysl-ingest"), "{out}");

    std::fs::write(dir.join("note.md"), "The pool is small.\n").unwrap();
    let out = expect(
        &dir,
        2,
        &[
            "ingest",
            "note.md",
            "--yes",
            "--sign",
            "--key",
            "alice.key",
            "-s",
            "s.cbor",
        ],
    );
    assert!(out.contains("attests as tool:smysl-ingest"), "{out}");
    // Without --yes there is nothing to sign: the staged batch is signed at merge.
    let out = expect(
        &dir,
        2,
        &["ingest", "note.md", "--sign", "--key", "alice.key"],
    );
    assert!(out.contains("merge --staged --sign"), "{out}");

    assert_eq!(
        calls.load(Ordering::SeqCst),
        before,
        "a refused key reached the provider"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn tel_read_sign_gives_a_library_that_verifies_strict() {
    let dir = scratch("tel");
    enrol(&dir, "tool:smysl-tel", "tel.key");
    let base = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel/diff/base.json")
        .display()
        .to_string();
    let read = |extra: &[&str], code: i32| {
        let mut args = vec![
            "tel",
            "read",
            "k8s/1",
            base.as_str(),
            "--profile",
            "k8s-deployments/1",
            "--at",
            "1768557600000",
            "--licence",
            "unknown",
            "-s",
            "lib",
        ];
        args.extend_from_slice(extra);
        expect(&dir, code, &args)
    };
    // A key for another agent is refused before anything is read.
    let other = run_in(
        &dir,
        &["keygen", "--agent", "human:alice", "--out", "alice.key"],
    );
    assert!(other.status.success());
    read(&["--yes", "--sign", "--key", "alice.key"], 2);
    assert!(!dir.join("lib").exists(), "a refused key wrote nothing");
    // --sign without --yes: nothing would be written to sign.
    read(&["--sign", "--key", "tel.key"], 2);

    let out = read(&["--yes", "--sign", "--key", "tel.key"], 0);
    assert!(
        out.contains("signed ") && !out.contains("signed 0 "),
        "{out}"
    );
    let v = expect(
        &dir,
        0,
        &[
            "verify",
            "--mode",
            "strict",
            "--keys",
            ".smysl/keys.hjson",
            "-s",
            "lib",
        ],
    );
    assert!(v.contains("passed"), "{v}");
    // The library did not exist when the key was checked; its signatures still go inside it.
    assert!(dir.join("lib/log/sigs.cbor").exists());
    assert!(
        !dir.join("lib.sig").exists(),
        "signatures written beside, not inside, the library"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
