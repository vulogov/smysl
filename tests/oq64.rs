//! OQ-64's measurement and decision (SMYSL-2.7 §9): threads and pack infos whose steps span
//! files, over the chat corpora.
//!
//! The corpora are the three chat samples of `fixtures/library/readers`, WhatsApp, Telegram
//! and Slack: two days of one conversation each. A unit is drawn per message row, Telegram's
//! `reply` ids become `x.text/replies` edges as `thread_schemas.rs` draws them, and each chat
//! gets a `dialogue` thread with a pack info naming it. One more thread runs across all three,
//! the shape a thread derived over a whole library takes. Every layout under both closures is
//! exported with `--with-packs`, and the test prints, per cell, what W490 counted and which
//! threads no file holds.
//!
//! What it asserts is the decision. Under `full` a thread is in a file only with all its steps,
//! so `by-time` loses every chat's thread and `by-expression` the cross-chat one, each counted,
//! with its pack info. Under `ref` none is lost: the file owning the first step holds it and
//! requires every file holding another. Every cell verifies, and P1 holds through a real import.

#![cfg(feature = "cli")]

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    canonical_uid, AgentId, Carry, Hlc, KernelType, PackInfo, Record, RelKind, Relation, Status,
    Thread, ThreadId, ThreadSchema, Uid, UnitCoreBuilder,
};
use smysl_lib::{
    export_library, import, retained, snapshot, verify_collection, write_collection, Closure,
    ExportOptions, ImportOptions, Input as ImportInput, Layout, Selection, With, INDEX_FILE,
};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;
use smysl_text::{locator, provenance, AddSpec, LangTag, Library};
use smysl_thread::{derive_thread, DeriveOptions};

/// A chat sample: its file, its reader, the alias it is added under, and the reader's params.
type Chat = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);

const CHATS: [Chat; 3] = [
    (
        "whatsapp.txt",
        "whatsapp/1",
        "chat/wa",
        &[("date-format", "dmy")],
    ),
    ("telegram.json", "telegram/1", "chat/tg", &[]),
    ("slack.zip", "slack/1", "chat/sl", &[]),
];

fn repo() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

/// The library, and the units drawn from each chat in row order.
fn chats() -> (Library, Vec<Vec<Uid>>) {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("oq64");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut lib = Library::create(&root).unwrap();
    let mut per_chat = Vec::new();
    for (file, reader, alias, params) in CHATS {
        let raw = std::fs::read(repo().join("fixtures/library/readers").join(file)).unwrap();
        let mut spec = AddSpec::new(reader, alias, "unknown")
            .with_lang(LangTag::new("mul").unwrap())
            .with_carry(Carry::Ref);
        for (k, v) in params {
            spec.params.set(reader, *k, *v).unwrap();
        }
        let added = lib.add(&Input::new(&raw), &spec, &Caps::DEFAULT).unwrap();
        let manifest = lib.store().manifest(&added.mid).unwrap().clone();
        let mut records = Vec::new();
        let mut uids = Vec::new();
        let mut by_msg = BTreeMap::new();
        let mut replies = Vec::new();
        for tid in &added.parts {
            let reading = lib.part_reading(&added.mid, tid).unwrap();
            for row in &reading.rows {
                if row.level.as_str() != "message" {
                    continue;
                }
                let at = row.locator.to_string();
                let Ok(passage) = lib.passage(alias, &locator::parse(&at).unwrap(), &Caps::DEFAULT)
                else {
                    continue;
                };
                let gist = passage.text.trim().to_string();
                if gist.is_empty() {
                    continue;
                }
                let unit = provenance::stamp(
                    UnitCoreBuilder::new(KernelType::Claim, &gist, Status::Cited),
                    &provenance::Drawn {
                        tid: passage.tid,
                        locator: Some(&at),
                        manifest: &manifest,
                        observed: row.observed,
                        span: Some((passage.range.start, passage.range.end)),
                        lang: Some(&manifest.lang),
                    },
                )
                .build()
                .unwrap();
                let uid = canonical_uid(&unit);
                if let Some(m) = row.ids.get("msg") {
                    by_msg.insert(m.clone(), uid);
                }
                if let Some(r) = row.ids.get("reply") {
                    replies.push((uid, r.clone()));
                }
                uids.push(uid);
                records.push(Record::Unit(unit));
            }
        }
        let kind = RelKind::Extension(smysl_core::schemas::text::REPLIES.to_string());
        for (from, to) in replies {
            if let Some(to) = by_msg.get(&to) {
                records.push(Record::Relation(Relation::new(kind.clone(), from, *to)));
            }
        }
        lib.append_records(&records).unwrap();
        per_chat.push(uids);
    }
    (lib, per_chat)
}

fn thread(lib: &Library, id: &str, scope: &[Uid]) -> Thread {
    let opts = DeriveOptions::new(
        ThreadId::new(id).unwrap(),
        AgentId::new("tool:smysl-thread").unwrap(),
    )
    .scoped(scope.iter().copied())
    .with_ts(Hlc::new(
        1_705_400_000_000,
        0,
        AgentId::new("tool:smysl-thread").unwrap(),
    ));
    derive_thread(lib.store(), ThreadSchema::Dialogue, &opts).0
}

#[test]
fn threads_that_span_files_on_the_chat_corpora() {
    let (mut lib, per_chat) = chats();
    let mut threads = Vec::new();
    for ((_, _, alias, _), uids) in CHATS.iter().zip(&per_chat) {
        assert!(uids.len() >= 3, "{alias}: {} messages drawn", uids.len());
        threads.push(thread(
            &lib,
            &format!("t/{}", alias.replace('/', "-")),
            uids,
        ));
    }
    let all: Vec<Uid> = per_chat.iter().flatten().copied().collect();
    threads.push(thread(&lib, "t/library", &all));
    let mut records = Vec::new();
    for t in &threads {
        let mut p = PackInfo::new(4_000, 1_000, "smysl/utf8-div4");
        p.thread = Some(t.id.clone());
        records.push(Record::Thread(t.clone()));
        records.push(Record::PackInfo(p));
    }
    lib.append_records(&records).unwrap();

    let with = With::new().packs(true);
    let snap = snapshot(&lib, &with).unwrap();
    let sel = Selection::all(&snap);
    eprintln!(
        "OQ-64: {} threads over {} messages",
        threads.len(),
        all.len()
    );
    for layout in [
        Layout::Single,
        Layout::ByExpression,
        Layout::ByCollection,
        Layout::ByTime,
    ] {
        for closure in [Closure::Full, Closure::Ref] {
            let opts = ExportOptions::new()
                .layout(layout)
                .closure(closure)
                .with(with);
            let c = export_library(&lib, &snap, &sel, &opts).unwrap();
            // Which file holds which unit, and which threads made it into some file.
            let mut home: BTreeMap<Uid, BTreeSet<&str>> = BTreeMap::new();
            let mut written: BTreeSet<ThreadId> = BTreeSet::new();
            let mut packs = 0;
            for f in &c.files {
                for r in smysl_core::from_cbor_seq(&f.bytes).unwrap().0 {
                    match r {
                        Record::Unit(u) => {
                            home.entry(canonical_uid(&u)).or_default().insert(&f.path);
                        }
                        Record::Thread(t) => {
                            written.insert(t.id);
                        }
                        Record::PackInfo(_) => packs += 1,
                        _ => {}
                    }
                }
            }
            let w490: usize = c
                .report
                .iter()
                .filter(|d| d.code == smysl_core::Code::W490)
                .map(|d| {
                    d.message
                        .split(' ')
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap()
                })
                .sum();
            let mut out = Vec::new();
            for t in &threads {
                if written.contains(&t.id) {
                    continue;
                }
                let spans: BTreeSet<&str> = t
                    .steps
                    .iter()
                    .flat_map(|s| home.get(&s.unit).into_iter().flatten().copied())
                    .collect();
                assert!(spans.len() > 1, "{t:?}: left out, and all in one file");
                out.push(format!("{} ({} files)", t.id, spans.len()));
            }
            eprintln!(
                "OQ-64: {:<14} {:<4} {} files, W490 {w490:>2}, threads left out {}/{}, pack infos {}/{}: {}",
                layout.as_str(),
                closure.as_str(),
                c.files.len(),
                out.len(),
                threads.len(),
                packs,
                threads.len(),
                out.join(", ")
            );
            // A pack info goes with its thread.
            assert_eq!(packs + out.len(), threads.len(), "{layout:?} {closure:?}");
            assert_eq!(w490 > 0, !out.is_empty(), "{layout:?} {closure:?}");
            match (layout, closure) {
                (_, Closure::Ref) | (Layout::Single | Layout::ByCollection, _) => {
                    assert!(out.is_empty(), "{layout:?} {closure:?}: {out:?}")
                }
                (Layout::ByExpression, Closure::Full) => {
                    assert!(out.iter().all(|t| t.starts_with("t/library ")), "{out:?}")
                }
                _ => {}
            }
            // Under `ref` the file holding a thread requires every file that holds a step.
            if closure == Closure::Ref {
                for f in &c.files {
                    for r in smysl_core::from_cbor_seq(&f.bytes).unwrap().0 {
                        let Record::Thread(t) = r else { continue };
                        for s in &t.steps {
                            for h in &home[&s.unit] {
                                assert!(
                                    *h == f.path || f.requires.iter().any(|q| q == h),
                                    "{layout:?}: {} in {} does not require {h}",
                                    t.id,
                                    f.path
                                );
                            }
                        }
                    }
                }
            }
            // And the collection verifies, and round-trips through a real import (P1).
            let v = verify_collection(&c);
            assert!(v.holds(), "{layout:?} {closure:?}: {v:?}");
            let at = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
                "oq64-{}-{}",
                layout.as_str(),
                closure.as_str()
            ));
            let _ = std::fs::remove_dir_all(&at);
            write_collection(&c, &at).unwrap();
            let mut to = Library::create(&at.join("lib")).unwrap();
            let r = import(
                &mut to,
                &[ImportInput::Index(at.join(INDEX_FILE))],
                &ImportOptions::new().accept(true).join(false),
            )
            .unwrap();
            assert_eq!(
                smysl_graph::record_set_digest(&retained(&to, &r.name).unwrap()),
                c.rsd(),
                "{layout:?} {closure:?}: P1"
            );
            let _ = std::fs::remove_dir_all(&at);
        }
    }
}
