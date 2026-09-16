//! What `smysl retract --dry-run` says, and what it no longer lets a reader infer.
//!
//! It printed "would reach N unit(s), orphaning M", where N counted the target plus the units
//! left with no support. Retracting one of a decision's two prerequisites said "reach 1" — true
//! of the retraction, and read by a caller building an impact report as "nothing depends on
//! this". The decision was left out because it keeps its other ground.

#![cfg(feature = "cli")]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "\
@claim p/one { status: speculative }
~ Prerequisite one.

@claim p/two { status: speculative }
~ Prerequisite two.

@decision d/choice { status: inferred, grounds: [p/one, p/two] }
~ A decision resting on both.

@claim c/only-on-one { status: inferred, grounds: [p/one] }
~ Something resting on prerequisite one alone.
";

fn dry_run(json: bool) -> String {
    let dir = std::env::temp_dir().join(format!("smysl-retract-{}-{json}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("prereq.smy");
    std::fs::write(&f, DOC).unwrap();
    let mut c = Command::new(BIN);
    if json {
        c.arg("--json");
    }
    let out = c
        .args(["retract", "--dry-run", "p/one", f.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).ok();
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_report_says_what_its_numbers_count_and_what_they_leave_out() {
    let out = dry_run(false);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert!(
        lines[0].contains("would leave 2 unit(s) unfounded, 1 of them orphaned"),
        "the target and its one orphan: {out}"
    );
    assert!(lines[1].contains("would lose all of its grounds"), "{out}");
    assert!(
        lines[2].contains("1 more unit(s) rest partly on it and keep other support"),
        "the decision with a surviving ground is the impact the old report hid: {out}"
    );
}

#[test]
fn the_json_report_names_the_partly_resting_units() {
    let out = dry_run(true);
    let at = out
        .find("\"rest_partly_on\":[")
        .expect("the key is missing");
    let list = &out[at..out[at..].find(']').unwrap() + at];
    assert_eq!(list.matches("b3:").count(), 1, "{out}");
}
