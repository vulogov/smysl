//! `smysl merge`, driven as a user drives it.
//!
//! Eleven mutants survived in `cmd_merge` at 1.1, the second-largest cluster in the CLI after
//! `cmd_fmt`. They fall into four decisions, and none of them is about *merging* — the
//! semilattice itself is well covered inside `smysl-graph`. They are about what the command
//! does with the result: whether a warning reaches the exit code, whether a staged batch is
//! committed, which form is written, and what the surface form admits it left behind.
//!
//! Those live between `clap` and the library, so there is no seam to call; the only way in is
//! to run the binary, as `tests/cmd_fmt.rs` does for `fmt`.
//!
//! Writing them turned up a defect the mutants had only pointed at. The dropped-record count
//! treated a label binding as unrepresentable, so `@claim c/a` warned that one record "has no
//! surface form" over output that read `@claim c/a`. The comment above that filter records the
//! *same* mistake being fixed once already, for the `@doc` header. The count is now asked of
//! the writer rather than assumed, and the test below that pins it is
//! `a_document_that_renders_whole_warns_about_nothing`.

// The binary only exists with `cli`, and a test that cannot find its subject is worse than one
// that is not there — the same reason `cmd_fmt.rs` compiles itself out this way.
//
// `--staged` needs a second feature on top of that: without `ingest` the flag is answered with
// "this build has no ingest layer" and exits 2. The three tests that use it carry their own
// `#[cfg(feature = "ingest")]` rather than gating the whole file, because the other four say
// nothing about staging and are worth running in a `cli`-only build. `make test-matrix` builds
// exactly that combination, which is how this was found.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

/// Exit codes, by the names the specification gives them.
const SUCCESS: i32 = 0;
const CHECK_ERRORS: i32 = 3;

struct Out {
    stdout: Vec<u8>,
    stderr: String,
    code: i32,
}

impl Out {
    /// Stdout as text. Only for the runs that asked for `--format surface`; the default
    /// output is CBOR, and `from_utf8_lossy` turns that into replacement characters.
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// Run the binary from `dir`, because `--staged` resolves `.smysl/staged.smy` against the
/// working directory and nothing else names it.
fn run_in(dir: &Path, args: &[&str]) -> Out {
    let o = Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary under test must run");
    Out {
        stdout: o.stdout,
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        code: o.status.code().unwrap_or(-1),
    }
}

/// A directory of this test's own, named for the process, so two runs cannot collide.
///
/// Every document below is written here rather than taken from `fixtures/`, and that is not
/// tidiness. `cmd_fmt.rs` records what happens otherwise: `cargo-mutants` reuses build
/// directories, a mutant that misroutes a write leaves a fixture altered, later tests fail
/// spuriously, and cargo-mutants scores each spurious failure as a *catch* — 97 of them, once.
/// These tests need documents with specific properties (a label collision, exactly one view,
/// a retraction inside the staged batch), and building each one is also the clearest statement
/// of which property the test depends on.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-merge-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, body).expect("write document");
    p
}

/// Two documents naming one label `c/x` over different gists, which is a `label-collision`.
fn colliding(dir: &Path) -> (PathBuf, PathBuf) {
    (
        write(
            dir,
            "first.smy",
            "@claim c/x { status: speculative }\n~ the first account of the outage\n",
        ),
        write(
            dir,
            "second.smy",
            "@claim c/x { status: speculative }\n~ a different account of the same outage\n",
        ),
    )
}

// ---------------------------------------------------------------------------------------
// `--strict` and the two places a warning can come from
// ---------------------------------------------------------------------------------------

/// `strict_failed |= …` accumulates; `&=` would let a later clean input clear an earlier
/// warning. With the flag starting `false`, `&=` never sets it at all, so one warning is
/// enough to tell them apart.
///
/// The warning here is `SMY-W055`: a contention cap of zero, with a collision to exceed it.
/// The pairing matters as much as the assertion — the same merge without `--strict` must
/// still exit 0, or the test would pass over a build that failed on every warning.
#[test]
fn a_warning_from_an_input_raises_the_exit_code_only_under_strict() {
    let dir = scratch("strict-input");
    let (a, b) = colliding(&dir);
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());

    let strict = run_in(
        &dir,
        &[
            "merge",
            "--strict",
            "--max-contentions-per-agent",
            "0",
            a,
            b,
        ],
    );
    assert_eq!(
        strict.code, CHECK_ERRORS,
        "--strict must turn the W055 warning into an exit code; stderr was {}",
        strict.stderr
    );
    assert!(
        strict.stderr.contains("SMY-W055"),
        "the warning that raised it should be named: {}",
        strict.stderr
    );

    let lax = run_in(&dir, &["merge", "--max-contentions-per-agent", "0", a, b]);
    assert_eq!(
        lax.code, SUCCESS,
        "the same warning without --strict is a warning; stderr was {}",
        lax.stderr
    );
}

/// The staged merge has its own `|=`, and an input that warns would mask a mutation of it.
///
/// So the staged batch is the *only* thing that can warn here: the input is a lone claim
/// grounded on nothing, and the batch carries a retraction of a unit it also supplies, which
/// `--retraction advisory` reports as `SMY-W052`. The control run — the same input, the same
/// flags, no `--staged` — must exit 0, which is what shows the warning came from the batch.
#[test]
#[cfg(feature = "ingest")]
fn a_warning_from_the_staged_batch_alone_raises_the_exit_code() {
    let dir = scratch("strict-staged");
    std::fs::create_dir_all(dir.join(".smysl")).expect("project dir");
    let input = write(
        &dir,
        "store.smy",
        "@claim c/a { status: speculative }\n~ a claim the staged batch does not touch\n",
    );
    let input = input.to_str().unwrap();
    write(
        &dir.join(".smysl"),
        "staged.smy",
        "@claim c/doomed { status: speculative }\n\
         ~ a claim that this same batch withdraws\n\
         \n\
         @claim c/reason { status: speculative }\n\
         ~ the reason it was withdrawn\n\
         \n\
         @rel c/reason --retracts--> c/doomed\n",
    );

    let staged = run_in(
        &dir,
        &[
            "merge",
            "--strict",
            "--retraction",
            "advisory",
            "--staged",
            input,
        ],
    );
    assert_eq!(
        staged.code, CHECK_ERRORS,
        "a warning raised by the staged merge must reach the exit code; stderr was {}",
        staged.stderr
    );
    assert!(
        staged.stderr.contains("SMY-W052"),
        "the advisory retraction should be reported against the staged batch: {}",
        staged.stderr
    );

    let without = run_in(
        &dir,
        &["merge", "--strict", "--retraction", "advisory", input],
    );
    assert_eq!(
        without.code, SUCCESS,
        "without --staged there is nothing to warn about, which is what makes the run above \
         attributable; stderr was {}",
        without.stderr
    );
}

// ---------------------------------------------------------------------------------------
// `--staged`, and what `--quiet` does and does not silence
// ---------------------------------------------------------------------------------------

/// `if !staged_records.is_empty()` guards the second merge. Delete the `!` and the command
/// merges an empty store and announces "committed 0 staged record(s)" on every run.
///
/// The control is the whole test: what fails under the mutation is the run that passed no
/// `--staged` and must therefore say nothing about staging.
#[test]
#[cfg(feature = "ingest")]
fn a_staged_batch_is_committed_only_when_it_is_asked_for() {
    let dir = scratch("staged");
    std::fs::create_dir_all(dir.join(".smysl")).expect("project dir");
    let input = write(
        &dir,
        "store.smy",
        "@claim c/a { status: speculative }\n~ a claim already in the store\n",
    );
    let input = input.to_str().unwrap();
    write(
        &dir.join(".smysl"),
        "staged.smy",
        "@claim c/s { status: speculative }\n~ a claim waiting to be committed\n",
    );

    let asked = run_in(&dir, &["merge", "--staged", input]);
    assert_eq!(asked.code, SUCCESS, "stderr: {}", asked.stderr);
    assert!(
        asked.stderr.contains("committed 2 staged record(s)"),
        "the claim and its label binding are both committed: {}",
        asked.stderr
    );

    let unasked = run_in(&dir, &["merge", input]);
    assert_eq!(unasked.code, SUCCESS, "stderr: {}", unasked.stderr);
    assert!(
        !unasked.stderr.contains("committed"),
        "a run that did not ask for the staged batch must not commit or mention one: {}",
        unasked.stderr
    );
    assert!(
        unasked.stdout.len() < asked.stdout.len(),
        "and the staged records must be absent from the output, not merely unannounced"
    );
}

/// `--quiet` suppresses the commit line. It must not suppress the commit.
///
/// `if !global.get_flag("quiet")` guards only the `eprintln!`, and deleting the `!` inverts
/// which run is silent — so both runs are checked, and both are checked for the records as
/// well as for the message. A quiet flag that dropped the batch would be a data-loss bug
/// wearing the costume of an output-formatting one.
#[test]
#[cfg(feature = "ingest")]
fn quiet_silences_the_staged_commit_line_and_not_the_commit() {
    let dir = scratch("staged-quiet");
    std::fs::create_dir_all(dir.join(".smysl")).expect("project dir");
    let input = write(
        &dir,
        "store.smy",
        "@claim c/a { status: speculative }\n~ a claim already in the store\n",
    );
    let input = input.to_str().unwrap();
    write(
        &dir.join(".smysl"),
        "staged.smy",
        "@claim c/s { status: speculative }\n~ a claim waiting to be committed\n",
    );

    let loud = run_in(&dir, &["merge", "--staged", input]);
    let quiet = run_in(&dir, &["merge", "--staged", "--quiet", input]);

    assert!(
        loud.stderr.contains("committed 2 staged record(s)"),
        "without --quiet the commit is announced: {}",
        loud.stderr
    );
    assert!(
        !quiet.stderr.contains("committed"),
        "--quiet suppresses the announcement: {}",
        quiet.stderr
    );
    assert_eq!(
        loud.stdout, quiet.stdout,
        "--quiet changes what is said, never what is written"
    );
}

// ---------------------------------------------------------------------------------------
// `--format`
// ---------------------------------------------------------------------------------------

/// `f == "surface"` decides the output form. Inverted, `--format surface` writes CBOR and the
/// default writes text — so neither run alone distinguishes the two, and both are here.
#[test]
fn the_format_flag_chooses_the_form_it_names() {
    let dir = scratch("format");
    let input = write(
        &dir,
        "store.smy",
        "@claim c/a { status: speculative }\n~ a claim to render in one form or the other\n",
    );
    let input = input.to_str().unwrap();

    let surface = run_in(&dir, &["merge", "--format", "surface", input]);
    assert_eq!(surface.code, SUCCESS, "stderr: {}", surface.stderr);
    assert!(
        surface.text().starts_with("@claim c/a"),
        "--format surface writes the text form: {:?}",
        surface.text()
    );

    // Stdout is not a terminal under a test harness, so the default is CBOR (rule P). A CBOR
    // log is a definite-length array of two, which is `0x82` — text cannot begin with that.
    let default = run_in(&dir, &["merge", input]);
    assert_eq!(default.code, SUCCESS, "stderr: {}", default.stderr);
    assert_eq!(
        default.stdout.first(),
        Some(&0x82),
        "the default on a non-TTY stdout is a CBOR log"
    );
}

// ---------------------------------------------------------------------------------------
// What the surface form admits it left behind
// ---------------------------------------------------------------------------------------

/// A document whose every record has a surface spelling must produce no warning at all.
///
/// This is the test that found the defect. `@claim c/a` counted its own label binding as
/// unrepresentable and warned about it, over output that named the label — the same mistake
/// the `@doc` header had already been fixed for, made again one record type along.
///
/// It is also where four of the eleven mutants die, because every one of them makes something
/// representable look dropped: deleting the unit arm counts the claim, deleting the view arm
/// counts the `@doc` header, flipping `!=` counts the header that *was* emitted, and `>= 0`
/// warns whatever the count is. An empty stderr refutes all four at once, which is why the
/// assertion is on the whole stream rather than on the absence of one phrase.
#[test]
fn a_document_that_renders_whole_warns_about_nothing() {
    let dir = scratch("renders-whole");
    let input = write(
        &dir,
        "store.smy",
        "@doc smysl/1.0 {\n\
         \x20 id: v/t\n\
         \x20 intent: a-document-with-one-view\n\
         \x20 lang: en\n\
         }\n\
         \n\
         @claim c/a { status: speculative }\n\
         ~ one claim, one label, one view header\n",
    );
    let input = input.to_str().unwrap();

    let out = run_in(&dir, &["merge", "--format", "surface", input]);
    assert_eq!(out.code, SUCCESS, "stderr: {}", out.stderr);
    assert_eq!(
        out.stderr, "",
        "a view, a unit and a label binding all have surface spellings, so there is nothing \
         to warn about"
    );
    // The control on the assertion above: silence is only meaningful if the three record
    // types were really there and really came back.
    let text = out.text();
    assert!(
        text.contains("@doc smysl/1.0"),
        "the view came back: {text}"
    );
    assert!(text.contains("@claim c/a"), "the label came back: {text}");
}

/// A name the grammar has nowhere to put *is* dropped, and is counted exactly once.
///
/// Two inputs name `c/x` over different gists. One binding is written as the label on its
/// unit; the other has no place to go — surface syntax writes a label as part of a unit
/// declaration and the losing unit's own name is already taken. That is a real omission and
/// the CBOR output is where it survives, which is what the message says.
///
/// The count is asserted exactly. `1` rather than "warns at all" is what separates a correct
/// filter from one that also counts the two units, and it is the assertion the defect above
/// would have failed.
#[test]
fn a_name_the_grammar_cannot_place_is_counted_exactly_once() {
    let dir = scratch("dropped-name");
    let (a, b) = colliding(&dir);
    let (a, b) = (a.to_str().unwrap(), b.to_str().unwrap());

    let out = run_in(&dir, &["merge", "--format", "surface", a, b]);
    assert_eq!(out.code, SUCCESS, "stderr: {}", out.stderr);
    assert!(
        out.stderr.contains("1 record(s) have no surface form"),
        "exactly the losing label binding is dropped, and nothing else: {}",
        out.stderr
    );
    assert!(
        out.stderr
            .contains("the default CBOR output preserves them"),
        "the warning has to say where the record survives, or it is only bad news: {}",
        out.stderr
    );
    // Both claims are still rendered; it is the second *name* that could not be placed.
    let text = out.text();
    assert!(
        text.contains("the first account") && text.contains("a different account"),
        "no unit was dropped, only a name: {text}"
    );
}
