//! `tel read otlp-traces/1|otlp-traces-json/1` and `tel trace` (RFC SMYSL-3 §9.6, §19.6, TL-P9
//! step 2), against the binary, over `fixtures/tel/otlp-traces/`.
//!
//! Two exports, one per service, hold one trace across both (a cart server span, its client
//! span, the payment server span it called, and that span's query) and one other trace. The
//! protobuf twins were written by `protoc` from the `.txtpb` text, and set a `trace_state`, a
//! span event and a link, which the reader does not keep.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const AT: &str = "1768557700000";
const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-trace-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel/otlp-traces")
        .join(name)
        .display()
        .to_string()
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{args:?}: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn read(dir: &Path, reader: &str, files: &[String], yes: bool) -> String {
    let mut args = vec!["tel", "read", reader];
    args.extend(files.iter().map(String::as_str));
    args.extend(["--instrument", "otel-collector:c1", "--at", AT, "-s", "lib"]);
    if yes {
        args.push("--yes");
    }
    ok(dir, &args)
}

/// The `span` lines of `tel trace`: what the substrate says the trace is.
fn spans(out: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with("span "))
        .map(str::to_string)
        .collect()
}

#[test]
fn json_and_protobuf_are_one_trace_and_it_commits_checks_and_evicts() {
    let (a, b) = (scratch("json"), scratch("pb"));
    let json = [fixture("t0.json"), fixture("t1.json")];
    let pb = [fixture("t0.pb"), fixture("t1.pb")];

    // A review writes nothing.
    let o = read(&a, "otlp-traces-json/1", &json, false);
    assert!(o.contains("5 span(s) would be added"), "{o}");
    assert!(!a.join("lib").exists());

    let o = read(&a, "otlp-traces-json/1", &json, true);
    assert!(
        o.contains("5 span(s) added to the substrate, 0 already there"),
        "{o}"
    );
    read(&b, "otlp-traces/1", &pb, true);
    let o = read(&a, "otlp-traces/1", &pb, true);
    assert!(
        o.contains("0 span(s) added to the substrate, 5 already there"),
        "{o}"
    );

    let ta = ok(&a, &["tel", "trace", TRACE, "-s", "lib"]);
    let tb = ok(&b, &["tel", "trace", TRACE, "-s", "lib"]);
    assert_eq!(spans(&ta).len(), 4, "{ta}");
    assert_eq!(spans(&ta), spans(&tb), "the two encodings are one reading");
    assert!(ta.contains("4 span(s), 3 precedes edge(s)"), "{ta}");
    assert!(!ta.contains("SMY-W617"), "{ta}");

    let o = ok(
        &a,
        &["tel", "trace", TRACE, "-s", "lib", "--yes", "--at", AT],
    );
    assert!(o.contains("record(s) committed"), "{o}");
    let c = ok(&a, &["tel", "check", "-s", "lib"]);
    assert!(
        c.contains("checked   0 window(s) and 4 span(s): 0 error(s), 0 warning(s)"),
        "{c}"
    );

    // Evicted: the units stay, and they are not re-derivable.
    ok(
        &a,
        &[
            "tel",
            "evict",
            "--older-than",
            "1d",
            "--at",
            "1768700000000",
            "-s",
            "lib",
            "--yes",
        ],
    );
    let c = ok(&a, &["tel", "check", "-s", "lib"]);
    assert_eq!(
        c.lines().filter(|l| l.contains("SMY-W605")).count(),
        4,
        "{c}"
    );
    let t = ok(&a, &["tel", "trace", TRACE, "-s", "lib"]);
    assert!(t.contains("SMY-W605") && spans(&t).is_empty(), "{t}");

    // A peer that kept them: reading the exports again brings the spans back, and the same
    // units with them.
    read(&a, "otlp-traces/1", &pb, true);
    let c = ok(&a, &["tel", "check", "-s", "lib"]);
    assert!(!c.contains("SMY-W605"), "{c}");
    assert_eq!(
        spans(&ok(&a, &["tel", "trace", TRACE, "-s", "lib"])),
        spans(&tb)
    );
}

#[test]
fn a_span_two_exports_disagree_about_is_w617_and_both_are_kept() {
    let d = scratch("w617");
    read(&d, "otlp-traces-json/1", &[fixture("t1.json")], true);
    let changed = std::fs::read_to_string(fixture("t1.json"))
        .unwrap()
        .replace("\"SELECT cards\"", "\"SELECT cards LIMIT 1\"");
    let other = d.join("t1-changed.json");
    std::fs::write(&other, changed).unwrap();
    read(
        &d,
        "otlp-traces-json/1",
        &[other.display().to_string()],
        true,
    );
    let t = ok(&d, &["tel", "trace", TRACE, "-s", "lib"]);
    assert!(t.contains("SMY-W617  span a1b2c3d4e5f60718"), "{t}");
    assert_eq!(spans(&t).len(), 3, "{t}");
}

#[test]
fn a_trace_id_is_32_hex_digits_and_an_unknown_one_is_said() {
    let d = scratch("usage");
    read(&d, "otlp-traces-json/1", &[fixture("t0.json")], true);
    assert_eq!(
        run(&d, &["tel", "trace", "xyz", "-s", "lib"]).status.code(),
        Some(2)
    );
    let o = run(
        &d,
        &[
            "tel",
            "trace",
            "0123456789abcdef0123456789abcdef",
            "-s",
            "lib",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("holds no span"));
}
