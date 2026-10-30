//! `smysl ingest --granularity` names a preset or is refused as a usage error.
//!
//! It accepted anything and only hashed it into the recipe, so `--granularity bogus` ran, spent
//! the calls, and recorded a recipe no real run shared.

#![cfg(all(feature = "cli", feature = "ingest"))]

use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

#[test]
fn an_unknown_granularity_is_a_usage_error_before_anything_runs() {
    let dir = std::env::temp_dir().join(format!("smysl-granularity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();
    let out = Command::new(BIN)
        .current_dir(&dir)
        .args([
            "ingest",
            "--offline",
            "--granularity",
            "bogus",
            f.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(2), "{stderr}");
    for preset in ["coarse", "default", "standard", "fine"] {
        assert!(stderr.contains(preset), "{preset} not offered: {stderr}");
    }
    assert!(!dir.join(".smysl/staged.smy").exists(), "it ran anyway");
}

/// `--temperature` is range-checked before anything is paid for.
///
/// H-12: temperature has been a condition of the recipe since recipes existed and has never
/// been settable outside the library, so a deployment that wanted anything but 0.0 had to
/// write its own binary. A value a provider would reject is refused here instead, for the
/// same reason `--granularity` is: the alternative is spending the calls to find out.
#[test]
fn a_temperature_outside_the_range_is_a_usage_error_before_anything_runs() {
    let dir = std::env::temp_dir().join(format!("smysl-temperature-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("doc.txt");
    std::fs::write(&f, "The pool saturated.").unwrap();
    // `--temperature=<t>`, not two arguments: clap reads a leading `-` as the start of a flag,
    // so `--temperature -0.1` is refused as `unexpected argument '-0'` and never reaches the
    // range check. The `=` form is how a negative value is passed, and it is what is tested.
    let run = |t: &str| {
        Command::new(BIN)
            .current_dir(&dir)
            .args([
                "ingest",
                "--offline",
                &format!("--temperature={t}"),
                f.to_str().unwrap(),
            ])
            .output()
            .unwrap()
    };

    for t in ["2.1", "-0.1"] {
        let out = run(t);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{t} was accepted: {stderr}");
        assert!(
            stderr.contains("0.0 to 2.0"),
            "the refusal must say the range: {stderr}"
        );
        assert!(!dir.join(".smysl/staged.smy").exists(), "{t} ran anyway");
    }

    // The ends of the range are inside it, and 2.0 gets past the check — it fails later, for
    // want of a provider, which is a different exit code. The point is that it was not refused
    // here.
    let out = run("2.0");
    assert_ne!(
        out.status.code(),
        Some(2),
        "2.0 is in range: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
