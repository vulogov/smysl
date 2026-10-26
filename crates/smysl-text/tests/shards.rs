//! A library's shards (TX-P13c step 1, SMYSL-2.8 §3.6–§3.7 with SMYSL-2.4 §14.2).
//!
//! Each expression's units go to a log of their own and everything else stays in the catalog;
//! the library still reads as the one store it always was. And a union of some of its shards
//! says, for a reference into a shard it does not hold, which shard it waits on (`SMY-W500`).

#![cfg(feature = "reader-txt")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use smysl_core::diag::{Code, Report};
use smysl_core::ids::{AgentId, KernelType, LangTag, Tid};
use smysl_core::types::epistemics::{SourceKind, SourceRef, Status};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::unit::UnitCoreBuilder;
use smysl_core::types::{Carry, Record};
use smysl_core::{canonical_uid, to_cbor_seq, Attestation, RelKind, Relation};
use smysl_graph::{Store, StoreRead, Union};
use smysl_text::library::{AddSpec, Library, LAYOUT, LAYOUT_1, MARKER};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;
use smysl_text::shards;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-shards-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("a scratch directory");
        Scratch(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spec(alias: &str) -> AddSpec {
    AddSpec::new("txt/1", alias, "CC0-1.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text)
}

fn unit_from(tid: &Tid, locator: &str, gist: &str) -> smysl_core::UnitCore {
    let source = SourceRef::new(SourceKind::Doc, format!("{}#{locator}", tid.canonical()));
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .source(source)
        .build()
        .expect("a unit")
}

fn agent() -> AgentId {
    AgentId::new("tool:shards").unwrap()
}

/// Two expressions, a unit from each, a `quotes` edge from one to the other, an attestation
/// on each unit and one unit with no source at all.
struct Fixture {
    reply: smysl_core::UnitCore,
    message: smysl_core::UnitCore,
    loose: smysl_core::UnitCore,
    quotes: Relation,
}

fn grow(lib: &mut Library) -> Fixture {
    let a = lib
        .add(
            &Input::new(b"the reply quotes the message\n"),
            &spec("chat/replies"),
            &Caps::DEFAULT,
        )
        .expect("added");
    let b = lib
        .add(
            &Input::new(b"the message being quoted\n"),
            &spec("chat/messages"),
            &Caps::DEFAULT,
        )
        .expect("added");
    let reply = unit_from(&a.parts[0], "L1", "the reply");
    let message = unit_from(&b.parts[0], "L1", "the message");
    let loose = UnitCoreBuilder::new(KernelType::Claim, "from nowhere", Status::Speculative)
        .build()
        .unwrap();
    let quotes = Relation::new(
        RelKind::Extension("x.text/quotes".into()),
        canonical_uid(&reply),
        canonical_uid(&message),
    );
    let attest = |u: &smysl_core::UnitCore| {
        Record::Attestation(Attestation::new(
            canonical_uid(u),
            agent(),
            smysl_core::Op::Imported,
            smysl_core::Rung::Computed,
            Hlc::new(7, 0, agent()),
        ))
    };
    lib.append_records(&[
        Record::Unit(reply.clone()),
        Record::Unit(message.clone()),
        Record::Unit(loose.clone()),
        Record::Relation(quotes.clone()),
        attest(&reply),
        attest(&message),
    ])
    .expect("appended");
    Fixture {
        reply,
        message,
        loose,
        quotes,
    }
}

fn read_log(root: &Path, shard: &str) -> Vec<Record> {
    let bytes = std::fs::read(root.join(shards::log_path(shard))).unwrap_or_default();
    smysl_core::from_cbor_seq(&bytes).expect("a log").0
}

#[test]
fn each_expression_gets_its_units_and_the_catalog_keeps_the_rest() {
    let dir = Scratch::new("route");
    let mut lib = Library::create(dir.path()).expect("created");
    let f = grow(&mut lib);
    assert_eq!(lib.shards(), ["catalog", "chat/messages", "chat/replies"]);
    assert!(
        dir.path().join("log/shards/chat~replies.cbor").is_file(),
        "an alias's `/` is not a directory"
    );

    let replies = read_log(dir.path(), "chat/replies");
    let messages = read_log(dir.path(), "chat/messages");
    let catalog = read_log(dir.path(), "catalog");
    let has_unit = |log: &[Record], u: &smysl_core::UnitCore| {
        log.iter().any(|r| matches!(r, Record::Unit(x) if x == u))
    };
    assert!(has_unit(&replies, &f.reply));
    assert!(has_unit(&messages, &f.message));
    assert!(
        has_unit(&catalog, &f.loose),
        "a unit from no text stays in the catalog"
    );
    assert!(
        replies
            .iter()
            .any(|r| matches!(r, Record::Relation(x) if x == &f.quotes)),
        "a relation goes where its `from` is"
    );
    assert!(
        messages
            .iter()
            .any(|r| matches!(r, Record::Attestation(a) if a.uid == canonical_uid(&f.message))),
        "an attestation goes where its subject is"
    );
    assert!(
        catalog.iter().all(|r| !matches!(r, Record::Attestation(_))),
        "{catalog:?}"
    );
    assert_eq!(
        catalog
            .iter()
            .filter(|r| matches!(r, Record::Manifest(_)))
            .count(),
        2,
        "manifests stay in the catalog"
    );

    // The library is the one store it always was: the same as every log read together.
    let together: Vec<Record> = lib
        .shards()
        .iter()
        .flat_map(|s| read_log(dir.path(), s))
        .collect();
    let union = Store::from_records(together);
    assert_eq!(lib.store().record_set_digest(), union.record_set_digest());
    assert_eq!(lib.store().state_hash(), union.state_hash());

    // And reopening reads it back exactly, routing nothing twice.
    let digest = lib.store().record_set_digest();
    let stamp = lib.stamp();
    drop(lib);
    let mut lib = Library::open(dir.path()).expect("reopened");
    assert_eq!(lib.store().record_set_digest(), digest);
    assert_eq!(lib.stamp(), stamp, "the stamp is a function of the logs");
    let before = std::fs::read(dir.path().join(shards::log_path("chat/replies"))).unwrap();
    lib.append_records(&[Record::Unit(f.reply.clone())])
        .expect("a repeat appends nothing");
    assert_eq!(
        std::fs::read(dir.path().join(shards::log_path("chat/replies"))).unwrap(),
        before
    );
}

#[test]
fn a_union_of_some_shards_waits_on_the_others() {
    let dir = Scratch::new("waits");
    let mut lib = Library::create(dir.path()).expect("created");
    let f = grow(&mut lib);
    drop(lib);

    let open = |s: &str| Store::from_records(read_log(dir.path(), s));
    let (catalog, replies, messages) =
        (open("catalog"), open("chat/replies"), open("chat/messages"));
    let index = Arc::new(shards::uid_index(dir.path()).expect("an index"));
    assert_eq!(
        index
            .shards_of(&canonical_uid(&f.message))
            .collect::<Vec<_>>(),
        ["chat/messages"]
    );

    let dangling = |u: &dyn Fn(&mut Report)| {
        let mut r = Report::new();
        u(&mut r);
        r.diagnostics
            .iter()
            .map(|d| (d.code, d.message.clone()))
            .collect::<Vec<_>>()
    };
    let some = Union::new([
        ("catalog".to_string(), &catalog),
        ("chat/replies".to_string(), &replies),
    ])
    .with_index(index.clone());
    let got = dangling(&|r| some.report_dangling(r));
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].0, Code::W500);
    assert!(got[0].1.contains("chat/messages"), "{got:?}");

    let all = Union::new([
        ("catalog".to_string(), &catalog),
        ("chat/replies".to_string(), &replies),
        ("chat/messages".to_string(), &messages),
    ])
    .with_index(index);
    assert!(dangling(&|r| all.report_dangling(r)).is_empty());
    let lib = Library::open(dir.path()).expect("reopened");
    assert_eq!(all.record_set_digest(), lib.store().record_set_digest());
    assert_eq!(all.state_hash(), lib.store().state_hash());
}

#[test]
fn a_layout_one_library_opens_and_becomes_layout_two_when_it_shards() {
    let dir = Scratch::new("upgrade");
    let mut lib = Library::create(dir.path()).expect("created");
    let added = lib
        .add(&Input::new(b"one line\n"), &spec("notes"), &Caps::DEFAULT)
        .expect("added");
    drop(lib);
    // What 1.10.0-dev wrote: one log, layout 1, every record in the catalog.
    std::fs::write(dir.path().join(MARKER), format!("{LAYOUT_1}\n")).unwrap();
    let unit = unit_from(&added.parts[0], "L1", "a claim");
    let mut catalog = read_log(dir.path(), "catalog");
    catalog.push(Record::Unit(unit.clone()));
    std::fs::write(
        dir.path().join(shards::log_path("catalog")),
        to_cbor_seq(&catalog),
    )
    .unwrap();

    let mut lib = Library::open(dir.path()).expect("a layout-1 library opens");
    assert_eq!(lib.layout(), LAYOUT_1);
    assert_eq!(lib.shards(), ["catalog"]);
    assert!(lib.store().contains_uid(&canonical_uid(&unit)));
    let stamp = lib.stamp();
    let bytes = std::fs::read(dir.path().join(shards::log_path("catalog"))).unwrap();
    assert_eq!(
        stamp,
        (bytes.len() as u64, smysl_core::hash_bytes(&bytes)),
        "one shard stamps as its log, as the catalog stamped before shards"
    );

    // The first write that makes a shard raises the marker first.
    lib.append_records(&[Record::Unit(unit_from(
        &added.parts[0],
        "L1",
        "another claim",
    ))])
    .expect("appended");
    assert_eq!(lib.layout(), LAYOUT);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(MARKER))
            .unwrap()
            .trim(),
        LAYOUT
    );
    assert_eq!(lib.shards(), ["catalog", "notes"]);
    assert_ne!(lib.stamp(), stamp);
}

#[test]
fn a_torn_tail_in_a_shard_is_cut_before_the_next_write() {
    let dir = Scratch::new("torn");
    let mut lib = Library::create(dir.path()).expect("created");
    let f = grow(&mut lib);
    drop(lib);
    let path = dir.path().join(shards::log_path("chat/replies"));
    let whole = std::fs::read(&path).unwrap();
    // Half a record: what a writer killed mid-append leaves.
    let mut torn = whole.clone();
    let half = to_cbor_seq(&[Record::Unit(f.loose.clone())]);
    torn.extend_from_slice(&half[..half.len() / 2]);
    std::fs::write(&path, &torn).unwrap();

    let mut lib = Library::open(dir.path()).expect("a torn tail does not fail an open");
    assert!(lib.store().contains_uid(&canonical_uid(&f.reply)));
    let next = unit_from(&tid_of(&lib, "chat/replies"), "L1", "a later reply");
    lib.append_records(&[Record::Unit(next.clone())])
        .expect("appended");
    let after = std::fs::read(&path).unwrap();
    assert!(after.starts_with(&whole));
    let (records, consumed) = smysl_core::from_cbor_seq(&after).unwrap();
    assert_eq!(consumed, after.len(), "the tail was cut, not written after");
    assert!(records
        .iter()
        .any(|r| matches!(r, Record::Unit(x) if x == &next)));
}

fn tid_of(lib: &Library, alias: &str) -> Tid {
    let mid = lib.store().heads(alias)[0];
    lib.store().manifest(&mid).unwrap().parts[0].tid
}
