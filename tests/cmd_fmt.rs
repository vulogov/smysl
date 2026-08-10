//! `smysl fmt`, driven as a user drives it.
//!
//! Twelve mutants survived in `cmd_fmt` at 1.1 — the joint largest cluster in the CLI, tied
//! with `cmd_providers`. Every one is a branch that decides what the command *does*: whether
//! it refuses a flag, whether a warning raises the exit code, whether it writes at all.
//!
//! They survived because nothing reached them. `cmd_fmt` reads a filesystem, parses, writes,
//! and prints; there is no seam to call, so the only way in is to run the binary — which is
//! what `tests/global_flags.rs` does for the flag matrix and what this does for one command's
//! behaviour. `make doc-output` replays documented `fmt` invocations and catches a good deal,
//! but the manual documents what `fmt` does when it works, not what it refuses.
//!
//! Each test below names the decision it pins rather than the mutant, because the decision is
//! what a reader needs and the mutant is only how the gap was found.

// The binary only exists with `cli`, and a test that cannot find its subject is worse than one
// that is not there — the reason `global_flags.rs` compiles itself out the same way.
#![cfg(feature = "cli")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const F1: &str = "fixtures/corpus/F1-incident.smy";
/// Six comment lines, which canonical form cannot carry.
const F9: &str = "fixtures/corpus/F9-forward-compat.smy";

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Raw stdout, for the one step whose output is binary.
///
/// `run` decodes stdout with `from_utf8_lossy`, which is right for text and destroys CBOR —
/// every invalid sequence becomes U+FFFD, and the log no longer parses. The first version of
/// the test below piped `bundle` through it and got `SMY-E004: malformed envelope at byte 0`,
/// which is the encoder being blamed for the test harness.
fn run_bytes(args: &[&str]) -> (Vec<u8>, i32) {
    let o = Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary under test must run");
    (o.stdout, o.status.code().unwrap_or(-1))
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

/// A directory of this test's own, named for the process, so two runs cannot collide.
///
/// A shared path under `TMPDIR` failed in CI once already, in a test added in 0.13: it wrote
/// into a fixed directory and depended on the state of the machine rather than on the code.
///
/// Every test that needs a document with a *property* — not canonical, has comments — builds
/// it here rather than relying on a repository fixture having that property. The first version
/// of this file used `F1` for "not canonically formatted" and `F9` for "has comments", and
/// that was wrong in a way worth recording: `cargo-mutants` reuses build directories, so a
/// mutant that misroutes a write can leave a fixture rewritten, and every later mutant tested
/// in that directory then sees a canonical `F1`. These tests failed spuriously, cargo-mutants
/// counted each spurious failure as a *catch*, and 97 mutants appeared to die that had not.
/// A test must not depend on the state of the tree any more than on the state of the machine.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-fmt-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Exit codes, by the names the specification gives them.
const SUCCESS: i32 = 0;
const USAGE: i32 = 2;
const CHECK_ERRORS: i32 = 3;

// ---------------------------------------------------------------------------------------
// `--check` and `--write` are surface-only
// ---------------------------------------------------------------------------------------

/// A CBOR log is canonical by construction, so there is nothing for `--check` to compare and
/// `--write` would silently convert a binary store to text.
///
/// The guard is `if check || write`, and `||` mutated to `&&` lets each flag through alone —
/// which is every way a user would actually pass them.
#[test]
fn check_and_write_are_refused_on_a_cbor_log() {
    let dir = scratch("cbor");
    let cbor = dir.join("log.cbor");
    let (bundled, code) = run_bytes(&["bundle", F1]);
    assert_eq!(code, SUCCESS, "bundling the fixture");
    std::fs::write(&cbor, &bundled).expect("write cbor");
    let p = cbor.to_str().unwrap();

    for flag in ["--check", "--write"] {
        let out = run(&["fmt", flag, p]);
        assert_eq!(
            out.code, USAGE,
            "`fmt {flag}` on a CBOR log must be refused, got {}\n{}",
            out.code, out.stderr
        );
        assert!(
            out.stderr.contains("already canonical"),
            "the refusal must say why: {}",
            out.stderr
        );
    }

    // The control: without either flag the same input is fine, so the refusal is about the
    // flags and not about CBOR being unreadable.
    let plain = run(&["fmt", p]);
    assert_eq!(
        plain.code, SUCCESS,
        "`fmt` alone renders a CBOR log as surface: {}",
        plain.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `--strict` and warnings
// ---------------------------------------------------------------------------------------

/// Under `--strict` a document that parses with warnings still fails, because a gate asked to
/// be told about them.
///
/// Both halves matter and both had mutants: `strict && any(!is_error)` becoming `||` fires on
/// every document, and deleting the `!` fires only on errors — which the next branch already
/// handles, so it would look like nothing changed.
#[test]
fn strict_turns_a_parse_warning_into_a_failing_exit_code() {
    let dir = scratch("strict");
    let path = dir.join("unknown-kind.smy");
    // An unknown relation kind is `SMY-W013`: a warning, not an error. The document is
    // otherwise valid and formats cleanly.
    std::fs::write(
        &path,
        "@claim c/a { status: speculative }\n~ a first claim here\n\n\
         @claim c/b { status: speculative }\n~ a second claim here\n\n\
         @rel c/a --wobbles--> c/b\n",
    )
    .expect("write fixture");
    let p = path.to_str().unwrap();

    let lax = run(&["fmt", p]);
    assert_eq!(
        lax.code, SUCCESS,
        "a warning alone does not fail: {}",
        lax.stderr
    );
    assert!(
        lax.stderr.contains("SMY-W013"),
        "the warning is still reported"
    );

    let strict = run(&["--strict", "fmt", p]);
    assert_eq!(
        strict.code, CHECK_ERRORS,
        "--strict must fail on a warning: {}",
        strict.stderr
    );

    // The control: --strict on a document with no warnings must still succeed, or the flag
    // would just be "always fail" and the test above would prove nothing.
    let clean = run(&["--strict", "fmt", F1]);
    assert_eq!(
        clean.code, SUCCESS,
        "--strict on a clean document succeeds: {}",
        clean.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------------------

/// Comments are not part of any record, so canonical form cannot reproduce them. The manual
/// recommends `fmt --write` as a pre-commit habit, which makes silent deletion of a
/// reviewer's notes the difference between a formatter and a hazard.
///
/// `out.comments > 0` had three mutants — `<`, `==`, `>=` — and all three survived. `>=` is
/// the dangerous one: it warns about every document, including the ones with no comments,
/// which is how a warning becomes noise and stops being read.
#[test]
fn a_document_with_comments_says_they_will_not_survive() {
    let dir = scratch("comments");
    let body = "@claim c/a { status: speculative }\n~ a claim that stands alone\n";

    let commented = dir.join("commented.smy");
    std::fs::write(&commented, format!("# a reviewer's note\n{body}")).expect("write");
    let with = run(&["fmt", commented.to_str().unwrap()]);
    assert!(
        with.stderr
            .contains("comment line(s) are not part of any record"),
        "a document with comments must say so: {}",
        with.stderr
    );
    assert!(with.stderr.contains('1'), "and how many: {}", with.stderr);

    let plain = dir.join("plain.smy");
    std::fs::write(&plain, body).expect("write");
    let without = run(&["fmt", plain.to_str().unwrap()]);
    assert!(
        !without.stderr.contains("comment line(s)"),
        "a document with no comments must not warn: {}",
        without.stderr
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `--output`
// ---------------------------------------------------------------------------------------

/// `--output` names one destination, so it takes one document. Two would silently overwrite
/// each other, leaving whichever came last.
#[test]
fn output_takes_one_document_and_refuses_several() {
    let dir = scratch("output");
    let dest = dir.join("out.smy");
    let d = dest.to_str().unwrap();

    let many = run(&["--output", d, "fmt", F1, F9]);
    assert_eq!(
        many.code, USAGE,
        "two inputs and one --output is a usage error"
    );
    assert!(
        many.stderr.contains("--output takes one document"),
        "{}",
        many.stderr
    );

    // One input writes, which is the control: the refusal is about the count, not the flag.
    let one = run(&["--output", d, "fmt", F1]);
    assert_eq!(
        one.code, SUCCESS,
        "one input and --output writes: {}",
        one.stderr
    );
    let written = std::fs::read_to_string(&dest).expect("--output wrote the file");
    assert!(
        written.starts_with("@doc"),
        "and wrote a document: {written:.40}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `--check`
// ---------------------------------------------------------------------------------------

/// `--check` reports whether the file is already canonical, and says which file is not.
///
/// The comparison is `formatted != src`; inverted, `--check` passes on everything that is
/// wrong and fails on everything that is right — which would look like a working flag to
/// anybody whose files were already formatted.
#[test]
fn check_distinguishes_a_formatted_file_from_an_unformatted_one() {
    let dir = scratch("check");

    // Built here rather than taken from the corpus: this test needs a document that is *not*
    // canonical, and that is a property of the file, not of the repository.
    let loose = dir.join("loose.smy");
    std::fs::write(
        &loose,
        "@claim c/a {  status: speculative  }\n~ a claim with loose spacing\n\n\n\n\
         @claim c/b { status: speculative }\n~ another claim entirely\n",
    )
    .expect("write");
    let l = loose.to_str().unwrap();

    let unformatted = run(&["fmt", "--check", l]);
    assert_eq!(
        unformatted.code, CHECK_ERRORS,
        "an unformatted file must fail --check: {}",
        unformatted.stderr
    );
    assert!(unformatted.stderr.contains("not canonically formatted"));

    // Its own canonical form is, by definition.
    let canonical = dir.join("canonical.smy");
    let formatted = run(&["fmt", l]);
    assert_eq!(formatted.code, SUCCESS);
    std::fs::write(&canonical, formatted.stdout.as_bytes()).expect("write canonical");

    let ok = run(&["fmt", "--check", canonical.to_str().unwrap()]);
    assert_eq!(
        ok.code, SUCCESS,
        "a canonical file must pass --check: {}",
        ok.stderr
    );
    assert!(!ok.stderr.contains("not canonically formatted"));
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// `--write`
// ---------------------------------------------------------------------------------------

/// `--write` rewrites the file in place — unless the input was stdin, which has no file to
/// rewrite and whose path is the literal `-`.
///
/// `write && path != "-"` had a mutant on each half. `||` writes whenever the path is not
/// `-`, flag or no flag, which would rewrite a user's file on a plain `fmt`. `==` writes only
/// for stdin, creating a file actually named `-` in the working directory.
#[test]
fn write_rewrites_a_file_and_leaves_stdin_alone() {
    let dir = scratch("write");
    let target = dir.join("target.smy");
    // Written, not copied from the corpus: `--write` must be seen to *change* something, and
    // whether a shipped fixture is canonical is not this test's business.
    std::fs::write(
        &target,
        "@claim c/a {  status: speculative  }\n~ a claim with loose spacing\n\n\n\n\
         @claim c/b { status: speculative }\n~ another claim entirely\n",
    )
    .expect("write");
    let t = target.to_str().unwrap();
    let before = std::fs::read_to_string(&target).unwrap();

    let out = run(&["fmt", "--write", t]);
    assert_eq!(out.code, SUCCESS, "{}", out.stderr);
    let after = std::fs::read_to_string(&target).unwrap();
    assert_ne!(before, after, "--write must actually rewrite the file");
    assert_eq!(
        after,
        run(&["fmt", t]).stdout,
        "and what it wrote is what `fmt` prints"
    );

    // Idempotent: formatting an already-formatted file changes nothing further.
    let again = run(&["fmt", "--write", t]);
    assert_eq!(again.code, SUCCESS);
    assert_eq!(std::fs::read_to_string(&target).unwrap(), after);

    // The control for the other half of the guard: without `--write`, the file is untouched.
    let unwritten = dir.join("untouched.smy");
    std::fs::write(
        &unwritten,
        "@claim c/x {  status: speculative  }\n~ another loosely spaced claim\n",
    )
    .expect("write");
    let u = unwritten.to_str().unwrap();
    let original = std::fs::read_to_string(&unwritten).unwrap();
    assert_eq!(run(&["fmt", u]).code, SUCCESS);
    assert_eq!(
        std::fs::read_to_string(&unwritten).unwrap(),
        original,
        "`fmt` without --write must not modify its input"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------------------
// The round-trip guard, and why two mutants on it stay alive
// ---------------------------------------------------------------------------------------

// `cmd_fmt` re-parses what it formatted and refuses to emit anything whose records or labels
// moved. Two mutants sit on that guard — replacing it with `true`, and loosening its `&&` to
// `||` — and both survive this file.
//
// They are unreachable rather than untested. The guard fires only when the writer produces
// something the parser reads back differently, which is a defect in `write_surface`, not an
// input a user can supply. No document reaches it, so no test can distinguish a guard that
// checks from one that does not.
//
// Recorded here the way 0.13 recorded `worse`'s `>=`, 1.1 recorded `Lineage::is_empty -> false`
// and `Style::detect`'s `||`: a survivor no test can kill is a fact about the code. What *is*
// tested is the property the guard protects — `crates/smysl-core/tests/versioning.rs` asserts
// that a document round-trips declaring the version it declared, and `tests/surface.rs`
// asserts the records survive. If those ever fail, this guard is why `fmt` will refuse rather
// than write the damage to disk.
