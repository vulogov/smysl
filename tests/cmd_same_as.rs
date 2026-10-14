//! `same-as` through the binary (SMYSL-2.4 §4.4, TX-P7 step 2).
//!
//! The engines and the anthology exit are tested where they live — `smysl-text`'s
//! `proposition` module and `smysl-ingest`'s `tests/same_as.rs`. What is only testable here is
//! the command: that `propose` stages rather than commits (rule S, exit 10), that `--yes`
//! commits, that `classes` reads and writes nothing, and that an engine asking for evidence
//! the corpus does not have says so as a usage error rather than reporting an empty result.
//!
//! The store is a surface document rather than a library, and that is one of the things under
//! test: `identical-span` reads units and nothing else, so a store of text units exported from
//! a library is a fair thing to run it over, and `--engine anchored` over the same store is
//! the case that has to be refused.

#![cfg(all(feature = "cli", feature = "text"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

/// One part, borrowed from the wire fixtures. Nothing reads it: what it is for is to put two
/// units at the same bytes of the same text.
const TID: &str = "t3:2ruhul6f4o3a735poauxd3snjdxdzol44imwk7gfse5jed2rb5ra";
/// Two manifests over that part — the collection, and the anthology that reprinted it.
const COLLECTION: &str = "m3:2ruhul6f4o3a735poauxd3snjdxdzol44imwk7gfse5jed2rb5ra";
const ANTHOLOGY: &str = "m3:4ruhul6f4o3a735poauxd3snjdxdzol44imwk7gfse5jed2rb5ra";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smysl-sameas-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .output()
        .expect("the binary under test must run")
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Two units at the same bytes under two manifests, and one at other bytes.
fn store(dir: &Path) -> PathBuf {
    let p = dir.join("corpus.smy");
    std::fs::write(
        &p,
        format!(
            "@claim {{ status: cited, lang: en, source: {{ kind: doc, ref: \"{TID}#p.1\", \
             span: [0, 120], manifest: {COLLECTION} }} }}\n\
             ~ The porter saw nobody pass and was looking at the mud.\n\
             \n\
             @claim {{ status: cited, lang: en, source: {{ kind: doc, ref: \"{TID}#p.1\", \
             span: [0, 120], manifest: {ANTHOLOGY} }} }}\n\
             ~ The porter saw nobody pass and was looking at the mud.\n\
             \n\
             @claim {{ status: cited, lang: en, source: {{ kind: doc, ref: \"{TID}#p.2\", \
             span: [120, 240], manifest: {COLLECTION} }} }}\n\
             ~ A ship is owned, sailed and insured by three different men.\n"
        ),
    )
    .expect("the document is written");
    p
}

/// `propose` stages and does not commit: rule S, and exit 10 is how a pipeline is told.
#[test]
fn propose_stages_the_edge_and_leaves_the_store_alone() {
    let dir = scratch("stage");
    let p = store(&dir);
    let before = std::fs::read(&p).expect("the document");

    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "identical-span",
            "-s",
            p.to_str().expect("a path"),
        ],
    );
    let text = out(&o);
    assert_eq!(
        o.status.code(),
        Some(10),
        "rule S: staged, not committed\n{text}"
    );
    assert!(
        text.contains("1 edge(s) staged"),
        "one reprint is one edge: {text}"
    );
    assert!(
        text.contains("identical-span") && text.contains("the same bytes 0..120"),
        "the run has to say what proposed the pair and why: {text}"
    );
    assert_eq!(
        std::fs::read(&p).expect("the document"),
        before,
        "staging wrote to the store"
    );
    assert!(
        dir.join(".smysl/staged.cbor").exists(),
        "the batch is not in the stage"
    );

    // Nothing is in the store, so nothing is a class yet.
    let o = run(
        &dir,
        &["same-as", "classes", "-s", p.to_str().expect("a path")],
    );
    assert!(
        out(&o).contains("0 class(es)"),
        "a staged edge is not a committed edge: {}",
        out(&o)
    );
}

/// `--yes` commits, and then the two units are one class.
///
/// Into a **CBOR** store, and that is not an incidental detail of the fixture: an attestation
/// has no surface form, so `commit_staged` refuses a surface target rather than keeping the
/// edge and losing the agent that proposed it — which is the one record that makes
/// `attested:n` answerable. The refusal is 1.9's and is asserted below, because a command that
/// stages attestations should be shown to meet it.
#[test]
fn yes_commits_and_the_reprint_becomes_a_class() {
    let dir = scratch("commit");
    let surface = store(&dir);
    let p = dir.join("corpus.cbor");
    let o = run(
        &dir,
        &[
            "merge",
            surface.to_str().expect("a path"),
            "-o",
            p.to_str().expect("a path"),
        ],
    );
    assert!(o.status.success(), "{}", out(&o));

    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "identical-span",
            "--yes",
            "-s",
            p.to_str().expect("a path"),
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("1 edge(s) proposed by identical-span and committed"),
        "{}",
        out(&o)
    );

    let o = run(
        &dir,
        &["same-as", "classes", "-s", p.to_str().expect("a path")],
    );
    let text = out(&o);
    assert!(o.status.success(), "{text}");
    assert!(
        text.contains("1 class(es) over 2 unit(s), 0 of them singleton(s)"),
        "the reprint and its original are one class of two: {text}"
    );

    // The default for corpus measures (§3.6) is not satisfied by one engine's run.
    let o = run(
        &dir,
        &[
            "same-as",
            "classes",
            "--policy",
            "attested:2",
            "-s",
            p.to_str().expect("a path"),
        ],
    );
    assert!(
        out(&o).contains("0 class(es)"),
        "one engine is one agent: {}",
        out(&o)
    );

    // The surface target, refused: an edge whose attestation cannot be written down is an
    // edge whose provenance would be lost by committing it.
    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "identical-span",
            "--yes",
            "-s",
            surface.to_str().expect("a path"),
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("is a surface store"), "{}", out(&o));

    // And a second run of the same engine finds the pair and stages nothing.
    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "identical-span",
            "-s",
            p.to_str().expect("a path"),
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("all of which this store already holds"),
        "{}",
        out(&o)
    );
}

/// `--engine anchored` over a plain store is a usage error, not an empty result.
#[test]
fn anchored_over_a_store_with_no_alignment_says_so() {
    let dir = scratch("anchored");
    let p = store(&dir);
    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "anchored",
            "-s",
            p.to_str().expect("a path"),
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(
        out(&o).contains("only a library has"),
        "the message has to say what is missing: {}",
        out(&o)
    );
}

/// `--engine` is required, and `--policy` is read strictly.
#[test]
fn the_arguments_are_refused_where_they_are_wrong() {
    let dir = scratch("usage");
    let p = store(&dir);
    let path = p.to_str().expect("a path");

    let o = run(&dir, &["same-as", "propose", "-s", path]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(
        out(&o).contains("the engine is the evidence"),
        "{}",
        out(&o)
    );

    let o = run(
        &dir,
        &["same-as", "classes", "--policy", "attested:0", "-s", path],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("at least 1"), "{}", out(&o));

    let o = run(
        &dir,
        &["same-as", "classes", "--policy", "sortof", "-s", path],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));

    // `semantic` is SMYSL-2.5's and is refused by clap rather than by a message saying it is
    // not in this build, which would read like a defect.
    let o = run(
        &dir,
        &["same-as", "propose", "--engine", "semantic", "-s", path],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
}

/// `--scope` over a plain store takes manifest ids, and says what it would take otherwise.
#[test]
fn a_scope_narrows_the_corpus() {
    let dir = scratch("scope");
    let p = store(&dir);
    let path = p.to_str().expect("a path");

    // The anthology's one unit alone: a pair needs two.
    let o = run(
        &dir,
        &[
            "same-as",
            "propose",
            "--engine",
            "identical-span",
            "--scope",
            ANTHOLOGY,
            "-s",
            path,
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    assert!(out(&o).contains("0 proposal(s)"), "{}", out(&o));

    let o = run(
        &dir,
        &[
            "same-as",
            "classes",
            "--scope",
            "collection/porter",
            "-s",
            path,
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("an alias needs a library"), "{}", out(&o));
}
