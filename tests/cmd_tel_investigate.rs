//! `smysl tel investigate` (RFC SMYSL-3 §11, §23, TL-P6 step 5), against the binary: an
//! investigation from opening to closing, over windows of the Prometheus fixture.
//!
//! - Hypotheses that explain one window are rival explanations (kind 6), in `show` and in
//!   `tel check`; one whose numeric quote is wrong is held at `speculative` (`SMY-W606`).
//! - Evidence consulted and cited by no hypothesis is **unused**; a window about the targeted
//!   series that nobody consulted is **unexplored** (§11.2).
//! - Closing supersedes the open investigation.
//! - Who and when are required.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const SERIES: &str =
    r#"prom:http_requests_total{code="200",instance="h:9100",job="node",method="get"}"#;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-inv-{}-{name}", std::process::id()));
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

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{args:?}: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    out(&o)
}

/// The uid on the first line naming `ty`.
fn uid_of(text: &str, ty: &str) -> String {
    text.lines()
        .find(|l| l.starts_with(ty))
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or_else(|| panic!("no {ty} in {text}"))
        .to_string()
}

fn t(ms: u64) -> String {
    (T0 + ms).to_string()
}

/// Four scrapes, and two committed windows: [0, 30 s) and [30 s, 60 s).
fn library(name: &str) -> (PathBuf, String, String, String) {
    let dir = scratch(name);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/prom");
    for n in 0..4u64 {
        let f = root.join(format!("s{n}.prom")).display().to_string();
        ok(
            &dir,
            &[
                "tel",
                "read",
                "prom/1",
                &f,
                "--job",
                "node",
                "--instance",
                "h:9100",
                "--at",
                &t(15_000 * n),
                "-s",
                "lib",
                "--yes",
            ],
        );
    }
    let w = |a: u64, b: u64| {
        uid_of(
            &ok(
                &dir,
                &[
                    "tel",
                    "window",
                    SERIES,
                    &t(a),
                    &t(b),
                    "-s",
                    "lib",
                    "--at",
                    &t(b),
                    "--yes",
                ],
            ),
            "window",
        )
    };
    let (w1, w2) = (w(0, 30_000), w(30_000, 60_000));
    let at = |n: &str| uid_of(&ok(&dir, &["tel", "at", SERIES, &t(0), "-s", "lib"]), n);
    let series = at("series");
    (dir, w1, w2, series)
}

fn inv(dir: &Path, args: &[&str], who: &str, at: u64) -> String {
    let mut a = vec!["tel", "investigate"];
    a.extend_from_slice(args);
    let at = t(at);
    a.extend(["-s", "lib", "--as", who, "--at", &at]);
    ok(dir, &a)
}

#[test]
fn an_investigation_from_opening_to_closing() {
    let (dir, w1, w2, series) = library("flow");
    let i = uid_of(
        &inv(
            &dir,
            &["open", "why did requests drop?", "--rotation", "sre-eu"],
            "human:alice",
            70_000,
        ),
        "x.tel/investigation",
    );
    // Alice looks at the first window and the second.
    inv(&dir, &["consult", &i, &w1, &w2], "human:alice", 71_000);
    let max = format!("{w2}#max=40");
    let h1 = uid_of(
        &inv(
            &dir,
            &[
                "hypothesis",
                &i,
                "the counter reset: a restart",
                "--explains",
                &w2,
                "--ground",
                &w2,
                "--quote",
                &max,
                "--target",
                &series,
            ],
            "human:alice",
            72_000,
        ),
        "x.tel/hypothesis",
    );
    let wrong = format!("{w2}#max=4000");
    let h2 = uid_of(
        &inv(
            &dir,
            &[
                "hypothesis",
                &i,
                "traffic moved",
                "--explains",
                &w2,
                "--ground",
                &w2,
                "--quote",
                &wrong,
            ],
            "model:qwen3-8b",
            73_000,
        ),
        "x.tel/hypothesis",
    );
    let h3 = uid_of(
        &inv(
            &dir,
            &[
                "hypothesis",
                &i,
                "a deploy",
                "--explains",
                &w2,
                "--ground",
                &w2,
            ],
            "human:bob",
            74_000,
        ),
        "x.tel/hypothesis",
    );
    inv(
        &dir,
        &["probe", &i, "kubectl get pods -n shop"],
        "tool:kubectl",
        75_000,
    );
    inv(
        &dir,
        &[
            "action",
            &i,
            "kubectl rollout restart deploy/api",
            "--confirmed-by",
            "alice",
        ],
        "tool:kubectl",
        76_000,
    );

    let show = ok(&dir, &["tel", "investigate", "show", &i, "-s", "lib"]);
    assert!(show.contains("why did requests drop?"), "{show}");
    assert!(show.contains("hypotheses    3"), "{show}");
    assert!(show.contains(&format!("{h1}  inferred")), "{show}");
    assert!(show.contains(&format!("{h2}  speculative")), "{show}");
    assert!(show.contains("SMY-W606"), "{show}");
    assert!(show.contains(&format!("{h3}  inferred")), "{show}");
    assert!(show.contains("probes        1"), "{show}");
    assert!(show.contains("actions       1"), "{show}");
    // w1 was looked at and cited by nothing.
    assert!(show.contains(&format!("unused        1  {w1}")), "{show}");
    // Rivals: h1 and h3 stand; h2 is speculative and does not.
    assert!(show.contains("rivals        1"), "{show}");

    let check = run(&dir, &["tel", "check", "-s", "lib"]);
    assert_eq!(check.status.code(), Some(0));
    let check = out(&check);
    assert!(check.contains("rival explanations of"), "{check}");
    assert!(
        check.contains(&h1)
            && check.contains(&h3)
            && !check
                .lines()
                .any(|l| l.starts_with("contention") && l.contains(&h2)),
        "{check}"
    );

    inv(&dir, &["close", &i], "human:alice", 90_000);
    let show = ok(&dir, &["tel", "investigate", "show", &i, "-s", "lib"]);
    assert!(show.contains(&format!("closed {}", t(90_000))), "{show}");
}

#[test]
fn a_window_about_the_target_that_nobody_consulted_is_unexplored() {
    let (dir, w1, w2, series) = library("unexplored");
    let i = uid_of(
        &inv(&dir, &["open", "q"], "human:alice", 70_000),
        "x.tel/investigation",
    );
    inv(&dir, &["consult", &i, &w2], "human:alice", 71_000);
    inv(
        &dir,
        &[
            "hypothesis",
            &i,
            "it dropped",
            "--explains",
            &w2,
            "--ground",
            &w2,
            "--target",
            &series,
        ],
        "human:alice",
        72_000,
    );
    let show = ok(&dir, &["tel", "investigate", "show", &i, "-s", "lib"]);
    assert!(show.contains("unused        0"), "{show}");
    assert!(show.contains(&format!("unexplored    1  {w1}")), "{show}");
    // A lookback that ends before w1 began finds nothing.
    let show = ok(
        &dir,
        &[
            "tel",
            "investigate",
            "show",
            &i,
            "-s",
            "lib",
            "--lookback",
            "1000",
        ],
    );
    assert!(show.contains("unexplored    0"), "{show}");
}

#[test]
fn who_and_when_are_required() {
    let dir = scratch("usage");
    for args in [
        vec!["tel", "investigate", "open", "q", "-s", "lib"],
        vec![
            "tel",
            "investigate",
            "open",
            "q",
            "-s",
            "lib",
            "--as",
            "alice",
            "--at",
            "1",
        ],
        vec![
            "tel",
            "investigate",
            "consult",
            "-s",
            "lib",
            "--as",
            "human:a",
            "--at",
            "1",
        ],
        vec![
            "tel",
            "investigate",
            "hypothesis",
            "b3:x",
            "c",
            "-s",
            "lib",
            "--as",
            "human:a",
            "--at",
            "1",
        ],
        vec![
            "tel",
            "investigate",
            "frobnicate",
            "-s",
            "lib",
            "--as",
            "human:a",
            "--at",
            "1",
        ],
        vec!["tel", "investigate", "show", "-s", "lib"],
    ] {
        assert_eq!(run(&dir, &args).status.code(), Some(2), "{args:?}");
    }
}

/// Open questions (§22.2): a window the investigation used whose samples were evicted is a gap,
/// `SMY-W605`, and its hypothesis keeps its status. Reading the scrapes again closes the gap.
#[test]
fn an_evicted_window_it_used_is_a_gap() {
    let (dir, w1, w2, series) = library("gaps");
    let i = uid_of(
        &inv(&dir, &["open", "q"], "human:alice", 70_000),
        "x.tel/investigation",
    );
    inv(&dir, &["consult", &i, &w1], "human:alice", 71_000);
    inv(
        &dir,
        &[
            "hypothesis",
            &i,
            "it dropped",
            "--explains",
            &w2,
            "--ground",
            &w2,
            "--target",
            &series,
        ],
        "human:alice",
        72_000,
    );
    let show = |dir: &Path| ok(dir, &["tel", "investigate", "show", &i, "-s", "lib"]);
    let before = show(&dir);
    assert!(before.contains("gaps          0"), "{before}");
    let status = |s: &str| {
        s.lines()
            .find(|l| l.contains("it dropped"))
            .map(str::to_string)
    };

    let day_later = t(86_400_000);
    ok(
        &dir,
        &[
            "tel",
            "evict",
            "--older-than",
            "1h",
            "--at",
            &day_later,
            "-s",
            "lib",
            "--yes",
        ],
    );
    let after = show(&dir);
    assert!(
        after.contains("gaps          2"),
        "consulted and cited: {after}"
    );
    assert_eq!(
        after.lines().filter(|l| l.contains("SMY-W605")).count(),
        2,
        "{after}"
    );
    assert_eq!(
        status(&after),
        status(&before),
        "the hypothesis keeps its status"
    );

    // The scrapes again: the samples come back as a set.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/prom");
    for n in 0..4u64 {
        let f = root.join(format!("s{n}.prom")).display().to_string();
        ok(
            &dir,
            &[
                "tel",
                "read",
                "prom/1",
                &f,
                "--job",
                "node",
                "--instance",
                "h:9100",
                "--at",
                &t(15_000 * n),
                "-s",
                "lib",
                "--yes",
            ],
        );
    }
    assert!(show(&dir).contains("gaps          0"));
}
