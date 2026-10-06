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

// The whole file tests the *binary*, which the `[[bin]]` target only builds with the `cli`
// feature. `cargo test --workspace --no-default-features` — one of the seven configurations
// CI runs — therefore has no binary to test, and every case here failed with
// `NotFound`. Compiled out rather than skipped at runtime: a test that silently passes
// because it could not find its subject is worse than one that is not there.
#![cfg(feature = "cli")]

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
        ("find", vec!["find", "--json", "pool", F1], "hits"),
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
        ("find", vec!["find", "pool", F1]),
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

/// `--strict` promotes a warning to the failure threshold.
///
/// It was honoured by `check` alone while every subcommand advertised it, so a CI gate
/// running `merge --strict` believed it would fail on a warning and would not. Each command
/// below is paired with an input that warns without erroring, because a command with nothing
/// to warn about proves nothing either way.
#[test]
fn strict_promotes_a_warning_wherever_a_command_has_one() {
    // With `exact-pack` on, the `push` below is compiled out and nothing mutates this —
    // so the `mut` is unused in exactly one of the seven configurations CI builds.
    #[allow(unused_mut)]
    let mut cases: Vec<(&str, Vec<&str>)> = vec![("check", vec!["check", F7])];

    // `--mode exact` warns `SMY-W202` — "exact packing is not compiled in" — only on a build
    // *without* the feature. With `exact-pack` on, the search succeeds, reports "proven
    // optimal", and there is no warning for `--strict` to promote. Asserting one either way
    // made this fail under `--all-features`, which is a test that depends on how it was
    // built rather than on what the code does.
    #[cfg(not(feature = "exact-pack"))]
    cases.push((
        "pack",
        vec!["pack", "--budget", "200", "--mode", "exact", F1],
    ));

    let cases = &cases[..];
    for (name, base) in cases {
        let plain = run(base);
        assert_eq!(
            plain.code, 0,
            "{name}: the input must warn without erroring"
        );
        let mut strict = base.clone();
        strict.push("--strict");
        assert_eq!(
            run(&strict).code,
            3,
            "{name} --strict must promote its warning to the failure threshold"
        );
    }
}

/// The other half of the contract: `--strict` must not invent a failure where there is no
/// warning. A flag that fails clean input is worse than one that does nothing.
#[test]
fn strict_leaves_a_clean_run_alone() {
    for cmd in [
        vec!["check", F1],
        vec!["find", "pool", F1],
        vec!["merge", F1],
        vec!["pack", "--budget", "2000", F1],
        vec!["bundle", F1],
        vec!["thread", "--derive", "brief", F1],
    ] {
        let mut with = cmd.clone();
        with.push("--strict");
        let out = run(&with);
        assert_eq!(
            out.code,
            0,
            "{:?} --strict failed a clean store; stderr: {}",
            cmd[0],
            out.stderr.trim()
        );
    }
}

/// `--quiet` suppresses the line that says it worked, and nothing else.
///
/// It had only ever dimmed the progress bar, while its help promised to suppress non-error
/// output. Diagnostics and exit codes are deliberately untouched: a quiet run that also
/// swallowed its warnings would be a worse flag than one that did nothing.
#[test]
fn quiet_suppresses_the_summary_but_never_a_diagnostic() {
    let loud = run(&["check", F1]);
    let quiet = run(&["check", "--quiet", F1]);
    assert!(!loud.stdout.trim().is_empty(), "check prints a summary");
    assert!(
        quiet.stdout.trim().is_empty(),
        "--quiet left a summary behind: {}",
        quiet.stdout.trim()
    );

    // A warning still reaches the operator, and the verdict still reaches the script.
    let warned = run(&["check", "--quiet", F7]);
    assert!(
        warned.stderr.contains("SMY-W041"),
        "--quiet swallowed a diagnostic: {}",
        warned.stderr.trim()
    );
    assert_eq!(
        run(&["check", "--quiet", F6]).code,
        3,
        "--quiet hid a failure"
    );
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

/// `--format` is honoured or refused, for all twenty-six commands and both values.
///
/// H-17. It is global, so every command advertised it; three read it. The other twenty-three
/// accepted it and wrote whatever they were going to — so `bundle --format surface` put CBOR on
/// a terminal and `check --format cbor` printed prose to a caller who had asked for bytes. The
/// flag was documentation of a feature that was not there.
///
/// The expectation is written out rather than read from the binary, for the reason
/// `tests/dispatch.rs` gives at length: a list derived from the subject cannot fail. An empty
/// list means the command writes no document at all — a report, a store updated in place, or an
/// artifact with its own `--target`.
#[test]
fn every_command_honours_format_or_refuses_it() {
    // Each row: the command, the forms it can write, and the arguments clap requires. The
    // arguments matter because clap's own required-argument check runs before `main` is
    // reached, so a bare `smysl diff --format surface` never gets as far as the refusal.
    const FORMS: &[(&str, &[&str], &[&str])] = &[
        ("fmt", &["surface", "cbor"], &[F1]),
        ("check", &[], &[F1]),
        ("pack", &["surface", "cbor"], &["--budget", "2000", F1]),
        ("merge", &["surface", "cbor"], &[F1]),
        ("diff", &[], &[F1, F3]),
        ("trace", &[], &[ROOT, F1]),
        ("view", &[], &["--id", "v/x", "--roots", ROOT, F1]),
        ("bundle", &["surface", "cbor"], &[F1]),
        ("thread", &["surface", "cbor"], &["--derive", "brief", F1]),
        ("salience", &[], &[F1]),
        ("find", &[], &["pool", F1]),
        ("retract", &[], &["--dry-run", GROUND, F1]),
        ("withdraw", &[], &["x", F1]),
        ("resolve", &[], &["x", F1]),
        ("review", &[], &[F1]),
        (
            "commit",
            &[],
            &["--level", "drafted", "--as", "a/x", ROOT, F1],
        ),
        ("render", &[], &[F1]),
        // A store log, which has no surface spelling.
        ("import", &["cbor"], &["-"]),
        ("relink", &["cbor"], &[F1]),
        ("compact", &["cbor"], &[F1]),
        ("ingest", &[], &["--offline", "-"]),
        ("attest", &[], &["--offline", F1]),
        ("providers", &[], &[]),
        ("usage", &[], &[]),
        ("reindex", &[], &[F1]),
        ("ui", &[], &[F1]),
    ];
    assert_eq!(FORMS.len(), 26, "one row per command");

    // The refusal happens before the command runs, so an invocation that would fail for want of
    // arguments still reaches it. That is the point of checking it in one place.
    for (name, forms, args) in FORMS {
        for form in ["surface", "cbor"] {
            let mut argv = vec![*name];
            argv.extend_from_slice(args);
            argv.push("--format");
            argv.push(form);
            let out = run(&argv);
            if forms.contains(&form) {
                // The refusal's own wording, not the flag name: clap's missing-argument
                // message repeats the whole usage line, `--format` included, for the several
                // commands here that need an argument this invocation does not supply.
                assert!(
                    !out.stderr.contains("is not honoured here")
                        && !out.stderr.contains("is not available"),
                    "{name} refuses --format {form}, which it can produce: {}",
                    out.stderr.trim()
                );
            } else {
                assert_eq!(
                    out.code,
                    2,
                    "{name} --format {form} must be a usage error; stderr: {}",
                    out.stderr.trim()
                );
                assert!(
                    out.stderr.contains("is not honoured here")
                        || out.stderr.contains("is not available"),
                    "{name} --format {form} was refused for some other reason: {}",
                    out.stderr.trim()
                );
            }
        }
    }
}

/// And the honoured values produce the form they name, rather than the other one.
///
/// The half of H-17 that a refusal cannot cover: `bundle --format surface` was *accepted* and
/// wrote CBOR. A surface document starts `@doc`; a CBOR sequence does not.
#[test]
fn an_honoured_format_produces_that_form() {
    let dir = std::env::temp_dir().join(format!("smysl-format-matrix-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let cases: &[(&str, Vec<&str>)] = &[
        ("fmt", vec!["fmt", F1]),
        ("merge", vec!["merge", F1]),
        ("pack", vec!["pack", "--budget", "2000", F1]),
        ("bundle", vec!["bundle", F1]),
        ("thread", vec!["thread", "--derive", "brief", F1]),
    ];
    for (name, base) in cases {
        for form in ["surface", "cbor"] {
            let dest = dir.join(format!("{name}.{form}"));
            let mut args = base.clone();
            args.insert(0, dest.to_str().unwrap());
            args.insert(0, "-o");
            args.push("--format");
            args.push(form);
            let out = run(&args);
            let bytes = std::fs::read(&dest).unwrap_or_else(|e| {
                panic!(
                    "{name} --format {form} wrote nothing ({e}); exit {}, stderr: {}",
                    out.code,
                    out.stderr.trim()
                )
            });
            let is_surface = bytes.starts_with(b"@doc");
            assert_eq!(
                is_surface,
                form == "surface",
                "{name} --format {form} wrote the other form (first bytes: {:?})",
                &bytes[..bytes.len().min(8)]
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `fmt --format cbor` converts; `--check` and `--write` format in place. They do not combine.
#[test]
fn fmt_refuses_a_conversion_that_pretends_to_be_a_formatting() {
    for flag in ["--check", "--write"] {
        let out = run(&["fmt", "--format", "cbor", flag, F1]);
        assert_eq!(
            out.code,
            2,
            "fmt --format cbor {flag} must be refused; stderr: {}",
            out.stderr.trim()
        );
        assert!(
            out.stderr.contains("converts a document"),
            "the refusal must say why: {}",
            out.stderr.trim()
        );
    }
}

/// `--seed-check` asserts rule D, and the assertion is checked.
///
/// H-19. It was declared global, advertised on all twenty-six commands, and read by nothing: a
/// caller asserting reproducibility had the assertion accepted and never checked, which is the
/// worst possible shape for a flag whose only job is to be a check.
///
/// The table's label is not the answer on its own. `find` is lexical unless asked otherwise, so
/// a `mixed` command is pure in most of its invocations and refusing all of them would make the
/// flag useless in the cases it exists for. Each row below is a pair: the same command, once in
/// an invocation that is reproducible and once in one that is not.
#[test]
fn seed_check_allows_a_pure_invocation_and_refuses_the_rest() {
    // Reproducible: these must behave exactly as without the flag.
    let allowed: &[(&str, Vec<&str>)] = &[
        ("check", vec!["check", F1]),
        ("find lexical", vec!["find", "pool", F1]),
        (
            "find --engine lexical",
            vec!["find", "--engine", "lexical", "pool", F1],
        ),
        ("pack", vec!["pack", "--budget", "2000", F1]),
        // `pack` reads `--engine` only inside its `--query` branch, so an engine with no query
        // cannot reach an embedding. Asserted, because refusing it would be a false negative
        // and the kind that trains people to drop the flag.
        (
            "pack --engine semantic without --query",
            vec!["pack", "--budget", "2000", "--engine", "semantic", F1],
        ),
        // Derivation consults no model. `thread` was labelled mixed for `--refine`, which does
        // not exist as a flag, so every invocation of it is pure.
        ("thread --derive", vec!["thread", "--derive", "brief", F1]),
    ];
    for (what, base) in allowed {
        let plain = run(base);
        let mut checked = base.clone();
        checked.push("--seed-check");
        let out = run(&checked);
        assert_eq!(
            out.code,
            plain.code,
            "{what}: --seed-check changed the outcome of a reproducible invocation; stderr: {}",
            out.stderr.trim()
        );
        assert!(
            !out.stderr.contains("--seed-check"),
            "{what}: a reproducible invocation must not be lectured: {}",
            out.stderr.trim()
        );
    }

    // Not reproducible: refused before anything runs.
    let refused: &[(&str, Vec<&str>)] = &[
        (
            "find --engine semantic",
            vec!["find", "--engine", "semantic", "pool", F1],
        ),
        (
            "find --engine hybrid",
            vec!["find", "--engine", "hybrid", "pool", F1],
        ),
        (
            "pack --query --engine semantic",
            vec![
                "pack", "--budget", "2000", "--query", "pool", "--engine", "semantic", F1,
            ],
        ),
        ("ingest", vec!["ingest", "--offline", "-"]),
        ("attest", vec!["attest", "--offline", F1]),
    ];
    for (what, base) in refused {
        let mut checked = base.clone();
        checked.push("--seed-check");
        let out = run(&checked);
        assert_eq!(
            out.code,
            2,
            "{what}: --seed-check must refuse an invocation that is not reproducible; \
             stderr: {}",
            out.stderr.trim()
        );
        assert!(
            out.stderr.contains("rule D"),
            "{what}: the refusal must name the rule it is asserting: {}",
            out.stderr.trim()
        );
        assert!(
            out.stdout.trim().is_empty(),
            "{what}: a refused invocation must not also produce output"
        );
    }
}

/// The refusal says which invocations are the impure ones, not just that some are.
///
/// `mixed` alone sends a reader to the source. The `impure_when` text H-20 added to the help
/// line is the same text, so the two cannot drift.
#[test]
fn seed_check_names_the_flag_that_broke_reproducibility() {
    let out = run(&["--seed-check", "find", "--engine", "semantic", "pool", F1]);
    assert!(
        out.stderr.contains("--engine semantic|hybrid"),
        "the refusal must name the impure invocations: {}",
        out.stderr.trim()
    );
    let model = run(&["--seed-check", "ingest", "--offline", "-"]);
    assert!(
        model.stderr.contains("model-dependent"),
        "a model-dependent command has no exception to name: {}",
        model.stderr.trim()
    );
}
