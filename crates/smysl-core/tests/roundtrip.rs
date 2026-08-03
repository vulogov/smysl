//! The SM-P1 gate: `encode -> decode -> encode` is byte-stable, and every conforming
//! record survives the trip unchanged.
//!
//! These are integration tests rather than unit tests on purpose - they exercise the
//! crate exactly as an embedder does, through its public API.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::cbor::envelope::unit_core_bytes;
use smysl_core::{
    canonical_uid, from_cbor, from_cbor_seq, to_cbor, to_cbor_seq, AgentId, Attestation,
    Contention, ContentionId, ContentionStatus, Date, Detected, DetectionKind, DropReason,
    GranularityProfile, Hlc, KernelType, LangTag, Lod, Op, Optimality, PackInfo, PackMode, Record,
    RelKind, Relation, Role, Rung, SchemaDecl, SchemaId, SourceKind, SourceRef, Status, Step,
    Thread, ThreadId, ThreadSchema, Uid, UnitCore, UnitCoreBuilder, View, ViewId,
};

fn uid(n: u8) -> Uid {
    Uid::from_bytes([n; 32])
}

fn agent() -> AgentId {
    AgentId::new("model:anthropic/claude-opus-5").unwrap()
}

fn hlc() -> Hlc {
    Hlc::new(1_752_000_000_000, 3, agent())
}

/// One of each record type, exercising every optional field in at least one direction.
fn corpus() -> Vec<Record> {
    let minimal: UnitCore =
        UnitCoreBuilder::new(KernelType::Claim, "a bare claim", Status::Speculative)
            .build()
            .unwrap();

    let maximal: UnitCore = UnitCoreBuilder::new(
        SchemaId::parse("x.sre/incident").unwrap(),
        "p95 auth latency tripled",
        Status::Cited,
    )
    .body("Between 2026-07-02 and 2026-07-09, p95 rose from 180 ms to 540 ms.")
    .detail("Per-shard breakdown at hourly resolution, and the canary result.")
    .deps([uid(1), uid(2)])
    .grounds([uid(3)])
    .source(
        SourceRef::new(SourceKind::Metric, "grafana://board/12/panel/4")
            .captured_on(Date::new(2026, 7, 9).unwrap()),
    )
    .payload(vec![0xA1, 0x00, 0x01])
    .build()
    .unwrap();

    let attestation = Attestation::new(uid(7), agent(), Op::Transformed, Rung::Model, hlc())
        .at_hop(3)
        .with_parents([uid(4), uid(5)].into_iter().collect())
        .with_recipe([0xAB; 32], [0xCD; 32]);

    let relation = Relation::new(RelKind::Rebuts, uid(8), uid(9))
        .with_weight(0.6)
        .with_note(uid(10));

    let ext_relation = Relation::new(RelKind::parse("x.sre/mitigates").unwrap(), uid(1), uid(2));

    let thread = Thread::new(
        ThreadId::new("t/brief").unwrap(),
        ThreadSchema::Brief,
        agent(),
        "Auth p95 tripled in eu-west",
        hlc(),
    )
    .with_steps([
        Step::new(Role::BottomLine, uid(1)),
        Step::new(Role::Support, uid(2)).with_note("the pool metrics"),
        Step::new(Role::Risk, uid(3)),
    ]);

    let view = View::new(ViewId::new("v/incident").unwrap(), "incident-brief")
        .with_roots([uid(1), uid(2)])
        .with_threads([
            ThreadId::new("t/brief").unwrap(),
            ThreadId::new("t/analysis").unwrap(),
        ])
        .requiring([
            SchemaId::parse("x.sre/1").unwrap(),
            SchemaId::from(KernelType::Claim),
        ])
        .with_granularity(GranularityProfile::fine())
        .with_lang(LangTag::new("en-GB").unwrap());

    let contention = Contention::new(
        ContentionId::new("k/pool-vs-index").unwrap(),
        uid(1),
        vec![uid(2), uid(3)],
        Detected {
            kind: DetectionKind::SupersessionFork,
            ts: hlc(),
        },
    );

    let mut packinfo = PackInfo::new(8_000, 6_142, "smysl/utf8-div4");
    packinfo.thread = Some(ThreadId::new("t/brief").unwrap());
    packinfo.dropped = vec![(uid(4), DropReason::Budget), (uid(5), DropReason::LowValue)];
    packinfo.degraded = vec![(uid(6), Lod::L0)];
    packinfo.optimality = Optimality {
        mode: PackMode::Exact,
        gap: 0.25,
    };

    let mut decl = SchemaDecl::new(SchemaId::parse("x.sre/1").unwrap(), 1);
    decl.types = vec![SchemaId::parse("x.sre/incident").unwrap()];
    decl.relations = vec![RelKind::Rebuts, RelKind::parse("x.sre/mitigates").unwrap()];
    decl.payload_shape = Some(vec![0xA0]);

    vec![
        Record::Unit(minimal),
        Record::Unit(maximal),
        Record::Attestation(attestation),
        Record::Relation(relation),
        Record::Relation(ext_relation),
        Record::Thread(thread),
        Record::View(view),
        Record::Contention(contention),
        Record::PackInfo(packinfo),
        Record::SchemaDecl(decl),
        Record::Unknown {
            code: 99,
            payload: vec![0xA1, 0x00, 0x01],
        },
    ]
}

#[test]
fn every_record_type_survives_a_round_trip() {
    for r in corpus() {
        let bytes = to_cbor(&r);
        let (back, n) = from_cbor(&bytes).unwrap_or_else(|e| panic!("{}: {e}", r.type_name()));
        assert_eq!(n, bytes.len(), "{}: trailing bytes", r.type_name());
        assert_eq!(back, r, "{} did not survive the round trip", r.type_name());
    }
}

#[test]
fn encoding_is_byte_stable_across_a_round_trip() {
    for r in corpus() {
        let a = to_cbor(&r);
        let (back, _) = from_cbor(&a).unwrap();
        let b = to_cbor(&back);
        assert_eq!(a, b, "{}: re-encoding changed the bytes", r.type_name());
    }
}

#[test]
fn encoding_is_deterministic_across_repeated_calls() {
    let c = corpus();
    assert_eq!(to_cbor_seq(&c), to_cbor_seq(&c));
}

#[test]
fn a_whole_store_round_trips_as_a_cbor_sequence() {
    let c = corpus();
    let bytes = to_cbor_seq(&c);
    let (back, off) = from_cbor_seq(&bytes).unwrap();
    assert_eq!(off, bytes.len());
    assert_eq!(back, c);
}

/// The log is append-only and may be read while a writer is mid-append, so a truncated
/// tail must yield everything up to the last complete record rather than an error (§7.3).
#[test]
fn a_truncated_sequence_parses_up_to_the_last_complete_record() {
    let c = corpus();
    let bytes = to_cbor_seq(&c);
    let first = to_cbor(&c[0]).len();
    let second = to_cbor(&c[1]).len();

    for cut in 1..second {
        let (back, off) = from_cbor_seq(&bytes[..first + cut]).unwrap();
        assert_eq!(back.len(), 1, "cut {cut} should leave exactly one record");
        assert_eq!(off, first);
    }
}

#[test]
fn an_empty_sequence_is_an_empty_store() {
    let (back, off) = from_cbor_seq(&[]).unwrap();
    assert!(back.is_empty());
    assert_eq!(off, 0);
}

/// `SMY-W014`: an unknown record type is forward compatibility, not corruption. Its
/// payload bytes must come back out exactly as they went in.
#[test]
fn unknown_record_types_are_preserved_verbatim() {
    let payload = vec![0xA2, 0x00, 0x63, b'f', b'o', b'o', 0x01, 0x18, 0xFF];
    let r = Record::Unknown {
        code: 12_345,
        payload: payload.clone(),
    };
    let bytes = to_cbor(&r);
    let (back, _) = from_cbor(&bytes).unwrap();
    match back {
        Record::Unknown { code, payload: p } => {
            assert_eq!(code, 12_345);
            assert_eq!(p, payload);
        }
        other => panic!("expected Unknown, got {}", other.type_name()),
    }
}

/// The checkpoint code is reserved but unimplemented (D-11). It must behave like any
/// other unknown type rather than being special-cased into an error.
#[test]
fn the_reserved_checkpoint_code_decodes_as_unknown() {
    let r = Record::Unknown {
        code: 9,
        payload: vec![0xA0],
    };
    let (back, _) = from_cbor(&to_cbor(&r)).unwrap();
    assert_eq!(back, r);
}

/// An unknown record must still be *well-formed*: it cannot smuggle a non-deterministic
/// encoding past the reader just by claiming a type nobody knows.
#[test]
fn an_unknown_record_with_a_bad_payload_is_still_rejected() {
    // [99, indefinite-length map]
    let bytes = [0x82, 0x18, 0x63, 0xBF, 0xFF];
    assert!(from_cbor(&bytes).is_err());
}

/// Unknown *keys* inside a known record type must survive too, or decoding and
/// re-encoding a store written by a later minor version would change every uid in it.
#[test]
fn unknown_map_keys_survive_and_do_not_disturb_the_uid() {
    let core = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
        .build()
        .unwrap();
    let mut with_extra = core.clone();
    with_extra.extra = BTreeMap::from([(42u16, vec![0x18, 0xFF])]);

    let bytes = to_cbor(&Record::Unit(with_extra.clone()));
    let (back, _) = from_cbor(&bytes).unwrap();
    let decoded = back.as_unit().unwrap();

    assert_eq!(decoded.extra.get(&42).unwrap(), &vec![0x18, 0xFF]);
    assert_eq!(
        to_cbor(&back),
        bytes,
        "re-encoding must reproduce the bytes"
    );
    assert_ne!(
        canonical_uid(decoded),
        canonical_uid(&core),
        "an extra hashed key is extra content, so it is a different unit"
    );
}

#[test]
fn extra_keys_are_re_emitted_in_key_order() {
    let mut core = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
        .build()
        .unwrap();
    core.extra = BTreeMap::from([(60u16, vec![0x01]), (20u16, vec![0x02])]);
    let bytes = unit_core_bytes(&core);
    let twenty = bytes.windows(2).position(|w| w == [20, 0x02]).unwrap();
    let sixty = bytes.windows(2).position(|w| w == [60, 0x01]).unwrap();
    assert!(twenty < sixty, "map keys must ascend");
}

#[test]
fn the_uid_is_stable_across_encode_decode() {
    for r in corpus() {
        if let Record::Unit(u) = &r {
            let before = canonical_uid(u);
            let (back, _) = from_cbor(&to_cbor(&r)).unwrap();
            assert_eq!(canonical_uid(back.as_unit().unwrap()), before);
        }
    }
}

/// Sets are encoded sorted, so the order they were built in cannot leak into the bytes -
/// and therefore cannot leak into a uid.
#[test]
fn set_insertion_order_does_not_affect_the_encoding() {
    let forward: BTreeSet<Uid> = [uid(1), uid(2), uid(3)].into_iter().collect();
    let backward: BTreeSet<Uid> = [uid(3), uid(2), uid(1)].into_iter().collect();

    let a = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
        .deps(forward)
        .build()
        .unwrap();
    let b = UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
        .deps(backward)
        .build()
        .unwrap();
    assert_eq!(unit_core_bytes(&a), unit_core_bytes(&b));
    assert_eq!(canonical_uid(&a), canonical_uid(&b));
}

/// Weight is quantised before encoding, so two floats within one quantum are one edge.
#[test]
fn relation_weights_are_quantised_on_the_wire() {
    let a = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6);
    let b = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6 + 1e-7);
    assert_eq!(to_cbor(&Record::Relation(a)), to_cbor(&Record::Relation(b)));
}

#[test]
fn a_decoded_weight_is_already_quantised() {
    let r = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6);
    let (back, _) = from_cbor(&to_cbor(&Record::Relation(r))).unwrap();
    match back {
        Record::Relation(rel) => assert_eq!(rel.weight, Some(614.0 / 1024.0)),
        other => panic!("expected a relation, got {}", other.type_name()),
    }
}

/// Extension relation kinds go on the wire as text, kernel kinds as a code. Both must
/// come back as the same kind.
#[test]
fn relation_kinds_round_trip_in_both_encodings() {
    for kind in RelKind::KERNEL
        .iter()
        .cloned()
        .chain([RelKind::parse("x.sre/mitigates").unwrap()])
    {
        let r = Relation::new(kind.clone(), uid(1), uid(2));
        let (back, _) = from_cbor(&to_cbor(&Record::Relation(r))).unwrap();
        match back {
            Record::Relation(rel) => assert_eq!(rel.kind, kind),
            other => panic!("expected a relation, got {}", other.type_name()),
        }
    }
}

#[test]
fn every_status_and_every_kernel_type_round_trips() {
    for &status in Status::ALL {
        if status == Status::Unfounded {
            continue; // unauthorable by construction
        }
        for &ty in KernelType::ALL {
            let mut b = UnitCoreBuilder::new(ty, "g", status);
            if status.requires_grounds() {
                b = b.grounds([uid(1)]);
            }
            if status.requires_source() {
                b = b.source(SourceRef::new(SourceKind::Doc, "x"));
            }
            let core = b.build().unwrap();
            let (back, _) = from_cbor(&to_cbor(&Record::Unit(core.clone()))).unwrap();
            assert_eq!(back.as_unit(), Some(&core), "{ty} at {status}");
        }
    }
}

#[test]
fn every_thread_schema_and_role_round_trips() {
    for &schema in ThreadSchema::ALL {
        let steps: Vec<Step> = schema
            .roles()
            .iter()
            .enumerate()
            .map(|(i, &r)| Step::new(r, uid(i as u8)))
            .collect();
        let t = Thread::new(ThreadId::new("t/x").unwrap(), schema, agent(), "g", hlc())
            .with_steps(steps);
        let (back, _) = from_cbor(&to_cbor(&Record::Thread(t.clone()))).unwrap();
        assert_eq!(back, Record::Thread(t), "{schema}");
    }
}

#[test]
fn every_contention_status_and_detection_kind_round_trips() {
    for &kind in DetectionKind::ALL {
        for &status in ContentionStatus::ALL {
            let mut c = Contention::new(
                ContentionId::new("k/x").unwrap(),
                uid(1),
                vec![uid(2)],
                Detected { kind, ts: hlc() },
            );
            c.status = status;
            let (back, _) = from_cbor(&to_cbor(&Record::Contention(c.clone()))).unwrap();
            assert_eq!(back, Record::Contention(c));
        }
    }
}

/// The presets are not enough, and mutation testing said so.
///
/// `every_granularity_preset_round_trips` iterates three profiles — and all three carry
/// `l0_max: 30`. Deleting the decoder's `L0_MAX` arm, so the field fell through to `extra` and
/// the profile took its default, changed nothing any test could see. A loop over variants that
/// do not vary in the field under test is the shape this project keeps finding.
#[test]
fn a_granularity_profile_round_trips_values_no_preset_uses() {
    let mut g = GranularityProfile::standard();
    g.l0_max = 47;
    g.l1_min = 51;
    g.l1_max = 149;
    g.profile = "custom".into();

    let v = View::new(ViewId::new("v/x").unwrap(), "i").with_granularity(g.clone());
    let (back, _) = from_cbor(&to_cbor(&Record::View(v))).unwrap();
    match back {
        Record::View(view) => {
            assert_eq!(view.granularity.l0_max, 47, "l0_max did not survive");
            assert_eq!(view.granularity.l1_min, 51);
            assert_eq!(view.granularity.l1_max, 149);
            assert_eq!(view.granularity, g);
        }
        other => panic!("expected a view, got {}", other.type_name()),
    }
}

/// `sig` is reserved and unimplemented (N9) — and it decodes, so it is testable, and was not
/// tested. Deleting its decoder arm sent a signature into `extra`: preserved verbatim, so the
/// bytes and the uid are unaffected, and `Attestation::sig` silently `None`. Whatever
/// eventually verifies signatures would have found none and said the record was unsigned.
#[test]
fn an_attestation_signature_round_trips_as_a_signature() {
    let mut a = Attestation::new(uid(7), agent(), Op::Transformed, Rung::Model, hlc());
    a.sig = Some(vec![0xD9, 0xD2, 0x84, 0x01, 0x02, 0x03]);

    let (back, _) = from_cbor(&to_cbor(&Record::Attestation(a.clone()))).unwrap();
    match back {
        Record::Attestation(got) => {
            assert_eq!(got.sig, a.sig, "the signature must come back as `sig`");
            assert!(
                got.extra.is_empty(),
                "and not as a preserved unknown key, which would read as unsigned"
            );
        }
        other => panic!("expected an attestation, got {}", other.type_name()),
    }
}

#[test]
fn every_granularity_preset_round_trips() {
    for g in [
        GranularityProfile::coarse(),
        GranularityProfile::standard(),
        GranularityProfile::fine(),
    ] {
        let v = View::new(ViewId::new("v/x").unwrap(), "i").with_granularity(g.clone());
        let (back, _) = from_cbor(&to_cbor(&Record::View(v))).unwrap();
        match back {
            Record::View(view) => assert_eq!(view.granularity, g),
            other => panic!("expected a view, got {}", other.type_name()),
        }
    }
}

/// A gist-only unit is the normal shape of imported summary material, and it must not
/// gain phantom fields on the way through.
#[test]
fn optional_fields_stay_absent() {
    let core = UnitCoreBuilder::new(KernelType::Prose, "just a gist", Status::Speculative)
        .build()
        .unwrap();
    let bytes = unit_core_bytes(&core);
    assert_eq!(bytes[0], 0xA3, "schema, gist, status and nothing else");
    assert!(!bytes.contains(&0xF6), "no null anywhere");

    let (back, _) = from_cbor(&to_cbor(&Record::Unit(core))).unwrap();
    let u = back.as_unit().unwrap();
    assert!(u.body.is_none() && u.detail.is_none() && u.source.is_none());
    assert!(u.deps.is_empty() && u.grounds.is_empty() && u.payload.is_none());
}

/// The golden encoding of the corpus.
///
/// A diff here is never noise: nothing in this file is produced by a model, so a change
/// means the wire format moved. If the move is intended, re-bless with
/// `SMYSL_BLESS=1 cargo test -p smysl-core --test roundtrip` and review the diff - it is
/// the evidence that the change did what it claimed.
#[test]
fn the_corpus_encoding_matches_the_golden_file() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/golden/cbor/corpus.cbor");
    let bytes = to_cbor_seq(&corpus());

    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        return;
    }

    let golden = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nrun with SMYSL_BLESS=1 to create it",
            path.display()
        )
    });
    assert_eq!(
        bytes.len(),
        golden.len(),
        "the canonical encoding changed length"
    );
    assert_eq!(
        bytes, golden,
        "the canonical encoding of the corpus changed"
    );
}

/// A record the decoder accepts must re-encode to the exact bytes it came from.
///
/// `dec_packinfo` defaulted `budget`, `used`, `optimality` and `estimator` when they were
/// absent, while the encoder writes all four unconditionally. So `[7, {0: 0}]` decoded fine
/// and re-encoded as a four-key map: two distinct byte strings mapping to one record, which
/// is precisely what stops a uid from being an identity. Found by fuzzing.
#[test]
fn a_pack_info_missing_a_mandatory_field_is_rejected_not_defaulted() {
    // array(2), 7 (PackInfo), map(1) { 0: 0 } — budget alone.
    let truncated = [0x82u8, 0x07, 0xA1, 0x00, 0x00];
    assert!(
        smysl_core::from_cbor(&truncated).is_err(),
        "a PackInfo missing three mandatory fields was accepted and silently completed"
    );

    // The invariant itself, swept over every record kind and every low key. Several value
    // shapes, because a decoder is only reachable through the ones its fields accept — an
    // earlier version of this sweep used integers alone, so `dec_view` (whose key 0 is a
    // text id) was never entered and its identical defect survived another fuzz run.
    // A probe value only tests a decoder it can actually get *into*. `"a/b"` is not a valid
    // `SchemaId`, so every schema-declaration case was rejected at key 0 and this sweep
    // reported the class clean while `dec_schema_decl` carried the identical defect — the
    // third time that has happened. A parseable kernel schema is here for exactly that
    // reason, and any new decoder guarding key 0 with a parser needs a value that satisfies
    // it or this sweep silently skips the whole record type.
    // `0x70` is text(16), and `smysl.kernel/0.1` is sixteen bytes. Getting that header
    // wrong is how the first attempt at this fix stayed as vacuous as the sweep it was
    // meant to repair: the value failed to decode, so the record type was skipped again.
    const KERNEL: &[u8] = b"\x70smysl.kernel/0.1";
    let values: [&[u8]; 6] = [
        &[0x00],                   // 0
        &[0x60],                   // ""
        &[0x80],                   // []
        &[0xF6],                   // null
        &[0x63, b'a', b'/', b'b'], // "a/b", a plausible label
        KERNEL,                    // a schema id that actually parses
    ];
    for code in 0u8..=12 {
        for key in 0u8..=9 {
            for v in values {
                let mut bytes = vec![0x82, code, 0xA1, key];
                bytes.extend_from_slice(v);
                if let Ok((r, n)) = smysl_core::from_cbor(&bytes) {
                    assert_eq!(
                        smysl_core::to_cbor(&r),
                        &bytes[..n],
                        "record code {code} with only key {key} = {v:?} did not re-encode \
                         to itself; the decoder supplied a default the encoder always writes"
                    );
                }
            }
        }
    }
}

/// Unicode form never reaches a uid — for *every* text field, not just a unit's gist.
///
/// The encoder used to assert NFC in debug and trust it in release, on the grounds that
/// constructors establish it. They establish it for a unit's gist, body and detail, and for
/// nothing else: a thread's gist, a step's note, a view's intent, a granularity profile, a
/// source reference and a pack estimator all reach the encoder unchecked. Two of those were
/// found by fuzzing in two separate releases, each fixed by normalising in one more
/// constructor — which is a class of defect being treated as a list of defects.
///
/// The encoder normalises now, so this asserts the property the format spec promises rather
/// than the discipline of whoever writes the next constructor.
#[test]
fn unicode_form_never_reaches_a_uid() {
    use smysl_core::{AgentId, Hlc, Record, Thread, ThreadId, ThreadSchema};

    // U+0301 (composed) and U+0341 (which NFC folds to U+0301) are the same text.
    let mk = |mark: char| {
        let mut gist = String::from("caf");
        gist.push('e');
        gist.push(mark);
        Record::Thread(Thread::new(
            ThreadId::new("t/x").unwrap(),
            ThreadSchema::Brief,
            AgentId::new("tool:test").unwrap(),
            gist,
            Hlc::new(0, 0, AgentId::new("tool:test").unwrap()),
        ))
    };

    let composed = smysl_core::to_cbor(&mk('\u{301}'));
    let folded = smysl_core::to_cbor(&mk('\u{341}'));
    assert_eq!(
        composed, folded,
        "two Unicode forms of one thread gist encoded to different bytes, so they would \
         carry different identities"
    );

    // And what comes back is the normalised form, so a decode/encode cycle is stable.
    let (r, n) = smysl_core::from_cbor(&folded).expect("decodes");
    assert_eq!(n, folded.len());
    assert_eq!(smysl_core::to_cbor(&r), folded);
}
