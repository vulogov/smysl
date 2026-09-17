//! `review`, `withdraw` and `resolve` — closing what verification opens (1.4) — and `retract`,
//! which until 1.4 reported a retraction it never wrote.
//!
//! Every test works on its own copy of a store in a scratch directory, because every command
//! here writes.

#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@claim c/pool { status: speculative }
~ The pool saturated.

@claim c/half { status: speculative }
~ The pool never exceeded half its size.

@rel c/half --rebuts--> c/pool
";

const EDGE: &str = "c/half --rebuts--> c/pool";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-review-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("s.smy"), DOC).unwrap();
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
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

/// A CBOR copy of the scratch store, which is what withdrawals and resolutions can be written to.
fn cbor(dir: &Path) {
    let o = run(dir, &["merge", "s.smy", "-o", "s.cbor"]);
    assert!(o.status.success(), "{}", out(&o));
}

fn review_json(dir: &Path, all: bool) -> String {
    let mut args = vec!["--json", "review"];
    if all {
        args.push("--all");
    }
    args.push("s.cbor");
    String::from_utf8(run(dir, &args).stdout).unwrap()
}

/// The whole loop: a rebuttal is open, a reviewer records the review, the queue empties and the
/// exit code says so — and the record is on disk, read back by a fresh process.
#[test]
fn a_rebuttal_is_open_until_resolved_and_the_resolution_is_written() {
    let dir = scratch("resolve");
    cbor(&dir);

    let o = run(&dir, &["review", "s.cbor"]);
    assert_eq!(o.status.code(), Some(5), "open items exit 5: {}", out(&o));
    assert!(out(&o).contains("1 item(s) open for review"), "{}", out(&o));
    assert!(out(&o).contains(EDGE), "{}", out(&o));

    let o = run(
        &dir,
        &[
            "resolve",
            "--as",
            "human:reviewer",
            "--at",
            "5",
            EDGE,
            "s.cbor",
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("resolved by human:reviewer"),
        "{}",
        out(&o)
    );

    let o = run(&dir, &["review", "s.cbor"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    assert!(review_json(&dir, true).contains("\"resolved\":true"));

    // The same reviewer again writes nothing more.
    let before = std::fs::read(dir.join("s.cbor")).unwrap();
    let o = run(&dir, &["resolve", "--as", "human:reviewer", EDGE, "s.cbor"]);
    assert!(out(&o).contains("already resolved"), "{}", out(&o));
    assert_eq!(std::fs::read(dir.join("s.cbor")).unwrap(), before);
}

/// A withdrawal is refused under the default origin authority for an edge nobody attested,
/// written under `any`, and read back: the rebuttal leaves the queue.
#[test]
fn a_withdrawal_respects_authority_and_is_written() {
    let dir = scratch("withdraw");
    cbor(&dir);
    let before = std::fs::read(dir.join("s.cbor")).unwrap();

    let o = run(
        &dir,
        &["withdraw", "--as", "human:reviewer", EDGE, "s.cbor"],
    );
    assert_eq!(o.status.code(), Some(1), "{}", out(&o));
    assert!(out(&o).contains("origin authority"), "{}", out(&o));
    assert_eq!(
        std::fs::read(dir.join("s.cbor")).unwrap(),
        before,
        "refused, not written"
    );

    let o = run(
        &dir,
        &[
            "withdraw",
            "--dry-run",
            "--as",
            "human:reviewer",
            "--authority",
            "any",
            EDGE,
            "s.cbor",
        ],
    );
    assert!(
        out(&o).contains("would no longer carry this rebuttal"),
        "{}",
        out(&o)
    );
    assert!(
        out(&o).contains("1 item(s) would leave the review queue"),
        "{}",
        out(&o)
    );
    assert_eq!(
        std::fs::read(dir.join("s.cbor")).unwrap(),
        before,
        "dry run"
    );

    let o = run(
        &dir,
        &[
            "withdraw",
            "--as",
            "human:reviewer",
            "--authority",
            "any",
            "--at",
            "7",
            EDGE,
            "s.cbor",
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    assert_eq!(review_json(&dir, true), "{\"open\":0,\"items\":[]}\n");

    let o = run(
        &dir,
        &[
            "withdraw",
            "--as",
            "human:reviewer",
            "--authority",
            "any",
            EDGE,
            "s.cbor",
        ],
    );
    assert!(out(&o).contains("already withdrawn"), "{}", out(&o));
}

/// A surface store takes both records as `@withdraw` and `@resolve` lines appended to the file,
/// in its own labels, leaving what was there untouched; a fresh process reads them back.
#[test]
fn a_surface_store_holds_withdrawals_and_resolutions_as_text() {
    let dir = scratch("surface");
    let o = run(
        &dir,
        &[
            "resolve",
            "--as",
            "human:reviewer",
            "--at",
            "5",
            EDGE,
            "s.smy",
        ],
    );
    assert!(o.status.success(), "{}", out(&o));
    let o = run(
        &dir,
        &[
            "withdraw",
            "--as",
            "human:reviewer",
            "--authority",
            "any",
            "--at",
            "7",
            EDGE,
            "s.smy",
        ],
    );
    assert!(o.status.success(), "{}", out(&o));

    let text = std::fs::read_to_string(dir.join("s.smy")).unwrap();
    assert!(
        text.starts_with(DOC),
        "the original text was changed:\n{text}"
    );
    assert!(
        text.contains("@resolve c/half --rebuts--> c/pool { agent: human:reviewer, ts: [5, 0] }"),
        "{text}"
    );
    assert!(
        text.contains("@withdraw c/half --rebuts--> c/pool { agent: human:reviewer, ts: [7, 0] }"),
        "{text}"
    );
    let o = run(&dir, &["review", "s.smy"]);
    assert_eq!(o.status.code(), Some(0), "{}", out(&o));
    let o = run(&dir, &["check", "s.smy"]);
    assert!(o.status.success(), "{}", out(&o));
}

/// `retract` said "now read as unfounded" and wrote nothing, so the next process saw the unit
/// unretracted. It writes now — a line to a surface store, a record to a CBOR one — and a second
/// run changes nothing.
#[test]
fn a_retraction_is_written_once_to_either_kind_of_store() {
    let dir = scratch("retract");
    cbor(&dir);
    for store in ["s.smy", "s.cbor"] {
        let o = run(
            &dir,
            &[
                "retract",
                "--as",
                "human:reviewer",
                "--authority",
                "any",
                "c/half",
                store,
            ],
        );
        assert!(o.status.success(), "{store}: {}", out(&o));
        let written = std::fs::read(dir.join(store)).unwrap();

        let o = run(
            &dir,
            &[
                "retract",
                "--as",
                "human:reviewer",
                "--authority",
                "any",
                "c/half",
                store,
            ],
        );
        assert!(
            out(&o).contains("already retracted"),
            "{store}: {}",
            out(&o)
        );
        assert_eq!(
            std::fs::read(dir.join(store)).unwrap(),
            written,
            "{store}: written twice"
        );

        // A fresh process reads it: the rebuttal came from the retracted unit, so it is not live
        // and nothing is left to review.
        let o = run(&dir, &["review", store]);
        assert_eq!(o.status.code(), Some(0), "{store}: {}", out(&o));
    }
    let o = run(&dir, &["check", "s.smy"]);
    assert!(o.status.success(), "the appended line parses: {}", out(&o));
}

/// Kinds that cannot be withdrawn, and edges that cannot be reviewed, are usage errors.
#[test]
fn a_lifecycle_edge_cannot_be_withdrawn_and_a_non_rebuttal_cannot_be_resolved() {
    let dir = scratch("kinds");
    std::fs::write(
        dir.join("s.smy"),
        format!("{DOC}\n@rel c/half --elaborates--> c/pool\n@rel c/half --supersedes--> c/pool\n"),
    )
    .unwrap();
    cbor(&dir);

    let o = run(
        &dir,
        &[
            "withdraw",
            "--as",
            "human:r",
            "--authority",
            "any",
            "c/half --supersedes--> c/pool",
            "s.cbor",
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("SMY-W056"), "{}", out(&o));

    let o = run(
        &dir,
        &[
            "resolve",
            "--as",
            "human:r",
            "c/half --elaborates--> c/pool",
            "s.cbor",
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));

    let o = run(
        &dir,
        &[
            "resolve",
            "--as",
            "human:r",
            "c/half --causes--> c/pool",
            "s.cbor",
        ],
    );
    assert_eq!(o.status.code(), Some(1), "no such edge: {}", out(&o));
}

/// A rebuttal a thread presents is reviewed as its contention. F1's canary rebuttal is one, so
/// resolving the edge is redirected to the contention id, and a dry run on the contention says
/// what would be recorded. Read-only against the corpus: both are refused or dry.
#[test]
fn a_threaded_rebuttal_is_resolved_through_its_contention() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let f1 = "fixtures/corpus/F1-incident.smy";
    let o = run(
        root,
        &[
            "resolve",
            "--dry-run",
            "--as",
            "human:r",
            "c/canary-clean --rebuts--> c/pool-saturation",
            f1,
        ],
    );
    assert_eq!(o.status.code(), Some(2), "{}", out(&o));
    assert!(out(&o).contains("resolve that"), "{}", out(&o));

    let o = run(root, &["--json", "review", f1]);
    let json = String::from_utf8_lossy(&o.stdout).to_string();
    let id = json
        .split("\"id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("a contention id");
    let o = run(root, &["resolve", "--dry-run", "--as", "human:r", id, f1]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("would be resolved by human:r"),
        "{}",
        out(&o)
    );
}

/// `smysl compact` on a CBOR log holding repeats says how many and writes a store without them.
#[test]
fn compact_reports_and_removes_repeated_records() {
    let dir = scratch("compact");
    cbor(&dir);
    let once = std::fs::read(dir.join("s.cbor")).unwrap();
    let mut twice = once.clone();
    twice.extend_from_slice(&once);
    std::fs::write(dir.join("old.cbor"), &twice).unwrap();

    let o = run(&dir, &["compact", "old.cbor", "-o", "clean.cbor"]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("record(s) the log held more than once removed"),
        "{}",
        out(&o)
    );
    assert_eq!(
        std::fs::read(dir.join("clean.cbor")).unwrap().len(),
        once.len()
    );
}
