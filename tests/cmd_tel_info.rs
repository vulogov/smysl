//! `smysl tel info` (RFC SMYSL-5 §13, TI-P1, TI-P2), against the binary, over the GE-M3 samples read
//! as one OTLP/JSON export.
//!
//! - A statistic prints its unit without `--yes` and writes nothing; with it, it commits.
//! - A second run into a fresh library gives the same uids: replay, end to end.
//! - A recipe that is not admitted is exit 2; a statistic not run is reported, exit 0.
//! - `mi/1` reads `--with` and commits one edge to each series; `jsd/1` reads its `ref`.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const DURING: &str = "1768550400000-1768564800000";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-info-{}-{name}", std::process::id()));
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

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn series(name: &str) -> String {
    format!(
        r#"otel:{{"attributes":{{}},"kind":"gauge","monotonic":false,"name":"ge_m3_{name}","resource":{{"service.name":"ge-m3"}},"scope":{{"name":"sim","version":"1"}},"temporality":"unspecified","unit":"1"}}"#
    )
}

/// A library with the GE-M3 samples, and the recipes the tests use.
fn library(name: &str) -> PathBuf {
    let dir = scratch(name);
    let f = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/detect/series.otlp.json");
    let o = run(
        &dir,
        &[
            "tel",
            "read",
            "otlp-json/1",
            f.to_str().unwrap(),
            "--instrument",
            "otel-collector:ge-m3",
            "--at",
            "1768550400000",
            "-s",
            "lib",
            "--yes",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    for (file, text) in [
        (
            "ent.hjson",
            r#"{ stat: ent/1, step: 60000, bins: "width:1" }"#,
        ),
        ("perm.hjson", "{ stat: perm/1, step: 60000, m: 3 }"),
        (
            "strict.hjson",
            r#"{ stat: ent/1, step: 60000, bins: diff, min_n: 100000 }"#,
        ),
        (
            "rank.hjson",
            r#"{ stat: ent/1, step: 60000, bins: "rank:4" }"#,
        ),
        (
            "mi.hjson",
            r#"{ stat: mi/1, step: 60000, bins: "rank:4", lag: "-2..2", null: 10, seed: 3 }"#,
        ),
        (
            "jsd.hjson",
            r#"{ stat: jsd/1, step: 60000, bins: diff, ref: "1768550400000-1768554000000",
                 min_n: 50 }"#,
        ),
    ] {
        std::fs::write(dir.join(file), text).unwrap();
    }
    dir
}

/// The uids a run printed, in order.
fn uids(o: &Output) -> Vec<String> {
    out(o)
        .lines()
        .filter(|l| l.starts_with("x.tel/info"))
        .map(|l| l.split_whitespace().nth(1).unwrap().to_string())
        .collect()
}

fn info(dir: &Path, recipe: &str, extra: &[&str]) -> Output {
    let (cpu, latency) = (series("cpu"), series("latency"));
    let mut args = vec![
        "tel", "info", recipe, "--series", &cpu, "--series", &latency, "--during", DURING, "-s",
        "lib",
    ];
    args.extend_from_slice(extra);
    run(dir, &args)
}

#[test]
fn a_statistic_commits_and_a_second_library_gives_the_same_uids() {
    let commit = |name: &str| {
        let dir = library(name);
        let mut all = Vec::new();
        for r in ["ent.hjson", "perm.hjson"] {
            let o = info(&dir, r, &["--at", "1768564800000", "--yes"]);
            assert_eq!(o.status.code(), Some(0), "{}", err(&o));
            assert!(out(&o).contains("record(s) committed"), "{}", out(&o));
            all.extend(uids(&o));
        }
        all
    };
    let a = commit("first");
    assert_eq!(a.len(), 4, "two statistics over two series");
    assert_eq!(a, commit("second"));
}

#[test]
fn without_yes_it_prints_and_writes_nothing() {
    let dir = library("review");
    let before = std::fs::read_dir(dir.join("lib")).unwrap().count();
    let o = info(&dir, "ent.hjson", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let text = out(&o);
    assert!(
        text.contains("value ") && text.contains("Miller-Madow"),
        "{text}"
    );
    assert!(text.contains("nothing was written"), "{text}");
    assert_eq!(std::fs::read_dir(dir.join("lib")).unwrap().count(), before);
    assert!(!dir.join(".smysl/staged.smy").exists());
}

#[test]
fn a_recipe_not_admitted_is_usage_and_a_statistic_not_run_is_reported() {
    let dir = library("refusals");
    let o = info(&dir, "rank.hjson", &[]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(err(&o).contains("SMY-E702"), "{}", err(&o));

    let o = info(&dir, "strict.hjson", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("SMY-W703"), "{}", out(&o));
    assert!(out(&o).contains("0 unit(s), 2 refused"), "{}", out(&o));

    let o = run(
        &dir,
        &["tel", "info", "ent.hjson", "--during", DURING, "-s", "lib"],
    );
    assert_eq!(o.status.code(), Some(2), "no --series");
    let cpu = series("cpu");
    let o = run(
        &dir,
        &["tel", "info", "ent.hjson", "--series", &cpu, "-s", "lib"],
    );
    assert_eq!(o.status.code(), Some(2), "no --during");
    let o = info(&dir, "ent.hjson", &["--yes"]);
    assert_eq!(o.status.code(), Some(2), "--yes without --at");
}

#[test]
fn a_pair_reads_with_and_a_divergence_reads_its_reference() {
    let dir = library("pairs");
    let latency = series("latency");
    let o = info(&dir, "mi.hjson", &["--with", &latency]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    let text = out(&o);
    assert!(text.contains(r#" ~ {"attributes""#), "{text}");
    assert!(text.contains("surrogates at least as high"), "{text}");
    assert!(text.contains("by lag -2:"), "{text}");
    assert_eq!(uids(&o).len(), 2, "{text}");
    // Two units: cpu with latency, and latency with itself, grounded on one series.
    let o = info(
        &dir,
        "mi.hjson",
        &["--with", &latency, "--at", "1768564800000", "--yes"],
    );
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).contains("2 record(s) committed"), "{}", out(&o));

    let o = info(&dir, "jsd.hjson", &[]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert_eq!(uids(&o).len(), 2, "{}", out(&o));

    // --with belongs to mi/1, and mi/1 needs it.
    let o = info(&dir, "ent.hjson", &["--with", &latency]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    let o = info(&dir, "mi.hjson", &[]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(err(&o).contains("--with"), "{}", err(&o));
}

/// `tent/1` (TI-P3): the first run materialises the window's templates and commits them with
/// the statistic; the second reads them from the store and gives the same unit.
#[test]
fn a_template_entropy_commits_its_templates_and_then_reads_them() {
    let dir = scratch("tent");
    let j0 = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/journald/j0.json");
    let o = run(
        &dir,
        &[
            "tel",
            "read",
            "journald/1",
            j0.to_str().unwrap(),
            "--instrument",
            "journald:h1",
            "--at",
            "1768557700000",
            "-s",
            "lib",
            "--yes",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    std::fs::write(dir.join("tent.hjson"), "{ stat: tent/1, min_n: 5 }").unwrap();
    let cart = r#"journald:{"host":"h1","unit":"cart.service"}"#;
    let tent = |extra: &[&str]| {
        let mut args = vec![
            "tel",
            "info",
            "tent.hjson",
            "--series",
            cart,
            "--during",
            "1768557600000-1768557660000",
            "-s",
            "lib",
        ];
        args.extend_from_slice(extra);
        run(&dir, &args)
    };
    let first = tent(&["--at", "1768557700000", "--yes"]);
    assert_eq!(first.status.code(), Some(0), "{}", err(&first));
    let text = out(&first);
    assert!(text.contains("template-entropy"), "{text}");
    assert!(text.contains("template(s) materialised"), "{text}");
    assert_eq!(uids(&first).len(), 1, "{text}");
    let second = tent(&[]);
    assert_eq!(second.status.code(), Some(0), "{}", err(&second));
    assert!(!out(&second).contains("materialised"), "{}", out(&second));
    assert_eq!(uids(&first), uids(&second));
}
