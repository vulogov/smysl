//! The clock `ingest --text` attests at (1.14.0).
//!
//! `IngestOptions` defaults its clock to time zero, and both ingest commands took the default, so
//! every unit they staged was attested at 1970. Rule E reads a unit's earliest attestation as a
//! ceiling on when it was said, so every chat message ingested from the CLI was "said after it
//! was first seen", and `check` reported each one contested (`SMY-W413`). The commands now attest
//! at `--at`, or at the wall clock.
//!
//! The provider is a loopback stub speaking Ollama's protocol, as in `cmd_providers.rs`. It
//! answers every chat request with one claim per message of the fixture chat, quoting the
//! message, so the units carry the messages' instants.

#![cfg(all(feature = "cli", feature = "ingest"))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("smysl-ingest-clock-{}-{name}", std::process::id()));
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

/// Read one HTTP request whole: the headers, then `Content-Length` bytes of body. Answering
/// before the body is read would close a socket the client is still writing to.
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

/// An Ollama stub: the model list for `/api/tags`, and one extraction for `/api/chat`.
fn stub(answer: String) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
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
    port
}

#[test]
fn units_ingested_from_a_chat_are_not_said_after_they_were_first_seen() {
    let dir = scratch("chat");
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/library/readers/telegram.json");
    std::fs::copy(&fixture, dir.join("chat.json")).expect("the fixture copies");

    // One claim per English message, each quoting the message's own words.
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
    let port = stub(format!(r#"{{"units":[{}]}}"#, units.join(",")));
    std::fs::write(
        dir.join(".smysl/config.hjson"),
        format!(
            "{{\n  providers: {{\n    nearby: {{ kind: ollama, endpoint: \"http://127.0.0.1:{port}\", \
             model: \"llama3.2\", context_window: 8192, max_output: 2048, structured: json-schema }}\n  }}\n  \
             routing: {{ content-ingest: nearby }}\n  fallback: [nearby]\n}}\n"
        ),
    )
    .expect("the configuration writes");

    let add = run_in(
        &dir,
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
    assert!(add.status.success(), "{}", text(&add));

    // An explicit clock after the chat, as a reproducible run gives one.
    let ingest = run_in(
        &dir,
        &[
            "ingest",
            "--text",
            "group",
            "--at",
            "1705400000000",
            "--yes",
            "-s",
            "lib",
        ],
    );
    let out = text(&ingest);
    assert!(ingest.status.success(), "{out}");
    assert!(
        !out.contains("SMY-W413"),
        "an ingested message was said after it was first seen: {out}"
    );

    let check = run_in(&dir, &["check", "lib"]);
    let out = text(&check);
    assert!(
        !out.contains("SMY-W413"),
        "check finds the ingested units' times contested: {out}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
