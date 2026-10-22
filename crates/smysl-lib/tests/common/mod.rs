//! What the library tests share: the corpus, the five-Bible library and the two chats.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, from_cbor_seq, AgentId, Attestation, Carry, Commit, Commitment, Dating,
    DatingTarget, Hlc, KernelType, LangTag, Op, Record, Redaction, RelKind, Relation, Resolution,
    ResolutionTarget, Rung, SourceKind, SourceRef, Status, Uid, UnitCoreBuilder, Withdrawal,
};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as Bytes;
use smysl_text::{AddSpec, Library};

pub const EDITIONS: [(&str, &str, &str); 5] = [
    ("deu1912", "deu1912", "de"),
    ("eng-kjv", "eng-kjv", "en"),
    ("fraLSG", "fralsg", "fr"),
    ("russyn", "russyn", "ru"),
    ("spaRV1909", "sparv1909", "es"),
];
pub const BOOKS: [&str; 2] = ["08-RUT", "65-JUD"];

/// 2024-01-15T00:00:00Z.
pub const BASE: u64 = 1_705_276_800_000;
pub const HOUR: u64 = 3_600_000;

pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A fresh directory under the test target's scratch space.
pub fn dir(group: &str, name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(group)
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

pub fn agent() -> AgentId {
    AgentId::new("tool:test").unwrap()
}

pub fn at(ms: u64) -> Hlc {
    Hlc::new(ms, 0, agent())
}

pub fn attest(uid: Uid, ms: u64) -> Record {
    Record::Attestation(Attestation::new(
        uid,
        agent(),
        Op::Authored,
        Rung::Document,
        at(ms),
    ))
}

/// The records of a corpus fixture, surface or CBOR.
pub fn fixture(rel: &str) -> Vec<Record> {
    let path = repo().join(rel);
    if rel.ends_with(".cbor") {
        from_cbor_seq(&std::fs::read(&path).unwrap()).unwrap().0
    } else {
        parse_surface(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .records
    }
}

/// F1–F13, F10 from the wire, and F14's unknown codes.
pub fn corpus() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo().join("fixtures/corpus"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".smy"))
        .map(|n| format!("fixtures/corpus/{n}"))
        .collect();
    out.push("fixtures/wire/F10-lifecycle.cbor".to_string());
    out.push("fixtures/wire/F14-unknown-codes.cbor".to_string());
    out.sort();
    out
}

/// The ten books, `<edition>/<book>`, so each edition is a collection; then one unit per part.
pub fn bibles(at_dir: &Path, carry: Carry) -> Library {
    let mut lib = Library::create(at_dir).unwrap();
    let mut units = Vec::new();
    for (folder, key, lang) in EDITIONS {
        for book in BOOKS {
            let file = repo().join(format!(
                "fixtures/library/bibles/slice/{folder}/{book}.usfm"
            ));
            let bytes = std::fs::read(&file).unwrap();
            let alias = format!("{key}/{}", book.to_ascii_lowercase());
            let licence = if carry == Carry::Text {
                "public-domain"
            } else {
                "unknown"
            };
            let spec = AddSpec::new("usfm/1", alias.clone(), licence)
                .with_lang(LangTag::new(lang).unwrap())
                .with_carry(carry);
            let added = lib.add(&Bytes::new(&bytes), &spec, &Caps::DEFAULT).unwrap();
            let m = lib.store().manifest(&added.mid).unwrap().clone();
            for (i, e) in m.parts.iter().enumerate() {
                let source = SourceRef::new(SourceKind::Doc, e.tid.canonical())
                    .in_part(0, e.length.min(64), added.mid)
                    .unwrap();
                let core = UnitCoreBuilder::new(
                    KernelType::Claim,
                    format!("the opening of {alias}, part {i}"),
                    Status::Speculative,
                )
                .lang(LangTag::new(lang).unwrap())
                .source(source)
                .build()
                .unwrap();
                let uid = canonical_uid(&core);
                units.push(Record::Unit(core));
                units.push(Record::Attestation(Attestation::new(
                    uid,
                    agent(),
                    Op::Authored,
                    Rung::Document,
                    Hlc::new(1, 0, agent()),
                )));
            }
        }
    }
    lib.append_records(&units).unwrap();
    lib
}

/// The two chats, a unit per part observed six hours apart, and the lifecycle records.
pub fn chats(at_dir: &Path) -> (Library, Vec<Uid>) {
    let mut lib = Library::create(at_dir).unwrap();
    let mut records = Vec::new();
    let mut uids = Vec::new();
    let mut t = BASE;
    let params: [&[(&str, &str)]; 2] = [&[("date-format", "dmy")], &[]];
    for ((file, reader, alias), pairs) in [
        ("whatsapp.txt", "whatsapp/1", "chat/wa"),
        ("telegram.json", "telegram/1", "chat/tg"),
    ]
    .into_iter()
    .zip(params)
    {
        let bytes = std::fs::read(repo().join("fixtures/library/readers").join(file)).unwrap();
        let mut spec = AddSpec::new(reader, alias, "unknown")
            .with_lang(LangTag::new("en").unwrap())
            .with_carry(Carry::Ref);
        for (k, v) in pairs {
            spec.params.set(reader, *k, *v).unwrap();
        }
        let added = lib.add(&Bytes::new(&bytes), &spec, &Caps::DEFAULT).unwrap();
        let m = lib.store().manifest(&added.mid).unwrap().clone();
        let spans = m.parts.iter().flat_map(|e| {
            (0..6u64).filter_map(move |k| (e.length >= 4 * (k + 1)).then_some((e, k)))
        });
        for (i, (e, k)) in spans.enumerate() {
            let source = SourceRef::new(SourceKind::Doc, e.tid.canonical())
                .in_part(4 * k, 4 * (k + 1), added.mid)
                .unwrap()
                .observed_at(t);
            let core = UnitCoreBuilder::new(
                KernelType::Claim,
                format!("{alias} says something, part {i}"),
                Status::Speculative,
            )
            .source(source)
            .build()
            .unwrap();
            let uid = canonical_uid(&core);
            records.push(Record::Unit(core));
            records.push(attest(uid, t));
            uids.push(uid);
            t += 6 * HOUR;
        }
    }
    // An edge between the first two, withdrawn and the withdrawal resolved; a commitment on
    // the first; a redaction of the first chat's first part.
    let rel = Relation::new(RelKind::Elaborates, uids[0], uids[1]);
    let rid = rel.uid();
    records.push(Record::Relation(rel));
    records.push(attest(rid, BASE + HOUR));
    records.push(Record::Withdrawal(Withdrawal::new(
        rid,
        agent(),
        at(BASE + 2 * HOUR),
    )));
    records.push(Record::Resolution(Resolution::new(
        ResolutionTarget::Relation(rid),
        agent(),
        at(BASE + 3 * HOUR),
    )));
    records.push(Record::Commit(Commit::new(
        uids[0],
        Commitment::Drafted,
        agent(),
        at(BASE + HOUR),
    )));
    records.push(Record::Dating(Dating::offset(
        DatingTarget::Unit(uids[1]),
        HOUR as i64,
        agent(),
        at(BASE + HOUR),
    )));
    let first_tid = lib.store().manifests().next().unwrap().1.parts[0].tid;
    records.push(Record::Redaction(Redaction::new(
        first_tid,
        agent(),
        at(BASE + HOUR),
    )));
    lib.append_records(&records).unwrap();
    (lib, uids)
}
