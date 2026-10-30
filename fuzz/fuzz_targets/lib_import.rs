//! Import of arbitrary bytes (SMYSL-2.7 §5.3, TX-P11 step 11).
//!
//! The input is one file offered to `lib import --accept` over a small fixed library (F13).
//! It must not panic; it is either refused whole (`SMY-E493`) or every record it decoded to is
//! counted exactly once — held already, dropped by rule Z, or new and then accepted, waiting or
//! quarantined — and the library's log grows by at most the accepted records and the file's
//! one manifest.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::Code;
use smysl_lib::{import, ImportOptions, Input};
use smysl_text::Library;
use std::path::PathBuf;
use std::sync::OnceLock;

const SEED: &str = include_str!("../../fixtures/corpus/F13-canon.smy");

fn root() -> &'static PathBuf {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        std::env::temp_dir().join(format!("smysl-fuzz-lib-import-{}", std::process::id()))
    })
}

fuzz_target!(|data: &[u8]| {
    let dir = root();
    let _ = std::fs::remove_dir_all(dir);
    let mut lib = Library::create(dir).expect("a fresh library");
    let seed = smysl_core::surface::parse_surface(SEED)
        .expect("F13 parses")
        .records;
    lib.append_records(&seed).expect("F13 appends");
    let before = lib.store().iter().count();

    let input = Input::Bytes {
        name: "fuzz.cbor".to_string(),
        bytes: data.to_vec(),
    };
    let Ok(r) = import(
        &mut lib,
        &[input],
        &ImportOptions::new().accept(true).join(false),
    ) else {
        return;
    };
    let f = &r.files[0];
    if f.rejected {
        assert!(
            r.report.iter().any(|d| d.code == Code::E493),
            "a rejected file says why: {:?}",
            r.report
        );
        return;
    }
    assert_eq!(
        f.accepted + f.staged + f.waiting + f.quarantined,
        f.added,
        "every new record is classified once: {f:?}"
    );
    assert_eq!(f.staged, 0, "--accept stages nothing");
    let after = lib.store().iter().count();
    assert!(
        after <= before + f.accepted + 1,
        "the log grew by {} for {} accepted record(s)",
        after - before,
        f.accepted
    );
});
