//! The global-flag matrix.
//!
//! Twelve flags are declared once, globally, so **every** subcommand's `--help` advertises
//! all twelve. Almost none of them were wired. Measured before this test existed:
//! `--output` was honoured by 3 of 9 commands, `--json` by 1 of 6, `--strict` by 1 of 8.
//! A user reads `--json` in `smysl trace --help`, passes it, gets prose, and has no way to
//! learn the flag was never implemented.
//!
//! Fixing instances does not stop that recurring — the next flag added reaches every
//! subcommand's help the moment it is declared. So the matrix is asserted here: every pair
//! is either **honoured** or **explicitly refused**, and silence is a failure.
//!
//! `--json` is checked with a real parser rather than a pattern. The bug being guarded
//! against is machine-readable output a machine cannot read, so a guard that only looks for
//! a leading `{` would miss exactly the case that matters — `check --json` shipped emitting
//! Rust's `\u{1}` debug escape, which no JSON parser accepts.

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const F1: &str = "fixtures/corpus/F1-incident.smy";
const F3: &str = "fixtures/corpus/F3-narrative.smy";
const F6: &str = "fixtures/corpus/F6-adversarial.smy";
const F7: &str = "fixtures/corpus/F7-mixed-granularity.smy";
const ROOT: &str = "b3:js4xzessu5zwjpv2rawtugnuvj";
const GROUND: &str = "b3:cvhirtgs2mpvli2ethhyeo32uf";

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

fn run(args: &[&str]) -> Out {
    let o = Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary under test must run");
    Out {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        code: o.status.code().unwrap_or(-1),
    }
}

/// Every command that reports something must produce *parseable* JSON under `--json`.
///
/// Not "different output", not "starts with a brace" — parsed. Each entry names the
/// command and one key a caller would reach for, so a rename cannot silently pass.
#[test]
fn every_reporting_command_emits_parseable_json() {
    let cases: &[(&str, Vec<&str>, &str)] = &[
        ("check", vec!["check", "--json", F6], "code"),
        ("diff", vec!["diff", "--json", F1, F3], "only_in_a"),
        ("trace", vec!["trace", "--json", ROOT, F1], "nodes"),
        ("salience", vec!["salience", "--json", F1], "ranking"),
        (
            "view",
            vec!["view", "--json", "--id", "v/x", "--roots", ROOT, F1],
            "reachable",
        ),
        (
            "retract",
            vec!["retract", "--json", "--dry-run", GROUND, F1],
            "blast_radius",
        ),
    ];

    for (name, args, key) in cases {
        let out = run(args);
        assert!(
            !out.stdout.trim().is_empty(),
            "{name} --json produced nothing on stdout"
        );
        // `check` reports one object per diagnostic, so parse line by line and require
        // every line to be valid rather than only the first.
        let mut saw_key = false;
        for line in out.stdout.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| {
                panic!("{name} --json emitted a line no parser accepts: {e}\n  line: {line}")
            });
            if v.get(*key).is_some() {
                saw_key = true;
            }
        }
        assert!(
            saw_key,
            "{name} --json parsed but never carried `{key}`; a caller reaching for it \
             would get nothing"
        );
    }
}

/// The escaping regression, pinned directly.
///
/// A diagnostic message quotes document content, so an authored gist — or a model's output
/// through `ingest` — can put a control character into it. Rust's `{:?}` renders that as
/// `\u{1}`, which is not JSON. This shipped.
#[test]
fn a_control_character_in_a_diagnostic_stays_valid_json() {
    let dir = std::env::temp_dir().join("smysl-json-escape-test");
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("ctl.smy");
    // A reference containing U+0001, so the "malformed reference" message quotes it back.
    std::fs::write(
        &path,
        "@claim c/a { status: derived, grounds: [c/miss\u{1}x] }\n~ a gist.\n",
    )
    .expect("write fixture");

    let out = run(&["check", "--json", path.to_str().unwrap()]);
    let mut lines = 0;
    for line in out.stdout.lines().filter(|l| !l.trim().is_empty()) {
        lines += 1;
        serde_json::from_str::<serde_json::Value>(line)
            .unwrap_or_else(|e| panic!("a control character broke the JSON: {e}\n  line: {line}"));
    }
    assert!(lines > 0, "expected a diagnostic quoting the bad reference");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--output` is either honoured or refused out loud. Writing to stdout while accepting the
/// flag is what this forbids.
#[test]
fn every_command_either_honours_output_or_says_it_cannot() {
    let dir = std::env::temp_dir().join("smysl-output-matrix-test");
    std::fs::create_dir_all(&dir).expect("temp dir");

    // Commands that emit one artifact: the file must appear.
    let writes: &[(&str, Vec<&str>)] = &[
        ("fmt", vec!["fmt", F1]),
        ("merge", vec!["merge", F1]),
        ("pack", vec!["pack", "--budget", "2000", F1]),
        ("bundle", vec!["bundle", F1]),
        ("thread", vec!["thread", "--derive", "brief", F1]),
        ("render", vec!["render", "--target", "markdown", F1]),
    ];
    for (name, base) in writes {
        let dest = dir.join(format!("{name}.out"));
        let _ = std::fs::remove_file(&dest);
        let mut args = base.clone();
        args.insert(0, dest.to_str().unwrap());
        args.insert(0, "-o");
        let out = run(&args);
        let wrote = std::fs::metadata(&dest)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
        assert!(
            wrote,
            "{name} accepted --output and wrote nothing (exit {}, stderr: {})",
            out.code,
            out.stderr.trim()
        );
    }

    // Commands whose output is a report: they must say so, not ignore it.
    let refuses: &[(&str, Vec<&str>)] = &[
        ("salience", vec!["salience", F1]),
        ("view", vec!["view", "--id", "v/x", "--roots", ROOT, F1]),
        ("retract", vec!["retract", "--dry-run", GROUND, F1]),
    ];
    for (name, base) in refuses {
        let dest = dir.join(format!("{name}.out"));
        let mut args = base.clone();
        args.insert(0, dest.to_str().unwrap());
        args.insert(0, "-o");
        let out = run(&args);
        assert!(
            out.stderr.contains("--output is not honoured"),
            "{name} accepted --output silently; stderr was: {}",
            out.stderr.trim()
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--strict` promotes warnings to the failure threshold. `F7` and the forward-compat
/// fixture both warn without erroring, so a command that honours it changes exit code.
///
/// Only `check` implements it today. This pins that as the *known* state rather than
/// asserting the aspiration, so the day another command wires it up the test is what says
/// the matrix moved.
#[test]
fn strict_is_honoured_by_check_and_the_others_are_recorded_as_not() {
    let plain = run(&["check", F7]);
    let strict = run(&["check", "--strict", F7]);
    assert_eq!(plain.code, 0, "F7 warns without erroring");
    assert_eq!(
        strict.code, 3,
        "check --strict must promote a warning to the failure threshold"
    );

    // Documented gap, not an assertion that it is right: these accept `--strict` and do
    // nothing with it. Listed so the count cannot drift unnoticed.
    for cmd in [
        vec!["merge", F7],
        vec!["pack", "--budget", "2000", F7],
        vec!["bundle", F7],
    ] {
        let a = run(&cmd);
        let mut with = cmd.clone();
        with.push("--strict");
        let b = run(&with);
        assert_eq!(
            a.code, b.code,
            "{:?} now reacts to --strict; the matrix changed and this test should say so",
            cmd[0]
        );
    }
}

/// A budget that cannot be represented is refused, not wrapped.
///
/// `--budget Nk` multiplied by 1000 without checking. Debug builds panicked; release builds
/// **wrapped**, so `--budget 18446744073709552k` silently became 384 tokens — and
/// `--explain` then reported 384 as the budget. A budget that quietly becomes a different
/// budget is precisely the silent-degradation failure this project exists to prevent, and it
/// was worse in the build people ship.
#[test]
fn an_unrepresentable_budget_is_refused_rather_than_wrapped() {
    // u64::MAX / 1000 is 18446744073709551, so one above it overflows the multiply.
    let out = run(&["pack", "--budget", "18446744073709552k", F1]);
    assert_eq!(
        out.code,
        2,
        "an overflowing budget must be a usage error; stderr was: {}",
        out.stderr.trim()
    );
    assert!(
        out.stdout.trim().is_empty(),
        "a refused budget must not also emit a pack"
    );

    // The largest budget that *does* fit still works, so the bound is on overflow rather
    // than on being large.
    let ok = run(&["pack", "--budget", "18446744073709551k", F1]);
    assert_eq!(
        ok.code,
        0,
        "the largest representable budget must still pack; stderr: {}",
        ok.stderr.trim()
    );
}
