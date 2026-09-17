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
