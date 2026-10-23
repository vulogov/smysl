//! `smysl schema show|check` (SMYSL-2.5 §3.11, TX-P12 step 5), over the rule N fixtures.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/schema/rule-n")
        .join(name)
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the binary under test must run")
}

fn code(o: &Output) -> i32 {
    o.status.code().expect("an exit code")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

/// A declared widening holds rule N; each violation exits 3 with its code.
#[test]
fn check_exits_by_what_rule_n_finds() {
    let ok = run(&[
        "schema",
        "check",
        fixture("widening-paired.cbor").to_str().unwrap(),
    ]);
    assert_eq!(code(&ok), 0, "{}", err(&ok));
    assert!(
        out(&ok).starts_with("x.rule/v1: rule N holds over 2 revision(s)"),
        "{}",
        out(&ok)
    );
    for (file, want) in [
        ("widening-unpaired.cbor", "SMY-E426"),
        ("reintroduced-key.cbor", "SMY-E426"),
        ("reintroduced-value.cbor", "SMY-E426"),
        ("new-meaning.cbor", "SMY-E426"),
        ("type-change.cbor", "SMY-E426"),
    ] {
        let o = run(&[
            "schema",
            "check",
            "x.rule/v1",
            fixture(file).to_str().unwrap(),
        ]);
        assert_eq!(code(&o), 3, "{file}: {}", err(&o));
        assert!(err(&o).contains(want), "{file}: {}", err(&o));
    }
    let fork = run(&["schema", "check", fixture("fork.cbor").to_str().unwrap()]);
    assert_eq!(code(&fork), 0, "a fork warns: {}", err(&fork));
    assert!(err(&fork).contains("SMY-W429"), "{}", err(&fork));
}

/// `show` lists revisions, fields, steps, chains and forks.
#[test]
fn show_prints_revisions_steps_and_chains() {
    let path = fixture("widening-paired.cbor");
    let o = run(&["schema", "show", "x.rule/v1", path.to_str().unwrap()]);
    assert_eq!(code(&o), 0, "{}", err(&o));
    let text = out(&o);
    for want in [
        "x.rule/v1\n  current: @2\n",
        "  @1  types x.rule/item\n",
        "      rule:hushed  bool  required-from 2 when rule:mode = loud\n",
        "    1->2  split-value rule:mode quiet -> loud + rule:hushed = true; require rule:hushed when rule:mode = loud; default rule:hushed = false when rule:mode = loud\n",
        "  chains to current:\n    @1->@2  ",
        "  forks: none\n",
    ] {
        assert!(text.contains(want), "missing {want:?} in\n{text}");
    }
    let o = run(&["schema", "show", "-s", path.to_str().unwrap(), "x.rule/v1"]);
    assert_eq!(code(&o), 0, "--store gives the store: {}", err(&o));
    let none = run(&["schema", "show", "x.none/v1", path.to_str().unwrap()]);
    assert_eq!(code(&none), 1);
    let usage = run(&["schema", "show", path.to_str().unwrap()]);
    assert_eq!(code(&usage), 2, "{}", err(&usage));
}

/// `migrate` stages a re-derivation and exits 10; a dry run stages nothing (TX-P12 step 6).
#[test]
#[cfg(feature = "stage")]
fn migrate_stages_and_exits_ten() {
    use smysl::schema::UnitView;
    use smysl::surface::HValue;
    use smysl::{Record, SchemaId, Status, UnitCoreBuilder};

    let dir = std::env::temp_dir().join(format!("smysl-cmd-migrate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bytes = std::fs::read(fixture("widening-paired.cbor")).unwrap();
    let (mut records, _) = smysl::from_cbor_seq(&bytes).unwrap();
    // Written at 1: `quiet`, which revision 2 splits into `loud` plus `rule:hushed`.
    let view = UnitView {
        ty: SchemaId::parse("x.rule/item").unwrap(),
        payload: [("rule:mode".to_string(), HValue::Str("quiet".into()))].into(),
    };
    let unit = UnitCoreBuilder::new(view.ty.clone(), "a quiet one", Status::Speculative)
        .payload(view.payload_bytes().unwrap())
        .build()
        .unwrap();
    records.push(Record::Unit(unit));
    let store = dir.join("store.cbor");
    std::fs::write(&store, smysl::to_cbor_seq(&records)).unwrap();
    let store = store.to_str().unwrap();

    let dry = run(&[
        "migrate",
        "--schema",
        "x.rule/v1",
        "--to",
        "2",
        "--dry-run",
        store,
    ]);
    assert_eq!(code(&dry), 0, "{}", err(&dry));
    assert!(
        out(&dry).starts_with("1 unit(s) of x.rule/v1 re-derived under @2"),
        "{}",
        out(&dry)
    );
    assert!(!dir.join(".smysl/staged.smy").exists());

    let staged = run(&["migrate", "--schema", "x.rule/v1", "--to", "2", store]);
    assert_eq!(code(&staged), 10, "{}", err(&staged));
    let text = std::fs::read_to_string(dir.join(".smysl/staged.smy")).unwrap();
    assert!(text.contains("\"rule:hushed\": true"), "{text}");
    assert!(text.contains("supersedes"), "{text}");

    let none = run(&["migrate", "--schema", "x.rule/v1", "--to", "9", store]);
    assert_eq!(code(&none), 1, "{}", err(&none));
    assert!(err(&none).contains("SMY-E466"), "{}", err(&none));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `fmt` round-trips an FC-9 declaration (TX-P12 step 7's exit, at the command): Appendix C's
/// `x.text/v1` written canonically, and the canonical text checked as its own fixed point.
#[test]
fn fmt_round_trips_an_fc9_declaration() {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-fmt-fc9-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rfc = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("Documentation/SMYSL-2_RFC/SMYSL-2.3_FORMAT_AMENDMENT.md"),
    )
    .unwrap();
    let start = rfc.find("@schema x.text/v1 {").unwrap();
    let end = start + rfc[start..].find("\n}\n").unwrap() + 3;
    let src = dir.join("appendix-c.smy");
    std::fs::write(&src, &rfc[start..end]).unwrap();
    let first = run(&["fmt", "--format", "surface", src.to_str().unwrap()]);
    assert_eq!(code(&first), 0, "{}", err(&first));
    let canonical = out(&first);
    assert!(canonical.contains("  fields: {\n"), "{canonical}");
    assert!(
        canonical.contains("\"text:mode\": { type: enum, values: [asserted, reported"),
        "{canonical}"
    );
    let again = dir.join("canonical.smy");
    std::fs::write(&again, &canonical).unwrap();
    let check = run(&["fmt", "--check", again.to_str().unwrap()]);
    assert_eq!(
        code(&check),
        0,
        "the canonical text is its own fixed point: {}",
        err(&check)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
