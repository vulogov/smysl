//! Emit `fixtures/wire/uid/cases.json` — unit cores, their canonical bytes, and their uids.
//!
//! ```text
//! cargo test -p smysl-core --test gen_uid_fixtures -- --ignored
//! ```
//!
//! §2.3 — *status is part of identity* — is the paragraph the whole format rests on, and until
//! now it was verified by this implementation alone. C-Read cannot reach it: reading a document
//! never requires deriving a uid, so the three implementations written from the specification
//! could round-trip every fixture byte for byte without ever computing one.
//!
//! Closing that means a second implementation at **C-Produce**, and a second implementation is
//! only evidence if it is checked against *this* one's output rather than against itself. So
//! the fixture carries the inputs, the canonical bytes and the uid: a reader that disagrees can
//! tell whether it encoded differently or hashed differently, which is the difference between a
//! ten-minute fix and an afternoon.
//!
//! `#[ignore]` because it writes to the repository. The fixture is committed; regenerating it
//! should be a decision.

use std::collections::BTreeSet;
use std::path::Path;

use smysl_core::cbor::envelope::unit_core_bytes;
use smysl_core::ids::KernelType;
use smysl_core::{
    canonical_uid, Date, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn uid_of(n: u8) -> Uid {
    Uid::from_bytes([n; 32])
}

/// A named case and the core it describes. The name is what a failing assertion prints.
fn cases() -> Vec<(&'static str, UnitCore)> {
    let minimal = UnitCoreBuilder::new(KernelType::Claim, "a minimal claim", Status::Speculative)
        .build()
        .unwrap();

    // The §2.3 pair: identical in every field except status.
    let status_a = UnitCoreBuilder::new(
        KernelType::Claim,
        "the same words either way",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let status_b = UnitCoreBuilder::new(
        KernelType::Claim,
        "the same words either way",
        Status::Inferred,
    )
    .grounds([uid_of(1)])
    .build()
    .unwrap();

    let with_body = UnitCoreBuilder::new(
        KernelType::Finding,
        "a finding with a body",
        Status::Speculative,
    )
    .body("the body text, which is a separate field")
    .build()
    .unwrap();

    let with_detail = UnitCoreBuilder::new(
        KernelType::Finding,
        "a finding with detail",
        Status::Speculative,
    )
    .body("the body text")
    .detail("the detail text, longer than the body and pinned at L2")
    .build()
    .unwrap();

    let with_refs = UnitCoreBuilder::new(
        KernelType::Claim,
        "a claim resting on two grounds",
        Status::Inferred,
    )
    .deps([uid_of(7), uid_of(3)])
    .grounds([uid_of(9), uid_of(1)])
    .build()
    .unwrap();

    let with_source = UnitCoreBuilder::new(KernelType::Evidence, "a measurement", Status::Measured)
        .source(SourceRef::new(
            SourceKind::Metric,
            "pool.wait_ms{shard=eu-west}",
        ))
        .build()
        .unwrap();

    let with_captured = UnitCoreBuilder::new(
        KernelType::Evidence,
        "a dated measurement",
        Status::Measured,
    )
    .source(
        SourceRef::new(SourceKind::File, "analysis/tables/latency.csv")
            .captured_on(Date::new(2026, 6, 30).unwrap()),
    )
    .build()
    .unwrap();

    // Text that is not ASCII, and text that is not NFC on the way in. The second must produce
    // the same uid as its composed twin — normalisation is part of identity, not presentation.
    let unicode = UnitCoreBuilder::new(
        KernelType::Claim,
        "caf\u{e9} latency, \u{4e2d}\u{6587}",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let decomposed = UnitCoreBuilder::new(
        KernelType::Claim,
        "cafe\u{301} latency, \u{4e2d}\u{6587}",
        Status::Speculative,
    )
    .build()
    .unwrap();

    let with_payload = {
        let mut b = UnitCoreBuilder::new(
            KernelType::Observation,
            "carries a payload",
            Status::Speculative,
        );
        b.payload = Some(vec![0xA1, 0x61, b'k', 0x01]); // {"k": 1}
        b.build().unwrap()
    };

    let every_status: Vec<(&'static str, UnitCore)> = vec![
        ("status-speculative", {
            UnitCoreBuilder::new(
                KernelType::Claim,
                "one gist, six statuses",
                Status::Speculative,
            )
            .build()
            .unwrap()
        }),
        ("status-inferred", {
            UnitCoreBuilder::new(
                KernelType::Claim,
                "one gist, six statuses",
                Status::Inferred,
            )
            .grounds([uid_of(1)])
            .build()
            .unwrap()
        }),
        ("status-derived", {
            UnitCoreBuilder::new(KernelType::Claim, "one gist, six statuses", Status::Derived)
                .grounds([uid_of(1)])
                .build()
                .unwrap()
        }),
        ("status-cited", {
            UnitCoreBuilder::new(KernelType::Claim, "one gist, six statuses", Status::Cited)
                .source(SourceRef::new(SourceKind::Doc, "a-document"))
                .build()
                .unwrap()
        }),
        ("status-measured", {
            UnitCoreBuilder::new(
                KernelType::Claim,
                "one gist, six statuses",
                Status::Measured,
            )
            .source(SourceRef::new(SourceKind::Doc, "a-document"))
            .build()
            .unwrap()
        }),
    ];

    let mut v: Vec<(&'static str, UnitCore)> = vec![
        ("minimal", minimal),
        ("status-pair-a", status_a),
        ("status-pair-b", status_b),
        ("with-body", with_body),
        ("with-detail", with_detail),
        ("with-refs", with_refs),
        ("with-source", with_source),
        ("with-captured", with_captured),
        ("unicode-composed", unicode),
        ("unicode-decomposed", decomposed),
        ("with-payload", with_payload),
    ];
    v.extend(every_status);
    v
}

fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn uids_json(set: &BTreeSet<Uid>) -> String {
    let items: Vec<String> = set
        .iter()
        .map(|u| json_string(&hex(u.as_bytes())))
        .collect();
    format!("[{}]", items.join(", "))
}

#[test]
#[ignore = "writes into the repository; regenerating should be a decision"]
fn emit() {
    let mut out = String::new();
    out.push_str("{\n");
    out.push_str("  \"purpose\": \"Unit cores, their canonical bytes and their uids, produced by the Rust. A second implementation at C-Produce must reproduce every uid. `core_bytes_hex` is included so a mismatch says whether the encoding or the hash disagreed.\",\n");
    out.push_str("  \"cases\": [\n");

    let all = cases();
    for (i, (name, core)) in all.iter().enumerate() {
        let bytes = unit_core_bytes(core);
        let u = canonical_uid(core);
        out.push_str("    {\n");
        out.push_str(&format!("      \"name\": {},\n", json_string(name)));
        out.push_str("      \"core\": {\n");
        out.push_str(&format!(
            "        \"schema\": {},\n",
            json_string(core.schema.as_str())
        ));
        out.push_str(&format!("        \"gist\": {},\n", json_string(&core.gist)));
        out.push_str(&format!(
            "        \"body\": {},\n",
            core.body
                .as_deref()
                .map(json_string)
                .unwrap_or("null".into())
        ));
        out.push_str(&format!(
            "        \"detail\": {},\n",
            core.detail
                .as_deref()
                .map(json_string)
                .unwrap_or("null".into())
        ));
        out.push_str(&format!("        \"deps\": {},\n", uids_json(&core.deps)));
        out.push_str(&format!(
            "        \"grounds\": {},\n",
            uids_json(&core.grounds)
        ));
        out.push_str(&format!("        \"status\": {},\n", core.status.as_u8()));
        match &core.source {
            None => out.push_str("        \"source\": null,\n"),
            Some(s) => {
                out.push_str("        \"source\": { ");
                out.push_str(&format!("\"kind\": {}, ", s.kind.as_u8()));
                out.push_str(&format!("\"reference\": {}, ", json_string(&s.reference)));
                match &s.captured {
                    None => out.push_str("\"captured\": null "),
                    Some(d) => {
                        out.push_str(&format!("\"captured\": {} ", json_string(&d.to_string())))
                    }
                }
                out.push_str("},\n");
            }
        }
        out.push_str(&format!(
            "        \"payload_hex\": {}\n",
            core.payload
                .as_deref()
                .map(|p| json_string(&hex(p)))
                .unwrap_or("null".into())
        ));
        out.push_str("      },\n");
        out.push_str(&format!(
            "      \"core_bytes_hex\": {},\n",
            json_string(&hex(&bytes))
        ));
        out.push_str(&format!(
            "      \"uid_hex\": {}\n",
            json_string(&hex(u.as_bytes()))
        ));
        out.push_str(if i + 1 == all.len() {
            "    }\n"
        } else {
            "    },\n"
        });
    }
    out.push_str("  ]\n}\n");

    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/uid");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("cases.json"), out).unwrap();
    println!("wrote {} cases", all.len());
}
