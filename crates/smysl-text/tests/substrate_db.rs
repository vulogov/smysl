//! `index/substrate.redb` against the sidecar it replaces (RFC SMYSL-2.4 §3.8,
//! `substrate-redb`), and the library that reads and writes it.
//!
//! Three claims, and the second is the one that matters.
//!
//! **The store holds what the table holds.** §3.8's gate for every other table is a byte
//! comparison, and it is unavailable here: a redb file's page allocation depends on the history
//! of writes, so two libraries with identical contents have different files. `Db::agrees` is the
//! replacement, over the logical table, and `the_store_disagrees_when_the_table_moves` is what
//! keeps it from being a function that returns `Ok`.
//!
//! **A score is the same score.** The reason to have a seam at all is that a query answered from
//! the store must be the query answered from memory — not close, the same `f32`. So the suite
//! scores a corpus through `postings::View` and through `db::View` and compares bit patterns,
//! over every query shape the two paths differ in: a term in one partition, a term in both, a
//! term in neither, and a repeated term.
//!
//! **A library writes it, reads it, and grows it.** The wiring: a write puts the `postings`
//! table in the store and leaves it out of the sidecar, an open scores it where it lies, and a
//! writing command brings it forward — and the index it arrives at is byte-identical to the one
//! a full rebuild produces, which is §3.8's own gate asked across the store rather than beside
//! it.

#![cfg(all(feature = "reader-txt", feature = "substrate-redb"))]

use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Tid};
use smysl_core::types::{Carry, Record, SourceKind, SourceRef};
use smysl_core::{KernelType, Status, UnitCoreBuilder};
use smysl_retrieve::{Postings, Retriever};
use smysl_text::index::db::Db;
use smysl_text::index::postings;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::part::Policy;
use smysl_text::readers::Input;
use smysl_text::reading::Level;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-db-{tag}-{}-{}",
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

const NOTES: &str = "\
the first line of the notes
the second, a little longer than the first
the third
the fourth line mentions the engine
the fifth
the sixth line is about the pool
the seventh
the eighth line closes the morning
the ninth
";

const LETTER: &str = "\
dear reader
the engine was repaired on tuesday
the pool is open again
";

fn spec(alias: &str) -> AddSpec {
    AddSpec::new("txt/1", alias, "CC0-1.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text)
        .with_policy(Policy::new(Level::new("line").expect("a level"), 40, 120))
}

fn unit_from(tid: &Tid, locator: &str, gist: &str) -> Record {
    let mut source = SourceRef::new(SourceKind::Doc, format!("{}#{locator}", tid.canonical()));
    source.observed = Some(1_726_000_000_000);
    Record::Unit(
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited)
            .source(source)
            .build()
            .expect("a unit"),
    )
}

/// Two expressions, units drawn from both, and one unit whose source names no part — so the
/// table has three partitions and one of them holds a single document.
fn grown(tag: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("created");
    let notes = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &spec("notes"),
            &Caps::DEFAULT,
        )
        .expect("added notes");
    let letter = lib
        .add(
            &Input::new(LETTER.as_bytes()),
            &spec("letter"),
            &Caps::DEFAULT,
        )
        .expect("added the letter");

    let loose = Record::Unit(
        UnitCoreBuilder::new(
            KernelType::Claim,
            "the engine and the pool together",
            Status::Speculative,
        )
        .build()
        .expect("a unit"),
    );
    let units = vec![
        unit_from(&notes.parts[0], "L1", "the notes open on the first line"),
        unit_from(
            &notes.parts[0],
            "L4",
            "the engine is mentioned in the notes",
        ),
        unit_from(
            &letter.parts[0],
            "L2",
            "the letter says the engine was repaired",
        ),
        loose,
    ];
    lib.append_records(&units).expect("appended");
    lib.complete_index();
    (dir, lib)
}

/// Every query shape the two paths can differ in.
const QUERIES: &[&str] = &[
    "engine",
    "pool",
    "engine pool",
    "the engine",
    "engine engine",
    "notes first line",
    "nothing at all matches this",
    "",
];

fn written(lib: &Library) -> Db {
    let db = Db::open(lib.root()).expect("opened the store");
    db.write(
        &lib.index().postings,
        lib.store().log_len(),
        lib.store().log_hash(),
    )
    .expect("wrote the store");
    db
}

#[test]
fn the_store_holds_what_the_table_holds() {
    let (_dir, lib) = grown("agrees");
    let db = written(&lib);
    assert!(
        !lib.index().postings.is_empty(),
        "a fixture with no postings would pass every test here"
    );
    if let Err(why) = db.agrees(&lib.index().postings) {
        panic!("{why}");
    }
}

/// The other half: `agrees` has to be able to say no. A posting removed from the in-memory
/// table is the smallest difference the comparison must find.
#[test]
fn the_store_disagrees_when_the_table_moves() {
    let (_dir, lib) = grown("disagrees");
    let db = written(&lib);

    let mut moved = lib.index().postings.clone();
    let partition = moved.keys().next().expect("a partition").clone();
    moved.remove(&partition);
    assert!(
        db.agrees(&moved).is_err(),
        "a table missing a whole partition compared equal"
    );
}

#[test]
fn the_stamp_is_the_catalog_it_was_written_for() {
    let (_dir, lib) = grown("stamp");
    let db = written(&lib);
    assert!(db.matches(lib.store().log_len(), lib.store().log_hash()));
    assert!(
        !db.matches(lib.store().log_len() + 1, lib.store().log_hash()),
        "a stale stamp compared current"
    );
}

/// The claim the seam exists for: the same `f32`, not a close one.
#[test]
fn a_score_from_the_store_is_the_score_from_memory() {
    let (_dir, lib) = grown("scores");
    let db = written(&lib);

    let memory = postings::View::all(&lib.index().postings);
    let stored = db.all().expect("a view");
    assert_eq!(stored.partitions(), memory.partitions());
    assert_eq!(stored.docs(), memory.docs());
    assert_eq!(stored.total_len(), memory.total_len());
    assert_eq!(stored.avgdl().to_bits(), memory.avgdl().to_bits());
    assert_eq!(stored.analyzer(), memory.analyzer());

    let analyzer = smysl_text::analyze::Chain::plain();
    for q in QUERIES {
        let qy = smysl_retrieve::Query::new(*q, 20);
        let from_memory = smysl_retrieve::Bm25::over(&memory, &analyzer, lib.store()).search(&qy);
        let from_store = smysl_retrieve::Bm25::over(&stored, &analyzer, lib.store()).search(&qy);
        assert_eq!(
            from_memory.len(),
            from_store.len(),
            "`{q}`: {} hits from memory and {} from the store",
            from_memory.len(),
            from_store.len()
        );
        for (m, s) in from_memory.iter().zip(from_store.iter()) {
            assert_eq!(m.uid, s.uid, "`{q}`: a different unit");
            assert_eq!(
                m.score.to_bits(),
                s.score.to_bits(),
                "`{q}`: {} from memory and {} from the store",
                m.score,
                s.score
            );
        }
    }
}

/// `tf` is overridden on both views, and a search does not exercise it — a decomposed score
/// does. The binary search over the stored value bytes is the thing being compared here.
#[test]
fn a_term_frequency_from_the_store_is_the_one_from_memory() {
    let (_dir, lib) = grown("tf");
    let db = written(&lib);
    let memory = postings::View::all(&lib.index().postings);
    let stored = db.all().expect("a view");

    let mut asked = 0usize;
    for term in ["engine", "the", "pool", "notes", "absent"] {
        let from_memory = memory.postings(term);
        assert_eq!(
            from_memory,
            stored.postings(term),
            "`{term}`: the lists differ"
        );
        assert_eq!(memory.df(term), stored.df(term), "`{term}`: the df differs");
        for (uid, _) in &from_memory {
            assert_eq!(
                memory.tf(term, uid),
                stored.tf(term, uid),
                "`{term}` in `{}`",
                uid.short()
            );
            assert_eq!(memory.doc_len(uid), stored.doc_len(uid));
            asked += 1;
        }
    }
    assert!(asked > 0, "no term of the fixture is in the table");
}

/// A scope narrower than the library is the same arithmetic on both backings (OQ-14): the
/// counts are over disjoint document sets, so a scope of one partition is that partition's own
/// statistics and not the library's.
#[test]
fn a_narrower_scope_agrees_too() {
    let (_dir, lib) = grown("scope");
    let db = written(&lib);
    let table = &lib.index().postings;

    for partition in table.keys() {
        let one = std::slice::from_ref(partition);
        let memory = postings::View::of(table, one);
        let stored = db.of(one).expect("a view");
        assert_eq!(stored.partitions(), 1, "{partition}");
        assert_eq!(stored.docs(), memory.docs(), "{partition}: docs");
        assert_eq!(
            stored.total_len(),
            memory.total_len(),
            "{partition}: length"
        );
        assert_eq!(
            stored.avgdl().to_bits(),
            memory.avgdl().to_bits(),
            "{partition}: avgdl"
        );
        for term in ["engine", "the", "pool"] {
            assert_eq!(memory.df(term), stored.df(term), "{partition}: `{term}`");
            assert_eq!(
                memory.postings(term),
                stored.postings(term),
                "{partition}: `{term}`"
            );
        }
    }
}

/// A file nothing has been written to answers rather than failing, because a cache that cannot
/// be read is rebuilt by its caller and an error path here would be one every scorer carried.
#[test]
fn an_empty_store_is_an_empty_index() {
    let dir = Scratch::new("empty");
    Library::create(dir.path()).expect("created");
    let db = Db::open(dir.path()).expect("opened");
    assert_eq!(db.stamp().expect("read"), None);
    assert_eq!(db.terms().expect("read"), 0);
    assert!(db.partitions().expect("read").is_empty());
    let view = db.all().expect("a view");
    assert_eq!(view.partitions(), 0);
    assert_eq!(view.docs(), 0);
    assert_eq!(view.df("engine"), 0);
    assert!(view.postings("engine").is_empty());
    assert_eq!(view.analyzer(), None);
    // The fallback, not a division by zero.
    assert_eq!(view.avgdl(), smysl_retrieve::postings::FALLBACK_AVGDL);
}

// -- the wiring --------------------------------------------------------------

/// Decode a sidecar and say which tables it claims and how many postings it carries.
fn sidecar(lib: &Library) -> smysl_text::index::Index {
    let bytes = std::fs::read(smysl_text::index::Index::path(lib.root())).expect("a sidecar");
    smysl_text::index::Index::from_bytes(&bytes).expect("a sidecar this build can read")
}

#[test]
fn a_write_leaves_the_postings_in_the_store_and_out_of_the_sidecar() {
    let (_dir, lib) = grown("split");
    assert!(!lib.index().postings.is_empty(), "nothing to split");
    lib.write_index().expect("wrote the index");

    let written = sidecar(&lib);
    assert!(
        written.postings.is_empty(),
        "the sidecar still carries {} partition(s)",
        written.postings.len()
    );
    assert!(
        !written.built.has(smysl_text::index::Table::Postings),
        "the sidecar claims a table it does not hold"
    );
    // Every other table is still the sidecar's, and still claimed.
    for table in smysl_text::index::Table::BUILT {
        if *table == smysl_text::index::Table::Postings {
            continue;
        }
        assert!(written.built.has(*table), "{table} went missing");
    }
    assert!(lib.has_postings_store(), "no store beside the library");
    let db = Db::open(lib.root()).expect("opened the store");
    assert!(db.matches(lib.store().log_len(), lib.store().log_hash()));
    if let Err(why) = db.agrees(&lib.index().postings) {
        panic!("{why}");
    }
}

/// The read path, end to end: a library opened over the two files scores out of the store, and
/// scores the same `f32` the resident table did before it was written.
#[test]
fn a_reopened_library_scores_out_of_the_store() {
    let (dir, lib) = grown("reopen");
    lib.write_index().expect("wrote the index");
    let analyzer = smysl_text::analyze::Chain::plain();
    let before: Vec<Vec<(smysl_core::Uid, u32)>> = {
        let view = lib.postings().expect("the table is resident");
        assert!(!view.is_stored(), "the resident table lost to the store");
        QUERIES
            .iter()
            .map(|q| {
                smysl_retrieve::Bm25::over(&view, &analyzer, lib.store())
                    .search(&smysl_retrieve::Query::new(*q, 20))
                    .into_iter()
                    .map(|h| (h.uid, h.score.to_bits()))
                    .collect()
            })
            .collect()
    };
    drop(lib);

    let again = Library::open(dir.path()).expect("reopened");
    let view = again.postings().expect("the store answers");
    assert!(
        view.is_stored(),
        "an open that had nothing resident still did not read the store"
    );
    assert!(
        !again.index().built.has(smysl_text::index::Table::Postings),
        "the open claims a resident table"
    );
    for (q, want) in QUERIES.iter().zip(&before) {
        let got: Vec<(smysl_core::Uid, u32)> =
            smysl_retrieve::Bm25::over(&view, &analyzer, again.store())
                .search(&smysl_retrieve::Query::new(*q, 20))
                .into_iter()
                .map(|h| (h.uid, h.score.to_bits()))
                .collect();
        assert_eq!(&got, want, "`{q}`: the reopened library ranks differently");
    }
}

/// What `text index` prints, from whichever file holds the rows. The counts have to be the same
/// counts: a report that said a stored table held fewer terms than a resident one would be a
/// reader's only evidence that something had been lost.
#[test]
fn a_report_says_where_the_rows_are_and_counts_the_same_rows() {
    let (dir, lib) = grown("held");
    lib.write_index().expect("wrote the index");
    let resident = lib.postings_held().expect("a resident table");
    assert!(!resident.stored);
    drop(lib);

    let again = Library::open(dir.path()).expect("reopened");
    let stored = again.postings_held().expect("a stored table");
    assert!(
        stored.stored,
        "a reopened library reported a resident table"
    );
    assert_eq!(stored.terms, resident.terms, "terms");
    assert_eq!(stored.partitions, resident.partitions, "partitions");
    assert_eq!(stored.analyzers, resident.analyzers, "analyzers");
    assert!(stored.terms > 0 && stored.partitions > 1, "a thin fixture");
}

/// The write path's second half, and the sharpest claim here: a library whose postings are in
/// the store, appended to and written again, arrives at the index a full rebuild would — byte
/// for byte, which is §3.8's gate. Hydrating and extending has to produce what analysing the
/// whole corpus produces, or the store is a second opinion rather than a cache.
#[test]
fn an_append_over_the_store_lands_where_a_rebuild_does() {
    let (dir, lib) = grown("append");
    lib.write_index().expect("wrote the index");
    drop(lib);

    let mut again = Library::open(dir.path()).expect("reopened");
    // What a writing command does, in the order it does it.
    again.complete_index();
    assert!(
        again.index().built.has(smysl_text::index::Table::Postings),
        "the stored table was not hydrated; the corpus was reanalysed instead"
    );
    let tid = *again.index().parts.keys().next().expect("a part to cite");
    again
        .append_records(&[unit_from(
            &tid,
            "L6",
            "a later unit about the pool and the engine",
        )])
        .expect("appended");
    again.write_index().expect("wrote the index");

    let maintained = again.index().to_bytes();
    let rebuilt = {
        let mut fresh = Library::open(dir.path()).expect("reopened again");
        fresh.reindex();
        fresh.index().to_bytes()
    };
    assert_eq!(
        maintained.len(),
        rebuilt.len(),
        "the maintained index and the rebuilt one are different sizes"
    );
    assert!(
        maintained == rebuilt,
        "the index maintained over the store differs from the one rebuilt from the corpus"
    );
}

/// A stamp that does not match the catalog is not scored, and the caller is told nothing is
/// there rather than handed last week's rows. The remedy is the one it always was.
#[test]
fn a_stale_store_answers_nothing() {
    let (dir, mut lib) = grown("stale");
    lib.write_index().expect("wrote the index");
    let tid = *lib.index().parts.keys().next().expect("a part");
    // Appended and *not* written down: the catalog has moved past both files.
    lib.append_records(&[unit_from(&tid, "L7", "a unit nothing has indexed")])
        .expect("appended");
    drop(lib);

    let again = Library::open(dir.path()).expect("reopened");
    assert!(
        again.postings().is_none(),
        "a store stamped for an older catalog was scored"
    );
    assert!(again.postings_held().is_none());
    // And the file is still there, which is what makes the next write a replacement.
    assert!(again.has_postings_store());
}
