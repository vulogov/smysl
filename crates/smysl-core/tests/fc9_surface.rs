//! `@schema`'s `fields:` and `lenses:` (SMYSL-2.3 A-10 item 4; SMYSL-2.5 TX-P12 step 7).
//!
//! The exit is a `fmt` round trip of an FC-9 declaration: written, read back, and written again,
//! byte for byte, over every declaration in `fixtures/schema/` that can be spelled — and
//! Appendix C's own text, which reads to the declaration `fixtures/schema/fc9/text-v1.cbor`
//! holds. A declaration that cannot be spelled (an unknown op, an opaque shape) stays CBOR-only.

use std::path::PathBuf;

use smysl_core::surface::write::schema_decl_has_surface_form;
use smysl_core::surface::{parse_surface, write_surface, WriteContext};
use smysl_core::types::Record;
use smysl_core::{from_cbor_seq, to_cbor_seq};

fn dir(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/schema")
        .join(rel)
}

fn decls(path: PathBuf) -> Vec<Record> {
    let (records, _) = from_cbor_seq(&std::fs::read(&path).unwrap()).unwrap();
    records
        .into_iter()
        .filter(|r| matches!(r, Record::SchemaDecl(_)))
        .collect()
}

fn round_trip(name: &str, records: &[Record]) {
    let ctx = WriteContext::default();
    let text = write_surface(None, records, &ctx);
    let parsed = parse_surface(&text).unwrap_or_else(|e| panic!("{name}: {e}\n{text}"));
    assert_eq!(
        to_cbor_seq(&parsed.records),
        to_cbor_seq(records),
        "{name}: the bytes come back\n{text}"
    );
    assert_eq!(
        write_surface(None, &parsed.records, &ctx),
        text,
        "{name}: idempotent"
    );
}

#[test]
fn every_spellable_declaration_round_trips_byte_for_byte() {
    let mut spelled = 0;
    let mut cbor_only = Vec::new();
    for sub in ["fc9", "rule-n"] {
        for entry in std::fs::read_dir(dir(sub)).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "cbor") {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            for r in decls(path) {
                let Record::SchemaDecl(d) = &r else { continue };
                if schema_decl_has_surface_form(d) {
                    round_trip(
                        &format!("{name} {}@{}", d.id, d.version),
                        std::slice::from_ref(&r),
                    );
                    spelled += 1;
                } else {
                    cbor_only.push(format!("{name} {}@{}", d.id, d.version));
                }
            }
        }
    }
    cbor_only.sort();
    assert_eq!(
        cbor_only,
        vec![
            "opaque.cbor x.opaque/v1@1".to_string(),
            "unknown-op.cbor x.unknown/v1@2".to_string(),
        ],
        "only what cannot be spelled stays CBOR-only"
    );
    assert!(spelled >= 20, "{spelled}");
}

/// SMYSL-2.3 Appendix C, as written there, reads to the declaration the FC-9 fixture holds.
#[test]
fn appendix_c_reads_to_the_fixture() {
    let rfc = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../Documentation/SMYSL-2_RFC/SMYSL-2.3_FORMAT_AMENDMENT.md"),
    )
    .unwrap();
    let start = rfc.find("@schema x.text/v1 {").expect("Appendix C");
    let end = start + rfc[start..].find("\n}\n").expect("its end") + 3;
    let parsed = parse_surface(&rfc[start..end]).unwrap_or_else(|e| panic!("{e}"));
    let want = decls(dir("fc9/text-v1.cbor"));
    assert_eq!(to_cbor_seq(&parsed.records), to_cbor_seq(&want));
}

/// A misspelt field key is refused, not carried: `required_from` passed over would leave a
/// key unrequired under a file that visibly requires it.
#[test]
fn a_misspelt_key_is_refused() {
    let text = "@schema x.a/v1 {\n  version: 1,\n  fields: {\n    \"a:b\": { type: text, required_from: 1 },\n  },\n}\n";
    let out = parse_surface(text).unwrap();
    assert!(out.records.is_empty());
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.code.as_str() == "SMY-E001" && d.message.contains("required_from")),
        "{:?}",
        out.diagnostics
    );
    let text = "@schema x.a/v1 {\n  version: 2,\n  lenses: [ { from: 1, to: 2, ops: [ { tag: transpose, key: k } ] } ],\n}\n";
    let out = parse_surface(text).unwrap();
    assert!(out.records.is_empty());
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message.contains("transpose")),
        "{:?}",
        out.diagnostics
    );
}
