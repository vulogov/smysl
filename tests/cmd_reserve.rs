//! `pack --reserve` (R21, 1.6): a budget that counts what else is in the window.
//!
//! A caller sending a pack to a model spends its context window on a system prompt and a question
//! before a single unit arrives. `--reserve` states that, so the solver makes one decision instead
//! of packing to a number and having the caller trim what it chose.

#![cfg(feature = "cli")]

use std::path::PathBuf;
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@claim c/pool { status: speculative }
~ The eu-west connection pool is saturated.

@evidence e/wait { status: speculative }
~ Pool acquisition wait rose from 2 ms to 310 ms.

@decision d/raise { status: speculative }
~ Raise the pool ceiling for eu-west.
";

fn store() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-reserve-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("s.smy");
    std::fs::write(&path, DOC).unwrap();
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(["--format", "surface"])
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

/// `--budget b --reserve r` packs what `--budget b-r` packs, and says so in the summary.
#[test]
fn reserving_packs_what_the_smaller_budget_packs() {
    let path = store();
    let p = path.to_str().unwrap();

    let reserved = run(&["pack", "--budget", "60", "--reserve", "20", p]);
    let smaller = run(&["pack", "--budget", "40", p]);
    assert!(reserved.status.success(), "{}", out(&reserved));
    assert!(smaller.status.success(), "{}", out(&smaller));
    assert_eq!(
        String::from_utf8_lossy(&reserved.stdout),
        String::from_utf8_lossy(&smaller.stdout),
        "a reservation must be a statement about the budget, not a second one"
    );

    let explained = run(&["pack", "--budget", "60", "--reserve", "20", "--explain", p]);
    assert!(
        out(&explained).contains("of 40 (20 reserved of 60) tokens"),
        "the summary should report both numbers:\n{}",
        out(&explained)
    );
    let plain = run(&["pack", "--budget", "60", "--explain", p]);
    assert!(
        out(&plain).contains("of 60 tokens") && !out(&plain).contains("reserved"),
        "reserving nothing must print what it always printed:\n{}",
        out(&plain)
    );
}

/// Reserving the whole window is a usage error, not an empty pack.
#[test]
fn reserving_the_whole_budget_is_refused() {
    let path = store();
    let p = path.to_str().unwrap();
    for r in ["60", "600"] {
        let o = run(&["pack", "--budget", "60", "--reserve", r, p]);
        assert_eq!(o.status.code(), Some(2), "{}", out(&o));
        assert!(
            out(&o).contains("leaves nothing"),
            "the message should say what went wrong:\n{}",
            out(&o)
        );
    }
}
