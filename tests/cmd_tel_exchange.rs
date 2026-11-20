//! `lib export --with-substrate` and `lib import` of what it writes (RFC SMYSL-3 §19.5, TL-P8
//! step 3), against the binary.
//!
//! Two libraries read the same three OTLP exports and materialise the same window over them.
//! One of them then evicts every sample (rule TL); the other keeps them. Then:
//!
//! - **The evicted library's export** carries no samples and lists the evicted days. Imported
//!   into a fresh library, its records arrive and its window is `SMY-W605`.
//! - **The peer's export**, imported on top, brings the samples back as a set. No unit changes,
//!   and the window recomputes clean.
//! - **Importing again** adds nothing, and a day file that is not its name adds nothing at all.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::text::library::Library;
use smysl::StoreRead;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const DAY: u64 = 86_400_000;
const SUM: &str = r#"otel:{"attributes":{},"kind":"histogram","monotonic":false,"name":"http.server.duration","part":"sum","resource":{"service.instance.id":"c1","service.name":"cart"},"scope":{"name":"io.opentelemetry.http","version":"1.2.0"},"temporality":"cumulative","unit":"s"}"#;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("smysl-tel-exchange-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
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
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

/// A library at `dir/lib` holding the three exports and one window over them.
fn library(dir: &Path) {
    let paths: Vec<String> = (0..3)
        .map(|n| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("fixtures/tel/otlp/e{n}.pb"))
                .display()
                .to_string()
        })
        .collect();
    let t0 = T0.to_string();
    let mut args = vec!["tel", "read", "otlp/1"];
    args.extend(paths.iter().map(String::as_str));
    args.extend([
        "--instrument",
        "otel-collector:c1",
        "--at",
        &t0,
        "-s",
        "lib",
        "--yes",
    ]);
    ok(dir, &args);
    let (from, to) = (T0.to_string(), (T0 + 60_000).to_string());
    ok(
        dir,
        &[
            "tel", "window", SUM, &from, &to, "-s", "lib", "--at", &to, "--yes",
        ],
    );
}

fn records(lib: &Path) -> (usize, usize, usize) {
    let lib = Library::open(lib).unwrap();
    let s = lib.store();
    (
        StoreRead::units(s).count(),
        s.relations().count(),
        StoreRead::units(s).map(|(_, u)| u.attestations.len()).sum(),
    )
}

fn export(dir: &Path, out: &str) -> String {
    ok(
        dir,
        &[
            "lib",
            "export",
            "--scope",
            "all",
            "--with-substrate",
            "-o",
            out,
            "-s",
            "lib",
        ],
    )
}

fn import(dir: &Path, from: &Path, lib: &str) -> Output {
    run(
        dir,
        &[
            "lib",
            "import",
            &from.display().to_string(),
            "--accept",
            "-s",
            lib,
        ],
    )
}

fn w605(dir: &Path, lib: &str) -> usize {
    ok(dir, &["tel", "check", "-s", lib])
        .lines()
        .filter(|l| l.contains("SMY-W605"))
        .count()
}

#[test]
fn an_evicted_export_and_an_unevicted_peer_converge_without_changing_a_unit() {
    let (mine, peer) = (scratch("mine"), scratch("peer"));
    library(&mine);
    library(&peer);
    let at = (T0 + DAY).to_string();
    ok(
        &mine,
        &[
            "tel",
            "evict",
            "--older-than",
            "1h",
            "--at",
            &at,
            "-s",
            "lib",
            "--yes",
        ],
    );
    assert_eq!(w605(&mine, "lib"), 1);

    let o = export(&mine, "exp");
    assert!(o.contains("substrate 0 day file(s), 0 sample(s)"), "{o}");
    assert!(!o.contains("; 0 day(s) listed as evicted"), "{o}");
    let o = export(&peer, "exp");
    assert!(!o.contains("substrate 0 day file(s)"), "{o}");
    assert!(o.contains("0 day(s) listed as evicted"), "{o}");
    // The export still verifies: the substrate is beside the collection, not in its index.
    ok(&mine, &["lib", "verify", "exp"]);

    // A fresh library: my export, then the peer's.
    let fresh = scratch("fresh");
    let o = import(&fresh, &mine.join("exp"), "lib");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let after_mine = records(&fresh.join("lib"));
    assert_eq!(after_mine, records(&mine.join("lib")));
    assert_eq!(w605(&fresh, "lib"), 1, "no samples came with it");

    let o = import(&fresh, &peer.join("exp"), "lib");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("0 already held"), "{out}");
    assert_eq!(
        records(&fresh.join("lib")),
        after_mine,
        "a substrate resurrection changes no unit"
    );
    let c = ok(&fresh, &["tel", "check", "-s", "lib"]);
    assert!(!c.contains("SMY-W605") && !c.contains("SMY-E604"), "{c}");

    // Again: a set, so nothing new.
    let o = import(&fresh, &peer.join("exp"), "lib");
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains(": 0 new sample(s)"), "{out}");

    // The peer's export into mine restores my samples too, and changes no unit.
    let before = records(&mine.join("lib"));
    let o = import(&mine, &peer.join("exp"), "lib");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(records(&mine.join("lib")), before);
    assert_eq!(w605(&mine, "lib"), 0);
}

#[test]
fn a_day_file_that_is_not_its_name_adds_no_sample() {
    let peer = scratch("tamper");
    library(&peer);
    export(&peer, "exp");
    let sub = peer.join("exp/substrate");
    let victim = std::fs::read_dir(&sub)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap() != "listing.cbor")
        .expect("a day file");
    let mut b = std::fs::read(&victim).unwrap();
    let last = b.len() - 1;
    b[last] ^= 1;
    std::fs::write(&victim, b).unwrap();

    let fresh = scratch("tamper-fresh");
    let o = import(&fresh, &peer.join("exp"), "lib");
    assert_eq!(o.status.code(), Some(1));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(
        err.contains("not the file its name says") && err.contains("nothing was imported"),
        "{err}"
    );
    assert!(
        !Library::is_library(&fresh.join("lib")) || records(&fresh.join("lib")).0 == 0,
        "neither records nor samples"
    );
}

#[test]
fn with_substrate_reads_a_library() {
    let dir = scratch("usage");
    std::fs::write(dir.join("plain.smysl"), b"").unwrap();
    let o = run(
        &dir,
        &[
            "lib",
            "export",
            "--scope",
            "all",
            "--with-substrate",
            "-o",
            "exp",
            "-s",
            "plain.smysl",
        ],
    );
    assert_eq!(o.status.code(), Some(2));
}
