//! TX-P8 step 2's exit: `exposition` and `dialogue` derived over the book and chat fixtures.
//!
//! Both schemas are ordered by something only a library records. An exposition is a text in
//! its own order, which is span order within a part — and a text's units carry no `observed`
//! and almost no ordering edges, so before step 2 derivation left them in uid order. A dialogue
//! is ordered by its replies, which a chat reader records as `reply` ids on reading rows.
//!
//! **One finding the chat half needed.** `x.text/replies` is declared by `x.text/v1` and read
//! by rule E as a free constraint, and nothing in this build writes one: the readers record
//! `reply` ids on rows, and no path turns a row's id into an edge between the units drawn from
//! those rows. So the edges are drawn here, from the ids, the way an extraction would — which
//! is what makes the test about derivation rather than about a producer that does not exist.
//! The producer is owed and recorded in SMYSL-2.5 §6 step 2.

#![cfg(all(feature = "reader-usfm", feature = "reader-telegram"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::ids::{AgentId, LangTag, Uid};
use smysl_core::types::{Carry, Record, RelKind, Relation};
use smysl_core::{
    canonical_uid, KernelType, Role, Status, ThreadId, ThreadSchema, UnitCoreBuilder,
};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;
use smysl_text::{locator, provenance};
use smysl_thread::{derivable, derive_thread, satisfies_rule_l, DeriveOptions};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A drawn unit: its uid, where its row starts, and the platform ids the row carried.
struct Drawn {
    uid: Uid,
    start: u64,
    ids: BTreeMap<String, String>,
}

struct Built {
    root: PathBuf,
    library: Library,
    drawn: Vec<Drawn>,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// One expression added and a unit drawn per row of `level`, as `sq_library.rs` draws them.
fn build(name: &str, file: &str, reader: &str, alias: &str, lang: &str, level: &str) -> Built {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-thread-schemas-{name}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    let mut library = Library::create(&root).expect("a library");

    let raw = std::fs::read(repo(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
    let mut spec = AddSpec::new(reader, alias, "public-domain")
        .with_lang(LangTag::new(lang).expect("a tag"))
        .with_carry(Carry::Text);
    if reader == "usfm/1" {
        spec.versification = Some("kjv".to_string());
    }
    let added = library
        .add(&Input::new(&raw), &spec, &Caps::DEFAULT)
        .unwrap_or_else(|e| panic!("{file}: {e}"));
    let manifest = library
        .store()
        .manifest(&added.mid)
        .expect("its manifest")
        .clone();

    let mut records = Vec::new();
    let mut drawn = Vec::new();
    for tid in &added.parts {
        let reading = library.part_reading(&added.mid, tid).expect("a reading");
        for row in &reading.rows {
            if row.level.as_str() != level {
                continue;
            }
            let at = row.locator.to_string();
            let Ok(passage) = library.passage(
                alias,
                &locator::parse(&at).expect("the reader's own locator"),
                &Caps::DEFAULT,
            ) else {
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
            .expect("a unit of a row");
            drawn.push(Drawn {
                uid: canonical_uid(&unit),
                start: passage.range.start,
                ids: row.ids.clone(),
            });
            records.push(Record::Unit(unit));
        }
    }
    library.append_records(&records).expect("the units append");
    Built {
        root,
        library,
        drawn,
    }
}

fn options(id: &str, scope: impl IntoIterator<Item = Uid>) -> DeriveOptions {
    DeriveOptions::new(
        ThreadId::new(id).expect("a thread id"),
        AgentId::new("tool:test").expect("an agent"),
    )
    .scoped(scope)
}

/// An exposition over Ruth follows the text: its steps are in span order, the first is the
/// introduction and the last the conclusion.
#[test]
fn an_exposition_of_ruth_is_in_the_order_the_text_has_it() {
    let built = build(
        "ruth",
        "fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm",
        "usfm/1",
        "kjv/08-rut",
        "en",
        "verse",
    );
    assert!(built.drawn.len() > 50, "Ruth has 85 verses");
    let store = built.library.store();
    let (thread, report) = derive_thread(
        store,
        ThreadSchema::Exposition,
        &options("t/ruth", built.drawn.iter().map(|d| d.uid)),
    );
    assert!(thread.steps.len() >= 2, "{report:?}");
    assert!(satisfies_rule_l(store, &thread).is_empty());

    let start: BTreeMap<Uid, u64> = built.drawn.iter().map(|d| (d.uid, d.start)).collect();
    let first_verse = built
        .drawn
        .iter()
        .min_by_key(|d| d.start)
        .expect("a verse")
        .uid;
    let last_verse = built
        .drawn
        .iter()
        .max_by_key(|d| d.start)
        .expect("a verse")
        .uid;

    let intro: Vec<&Uid> = thread
        .steps
        .iter()
        .filter(|s| s.role == Role::Introduces)
        .map(|s| &s.unit)
        .collect();
    assert_eq!(intro, vec![&first_verse], "the introduction is Ruth 1:1");
    let concl: Vec<&Uid> = thread
        .steps
        .iter()
        .filter(|s| s.role == Role::Concludes)
        .map(|s| &s.unit)
        .collect();
    assert_eq!(concl, vec![&last_verse], "the conclusion is the last verse");

    // Within each role the steps keep the text's order: the span key is the ordering, not a
    // tie-break the role order happens to agree with.
    for role in ThreadSchema::Exposition.roles() {
        let at: Vec<u64> = thread
            .steps
            .iter()
            .filter(|s| s.role == *role)
            .map(|s| start[&s.unit])
            .collect();
        let mut sorted = at.clone();
        sorted.sort_unstable();
        assert_eq!(at, sorted, "{role} steps are out of the text's order");
    }
    // And the thread as a whole only moves forward through the book, role by role.
    let roles: Vec<Role> = thread.steps.iter().map(|s| s.role).collect();
    assert_eq!(roles.first(), Some(&Role::Introduces));
}

/// A dialogue over the Telegram fixture is ordered by its replies: a message somebody answered
/// is uptake, a message that answers is a reply, and each reply comes after what it answers.
#[test]
fn a_dialogue_is_ordered_by_its_replies() {
    let mut built = build(
        "chat",
        "fixtures/library/readers/telegram.json",
        "telegram/1",
        "reading-group",
        "mul",
        "message",
    );
    assert_eq!(built.drawn.len(), 5, "the fixture has five messages");

    // The edges no producer writes yet (see the module header): from each row's `reply` id to
    // the row whose `msg` id it names.
    let by_msg: BTreeMap<String, Uid> = built
        .drawn
        .iter()
        .filter_map(|d| d.ids.get("msg").map(|m| (m.clone(), d.uid)))
        .collect();
    let replies = RelKind::Extension(smysl_core::schemas::text::REPLIES.to_string());
    let mut pairs: Vec<(Uid, Uid)> = Vec::new();
    for d in &built.drawn {
        if let Some(to) = d.ids.get("reply").and_then(|r| by_msg.get(r)) {
            pairs.push((d.uid, *to));
        }
    }
    assert_eq!(pairs.len(), 2, "the fixture has two replies");
    let edges: Vec<Record> = pairs
        .iter()
        .map(|(from, to)| Record::Relation(Relation::new(replies.clone(), *from, *to)))
        .collect();
    built
        .library
        .append_records(&edges)
        .expect("the edges append");

    let store = built.library.store();
    let (thread, report) = derive_thread(
        store,
        ThreadSchema::Dialogue,
        &options("t/chat", built.drawn.iter().map(|d| d.uid)),
    );
    assert_eq!(thread.steps.len(), 5, "{report:?}");
    let role_of: BTreeMap<Uid, Role> = thread.steps.iter().map(|s| (s.unit, s.role)).collect();
    for (reply, answered) in &pairs {
        assert_eq!(
            role_of[answered],
            Role::Uptake,
            "an answered message is uptake"
        );
        assert_eq!(
            role_of[reply],
            Role::Reply,
            "an answering message is a reply"
        );
    }
    let position: BTreeMap<Uid, usize> = thread
        .steps
        .iter()
        .enumerate()
        .map(|(i, s)| (s.unit, i))
        .collect();
    for (reply, answered) in &pairs {
        assert!(
            position[answered] < position[reply],
            "a reply comes after what it answers"
        );
    }
}

/// `dossier` has no rules; the library and the CLI agree on that one fact.
#[test]
fn a_dossier_is_not_derivable() {
    assert!(!derivable(ThreadSchema::Dossier));
    assert!(derivable(ThreadSchema::Exposition));
    assert!(derivable(ThreadSchema::Dialogue));
}
