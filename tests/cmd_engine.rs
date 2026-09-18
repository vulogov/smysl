//! `find --engine` and `pack --engine` (1.7): the retriever the command ranks with.
//!
//! `Hybrid` was built, measured and exported, and no command could use it: `find` and
//! `pack --query` constructed a `Bm25` unconditionally, so a build compiled with
//! `--features semantic` still retrieved lexically. The numbers that engine records for itself —
//! 0.84 MRR against lexical's 0.74, 0.50 against 0.12 on paraphrase — were unreachable by anyone
//! who was not writing Rust.

#![cfg(feature = "cli")]

use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const STORE: &str = "fixtures/corpus/F1-incident.smy";
/// The synthetic 4 KB model. It exercises the path and means nothing about quality — its own
/// README says so, and no assertion here reads a score.
const TINY: &str = "fixtures/embed-tiny";

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .env_remove("SMYSL_EMBED_MODEL")
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

/// Lexical is the default, and it is what every release before 1.7 did.
#[test]
fn the_default_engine_is_lexical() {
    let bare = run(&["find", "connection pool", STORE]);
    let named = run(&["find", "connection pool", "--engine", "lexical", STORE]);
    assert!(bare.status.success(), "{}", out(&bare));
    assert_eq!(
        String::from_utf8_lossy(&bare.stdout),
        String::from_utf8_lossy(&named.stdout),
        "naming lexical must change nothing"
    );
}

/// A semantic run without a model is refused rather than quietly answered by the other engine:
/// falling back would report numbers from an engine the caller did not ask for.
#[test]
fn semantic_without_a_model_is_refused() {
    for engine in ["semantic", "hybrid"] {
        let o = run(&["find", "connection pool", "--engine", engine, STORE]);
        assert_eq!(o.status.code(), Some(2), "{}", out(&o));
        assert!(
            out(&o).contains("needs a model"),
            "the message should say what is missing:\n{}",
            out(&o)
        );
    }
}

/// An engine nobody implements is a usage error, not a fallback.
#[test]
fn an_unknown_engine_is_refused() {
    let o = run(&["find", "pool", "--engine", "telepathy", STORE]);
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
}

/// Without the feature, asking for it says so in the phrasing every other absent layer uses.
#[cfg(not(feature = "semantic"))]
#[test]
fn a_build_without_the_embedder_says_so() {
    let o = run(&[
        "find",
        "connection pool",
        "--engine",
        "hybrid",
        "--model",
        TINY,
        STORE,
    ]);
    assert!(!o.status.success());
    assert!(
        out(&o).contains("this build has no semantic retrieval"),
        "{}",
        out(&o)
    );
}

/// With the feature and a model, both engines rank and `pack --query` focuses through them.
#[cfg(feature = "semantic")]
#[test]
fn the_engines_rank_and_pack_focuses_through_them() {
    for engine in ["semantic", "hybrid"] {
        let o = run(&[
            "find",
            "connection pool",
            "--engine",
            engine,
            "--model",
            TINY,
            STORE,
        ]);
        assert!(o.status.success(), "{engine}: {}", out(&o));
        assert!(
            String::from_utf8_lossy(&o.stdout).lines().count() >= 1,
            "{engine} returned nothing: {}",
            out(&o)
        );
    }

    let packed = run(&[
        "--format",
        "surface",
        "pack",
        "--budget",
        "400",
        "--query",
        "connection pool",
        "--engine",
        "hybrid",
        "--model",
        TINY,
        STORE,
    ]);
    assert!(packed.status.success(), "{}", out(&packed));
    assert!(
        out(&packed).contains("--query focused"),
        "the pack should say what it focused on:\n{}",
        out(&packed)
    );
}

/// `SMYSL_EMBED_MODEL` is the other way to name one, so a pipeline sets it once.
#[cfg(feature = "semantic")]
#[test]
fn the_environment_can_name_the_model() {
    let o = Command::new(BIN)
        .args(["find", "connection pool", "--engine", "semantic", STORE])
        .env("SMYSL_EMBED_MODEL", TINY)
        .output()
        .expect("the binary under test must run");
    assert!(o.status.success(), "{}", out(&o));
}
