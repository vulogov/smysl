//! The manual's transcripts, replayed against the binary, as something `cargo test` can see.
//!
//! `scripts/verify-doc-output.py` has replayed ~190 documented command outputs since 0.3 and
//! has been a good gate — it caught the 34 transcripts that went stale when `check` changed
//! what it reports. But nothing in `cargo test` invoked it, so nothing that *counts* coverage
//! could see it either.
//!
//! That mattered for one number in particular. The CLI scored **73.6% mutation survivors** in
//! 0.12, more than twice the worst library crate, on a reading of `src/main.rs` as having four
//! tests across 3 600 lines. Part of that is real. Part of it was that the largest body of
//! checking the CLI has was invisible to the tool doing the measuring — `cargo-mutants` runs
//! `cargo test`, and `make doc-output` is not `cargo test`.
//!
//! So this is a measurement instrument before it is a test. Phase 2.1 of `ROAD_TO_1.0.md` says
//! the output is a fact rather than a fix: if the survivor rate collapses, the CLI was better
//! tested than it looked and the finding was about measurement. If it barely moves, four tests
//! across 3 600 lines is the whole story.
//!
//! # Why the binary path is passed in
//!
//! `CARGO_BIN_EXE_smysl` is the binary cargo built *for this test*. `cargo-mutants` rebuilds
//! it with a mutation applied, so replaying against it exercises the mutated code. The script's
//! own default — `./target/debug/smysl` — would often be the same file and sometimes not, and
//! "sometimes not" is a check that reports every mutant as caught while testing none of them.
//!
//! # Why the feature gate is so specific
//!
//! The script compares against output the manual claims, and the manual documents a
//! **default-features** build. Its own header records being bitten by this: a stale
//! `--all-features` binary made a correct `SMY-W202` claim look like drift, because
//! `exact-pack` was compiled in where the doc assumed it was not.
//!
//! So this runs in exactly one of the eight matrix configurations — the plain
//! `cargo test --workspace` — and is compiled out of the rest. Compiled out rather than
//! skipped at runtime, for the reason `global_flags.rs` gives: a test that silently passes
//! because it could not find its subject is worse than one that is not there.
#![cfg(all(
    feature = "cli",
    feature = "local",
    feature = "render-typst",
    not(feature = "exact-pack"),
    not(feature = "tui"),
    not(feature = "semantic"),
    not(feature = "remote"),
    not(feature = "render-html")
))]

use std::process::Command;

#[test]
fn the_manual_still_describes_this_binary() {
    let out = Command::new("python3")
        .arg("scripts/verify-doc-output.py")
        .env("SMYSL_BIN", env!("CARGO_BIN_EXE_smysl"))
        .output()
        .expect("python3 must be available to replay the manual's transcripts");

    let stdout = String::from_utf8_lossy(&out.stdout);

    // The control, and it has to be this specific.
    //
    // The script prints `ran N, skipped N, excerpt-matched N, MISMATCHED N`. The first version
    // of this test asserted only that the word `MISMATCHED` appeared, which is satisfied by
    // `ran 0, skipped 168, MISMATCHED 0` — a script that replayed nothing at all. It was
    // satisfied by exactly that, for a real reason: this test passes an *absolute*
    // `CARGO_BIN_EXE_smysl`, and the script's absolute-path rule then matched the program
    // itself and skipped every command in the book. Changing the binary's output and watching
    // this test still pass is how that was found.
    //
    // So the assertion is on the count. A gate that cannot tell "nothing was wrong" from
    // "nothing was checked" is the failure this repository keeps rediscovering.
    let ran: usize = stdout
        .split("ran ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or_else(|| {
            panic!(
                "the replay printed no summary line.\nstdout:\n{stdout}\nstderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            )
        });
    // The floor is a ratchet: 40, then 70 when 1.1 took the count from 46 to 78, then 80 when
    // 1.2 replayed chapters as sequences and reached 88. Set just under the real number: high
    // enough that losing a chapter's worth of coverage fails, low enough that deleting one
    // transcript does not. The three ways coverage has silently collapsed here were
    // all-or-nothing — an absolute path matching the program, a block regex that could not see
    // a chapter, a label mistaken for a missing file — so the gap between 80 and 88 is not
    // where the risk lives.
    assert!(
        ran >= 80,
        "the replay covered {ran} transcripts, which is too few to mean anything — 88 of the \
         manual's 194 are replayable, and a sudden drop means the script stopped matching \
         them rather than that the manual got shorter.\nstdout:\n{stdout}"
    );

    assert!(
        out.status.success(),
        "the manual and the binary disagree:\n{stdout}"
    );
}
