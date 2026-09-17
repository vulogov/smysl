//! `fixtures/wire/relation-id/cases.json` and `fixtures/wire/contention-id/cases.json` (1.4).
//!
//! A withdrawal names an edge by its rid and a resolution names a contention by its id, so both
//! derivations became format: an implementation that derived either differently would read every
//! other implementation's withdrawals and resolutions as naming nothing. The fixtures carry the
//! inputs, the preimage and the result, so a reader that disagrees can tell whether it built the
//! preimage differently or hashed differently.
//!
//! The comparison runs on every `cargo test`. `SMYSL_BLESS=1` rewrites the files; doing that
//! should be a decision, because it changes what every other implementation is checked against.

use std::path::Path;

use smysl_core::{hash_bytes, ContentionId, DetectionKind, RelKind, Relation, Uid};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn uid(tag: &str) -> Uid {
    Uid::from_bytes(hash_bytes(tag.as_bytes()))
}

fn golden(rel: &str, generated: String) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel);
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}; SMYSL_BLESS=1 creates it", path.display()));
    assert_eq!(
        on_disk, generated,
        "{rel} no longer matches the derivation; that is a format change"
    );
}

#[test]
fn relation_ids_match_the_fixture() {
    let cases: Vec<(&str, RelKind, Uid, Uid, &str)> = vec![
        (
            "kernel-kind",
            RelKind::Rebuts,
            uid("claim"),
            uid("rebuttal"),
            "`rebuts` is encoded on the wire as a kernel integer; the rid hashes its name",
        ),
        (
            "reversed-endpoints",
            RelKind::Rebuts,
            uid("rebuttal"),
            uid("claim"),
            "direction is identity",
        ),
        (
            "extension-kind",
            RelKind::parse("x.verify/supports").unwrap(),
            uid("claim"),
            uid("commit"),
            "an extension kind is encoded as text, and hashed as the same text",
        ),
        (
            "self-loop-retraction",
            RelKind::Retracts,
            uid("claim"),
            uid("claim"),
            "the retraction edge `smysl retract` writes",
        ),
    ];

    let mut out = String::from(
        "{\n  \"purpose\": \"Relation ids (rids), produced by the Rust. rid = BLAKE3-256(0x03 || kind name || 0x00 || from || to). A withdrawal and an edge attestation name a relation by this. `preimage_hex` is included so a mismatch says whether the preimage or the hash disagreed.\",\n  \"cases\": [\n",
    );
    for (i, (name, kind, from, to, note)) in cases.iter().enumerate() {
        let rel = Relation::new(kind.clone(), *from, *to);
        let mut pre = vec![0x03];
        pre.extend_from_slice(kind.as_str().as_bytes());
        pre.push(0);
        pre.extend_from_slice(from.as_bytes());
        pre.extend_from_slice(to.as_bytes());
        assert_eq!(Uid::from_bytes(hash_bytes(&pre)), rel.uid(), "{name}");
        out.push_str(&format!(
            "    {{\n      \"name\": \"{name}\",\n      \"note\": \"{note}\",\n      \"kind\": \"{}\",\n      \"from_hex\": \"{}\",\n      \"to_hex\": \"{}\",\n      \"preimage_hex\": \"{}\",\n      \"rid_hex\": \"{}\",\n      \"rid\": \"{}\"\n    }}{}\n",
            kind.as_str(),
            hex(from.as_bytes()),
            hex(to.as_bytes()),
            hex(&pre),
            hex(rel.uid().as_bytes()),
            rel.uid().canonical(),
            if i + 1 < cases.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    golden("fixtures/wire/relation-id/cases.json", out);
}

#[test]
fn contention_ids_match_the_fixture() {
    let cases: Vec<(&str, DetectionKind, Uid, Vec<Uid>, &str)> = vec![
        (
            "live-rebuttal",
            DetectionKind::LiveRebuttal,
            uid("claim"),
            vec![uid("rebuttal"), uid("claim")],
            "positions listed unsorted; the id sorts them",
        ),
        (
            "duplicate-position",
            DetectionKind::LiveRebuttal,
            uid("claim"),
            vec![uid("claim"), uid("rebuttal"), uid("rebuttal")],
            "the same contention as above: positions are deduplicated",
        ),
        (
            "supersession-fork",
            DetectionKind::SupersessionFork,
            uid("claim"),
            vec![uid("successor-a"), uid("successor-b")],
            "kind is identity",
        ),
    ];

    let mut out = String::from(
        "{\n  \"purpose\": \"Contention ids, produced by the Rust. digest = BLAKE3-256(kind byte || over || sorted, deduplicated positions); id = \\\"k/c\\\" + the first 130 bits of digest in the base32 of spec section 2.1 (26 characters). A resolution names a contention by this.\",\n  \"cases\": [\n",
    );
    for (i, (name, kind, over, positions, note)) in cases.iter().enumerate() {
        let mut sorted = positions.clone();
        sorted.sort();
        sorted.dedup();
        let mut pre = vec![kind.as_u8()];
        pre.extend_from_slice(over.as_bytes());
        for p in &sorted {
            pre.extend_from_slice(p.as_bytes());
        }
        let id = ContentionId::derive(*kind, over, positions);
        let positions_hex: Vec<String> = positions
            .iter()
            .map(|p| format!("\"{}\"", hex(p.as_bytes())))
            .collect();
        out.push_str(&format!(
            "    {{\n      \"name\": \"{name}\",\n      \"note\": \"{note}\",\n      \"kind\": {},\n      \"over_hex\": \"{}\",\n      \"positions_hex\": [{}],\n      \"preimage_hex\": \"{}\",\n      \"digest_hex\": \"{}\",\n      \"id\": \"{}\"\n    }}{}\n",
            kind.as_u8(),
            hex(over.as_bytes()),
            positions_hex.join(", "),
            hex(&pre),
            hex(&hash_bytes(&pre)),
            id.as_str(),
            if i + 1 < cases.len() { "," } else { "" }
        ));
    }
    out.push_str("  ]\n}\n");
    golden("fixtures/wire/contention-id/cases.json", out);
}

/// `fixtures/wire/F10-lifecycle.cbor`: a store holding 1.4's records — a withdrawal, two
/// resolutions and an attestation naming an edge's rid — built from surface text, so the other
/// implementations' round-trip suites carry them from the moment they exist.
#[test]
fn the_lifecycle_wire_fixture_matches_its_source() {
    let src = "\
@claim c/pool { status: speculative }
~ The eu-west connection pool saturated.

@claim c/half { status: speculative }
~ The pool never exceeded half its size.

@claim c/canary { status: speculative }
~ The canary shard stayed clean.

@rel c/half --rebuts--> c/pool { weight: 0.5 }

@rel c/canary --rebuts--> c/pool

@withdraw c/half --rebuts--> c/pool { agent: human:reviewer, ts: [1726500000000, 0], reason: c/canary }

@resolve c/canary --rebuts--> c/pool { agent: human:reviewer, ts: [1726500000001, 0] }

@resolve k/ccm3actwjjti65famnoe6mapo5d { agent: \"model:gemini/flash-lite\", ts: [1726500000002, 3] }
";
    let out = smysl_core::surface::parse_surface(src).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let mut records = out.records;
    let pool = out.labels[&smysl_core::Label::new("c/pool").unwrap()];
    let canary = out.labels[&smysl_core::Label::new("c/canary").unwrap()];
    let agent = smysl_core::AgentId::new("model:gemini/flash-lite").unwrap();
    records.push(smysl_core::Record::Attestation(
        smysl_core::Attestation::new(
            Relation::new(RelKind::Rebuts, canary, pool).uid(),
            agent.clone(),
            smysl_core::Op::Imported,
            smysl_core::Rung::Model,
            smysl_core::Hlc::new(1726500000000, 0, agent),
        ),
    ));
    let bytes = smysl_core::to_cbor_seq(&records);
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/F10-lifecycle.cbor");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &bytes).unwrap();
        return;
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "fixtures/wire/F10-lifecycle.cbor no longer matches its source"
    );
    let kinds: Vec<&str> = records.iter().map(|r| r.type_name()).collect();
    for k in ["withdrawal", "resolution", "attestation"] {
        assert!(kinds.contains(&k), "{k} missing from the fixture");
    }
}
