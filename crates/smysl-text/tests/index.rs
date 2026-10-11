//! The substrate index: §3.8's tables, and the gate that the two builders agree (TX-P4 step 1).
//!
//! The load-bearing test is `rebuild_and_incremental_agree_byte_for_byte`. Everything else here
//! is a case that was found by making it fail: each of the four tables whose shape needed
//! choosing — `parts`, `by_part`, `manifests.head`, and the two that rule Z empties — has a
//! test below because the byte comparison caught the obvious implementation first.
//!
//! The comparison is over [`Index::to_bytes`] and not over the values. Two indexes can be
//! equal as values and disagree about what they would persist, so the gate compares what
//! reaches the disk, which is also what a reader on the other side of a release would see.

// For the reason `tests/library.rs` gives: at default features this crate has no readers, so
// without the gate the suite would fail in the configuration `make crate-features` builds.
#![cfg(feature = "reader-txt")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::ids::Rdid;
use smysl_core::ids::{AgentId, LangTag, Mid, Tid};
use smysl_core::types::library::Axis;
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{
    Carry, Dating, DatingTarget, DatingValue, PartEntry, Record, SourceKind, SourceRef,
};
use smysl_core::{KernelType, Status, UnitCoreBuilder};
use smysl_text::index::{Built, Index, Table};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::locator;
use smysl_text::manifest::ManifestBuilder;
use smysl_text::part::Policy;
use smysl_text::readers::Input;
use smysl_text::reading::Level;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-index-{tag}-{}-{}",
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

/// Nine lines, which the policy below cuts into several parts.
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

/// The same text with a tenth line: what `append` reads as a new version.
const NOTES_V2: &str = "\
the first line of the notes
the second, a little longer than the first
the third
the fourth line mentions the engine
the fifth
the sixth line is about the pool
the seventh
the eighth line closes the morning
the ninth
the tenth line was added later
";

/// A second expression, so `by_part` and `manifests` hold more than one of everything.
const LETTER: &str = "\
dear reader
the engine was repaired on tuesday
the pool is open again
";

/// A policy sized for a fixture rather than for a Bible: the default `target_min` is 64 KiB,
/// which would make every text here one part and leave the part-local shift untested.
fn policy() -> Policy {
    Policy::new(Level::new("line").expect("a level"), 40, 120)
}

fn spec(alias: &str) -> AddSpec {
    AddSpec::new("txt/1", alias, "CC0-1.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text)
        .with_policy(policy())
}

/// The same, declaring a versification scheme (manifest key 14).
///
/// Line numbering rather than chapter and verse, because what the `align` table is being asked
/// here is whether it maps *locators* — and `txt/1`'s locators are lines. The Psalms case the
/// phase exit names is in `tests/align.rs`, where the locators are OSIS-shaped and the
/// divergence is the real one.
fn versified(alias: &str, scheme: &str) -> AddSpec {
    let mut spec = spec(alias);
    spec.versification = Some(scheme.to_string());
    spec
}

/// A map between the fixture's two line numberings: the letter's line *n* is the notes' line
/// *n+1*.
///
/// Three rows and six locators it says nothing about, so the table it produces has both rows and
/// an uncovered count — `SMY-W451` is as much a part of this table as its rows are, and a
/// fixture where every locator aligned would never exercise it.
const LINE_MAP: &str =
    "smysl/versification/1\nfrom\tnotes-lines\nto\tletter-lines\nL2\tL1\nL3\tL2\nL4\tL3\n";

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

/// A unit whose source names a part, which is what puts a row in the journal.
fn unit_from(tid: &Tid, locator: &str, gist: &str, status: Status) -> Record {
    unit_at(tid, locator, gist, status, Some(1_726_000_000_000))
}

/// The same, with the `observed` instant said outright.
///
/// Every unit the fixture makes carries one, because three of the eleven tables are about time
/// and a corpus of undated units leaves all three empty — which the non-vacuity half of the
/// gate catches, and which is the honest reading of a fixture anyway: a unit extracted from a
/// chat or a document has an instant its source recorded.
fn unit_at(tid: &Tid, locator: &str, gist: &str, status: Status, observed: Option<u64>) -> Record {
    let mut source = SourceRef::new(SourceKind::Doc, format!("{}#{locator}", tid.canonical()));
    source.observed = observed;
    let core = UnitCoreBuilder::new(KernelType::Claim, gist, status)
        .source(source)
        .build()
        .expect("a unit");
    Record::Unit(core)
}

/// A library with everything the tables can hold: two expressions, one of them superseded, a
/// third part shared by nothing, and units drawn from the text.
///
/// Built by **growing** a library rather than by writing a log, because the whole point of the
/// gate is that growing one and rebuilding from one agree — a fixture log would only exercise
/// the rebuild.
fn grown(tag: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("created");

    // The map goes in before anything is indexed, because the `align` table is rebuilt from
    // what the library holds — a map installed later is a table the next rebuild changes, which
    // is exactly what `on_append`'s align arm does not pretend to catch.
    smysl_text::versification::install(dir.path(), "lines", LINE_MAP).expect("installed a map");

    let notes = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &versified("notes", "notes-lines"),
            &Caps::DEFAULT,
        )
        .expect("added notes");
    assert!(notes.parts.len() >= 3, "the policy should cut nine lines");

    // Everything after this line is maintained **incrementally**, which is what makes the gate
    // a gate. Until TX-P4 step 2 this call was at the end of the fixture, and the consequence
    // was quiet: `Library::open` builds five of the eight tables, so the three it leaves — the
    // two object tables and `postings` — were *rebuilt* on both sides of a comparison that
    // claimed to be rebuild against incremental. The redaction case caught the removal arms
    // because it redacts after completing; nothing exercised the arrival arms. Completing here
    // instead puts a second manifest, three units and a supersession through `on_append` with
    // every table already built.
    lib.complete_index();

    let letter = lib
        .add(
            &Input::new(LETTER.as_bytes()),
            &versified("letter", "letter-lines"),
            &Caps::DEFAULT,
        )
        .expect("added the letter");

    let units = [
        // One unit from the letter, so the postings table has two partitions and one of them
        // holds a single document. That is what the IDF-scope tests need (OQ-14): a scope of one
        // partition with one unit in it is where per-partition IDF degenerates.
        unit_from(
            &letter.parts[0],
            "L2",
            "the engine was repaired",
            Status::Cited,
        ),
        unit_from(&notes.parts[0], "L1", "the notes open", Status::Cited),
        unit_from(&notes.parts[0], "L2", "the second line", Status::Cited),
        unit_at(
            notes.parts.last().expect("a part"),
            "L9",
            "the notes close",
            Status::Measured,
            // One unit with no instant of its own, so the time tables are not uniform: a table
            // where every row looks alike cannot tell a row it should hold from one it should
            // not.
            None,
        ),
    ];
    let Record::Unit(first) = &units[1] else {
        panic!("a unit")
    };
    let dated = smysl_core::canonical_uid(first);
    lib.append_records(&units).expect("appended units");

    // A dating on the `composed` axis, which is the only way that table gets a row: a unit's own
    // fields say when it was *said*, and nothing says when the work it quotes was composed. It
    // also puts the one record type that can move any subject's time through `on_append`, which
    // is the branch where the incremental builder recomputes rather than extends.
    lib.append_records(&[Record::Dating(Dating::new(
        DatingTarget::Unit(dated),
        Axis::Composed,
        DatingValue::Absolute("1890".to_string()),
        agent(),
        Hlc::new(1_726_100_000_000, 0, agent()),
    ))])
    .expect("appended a dating");

    // A second version of `notes`, so `manifests.head` has something to be wrong about and
    // `by_part` has a tid two manifests record. **After** the units, deliberately: the parts it
    // shares with the first version change hands when it unseats the head, so the units
    // extracted from them change partition. A fixture that appended it first would leave the
    // `postings` builder's manifest arm — the one step here that reaches backwards — unexercised,
    // and a probe confirmed that removing the arm then passes the gate.
    lib.append(
        &Input::new(NOTES_V2.as_bytes()),
        "notes",
        None,
        &Caps::DEFAULT,
    )
    .expect("appended notes");

    (dir, lib)
}

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// §3.8: an index rebuilt from the sources alone is byte-identical to one maintained while the
/// library grew.
#[test]
fn rebuild_and_incremental_agree_byte_for_byte() {
    let (_dir, mut lib) = grown("agree");
    let incremental = lib.index().to_bytes();
    let rebuilt = lib.reindex().to_bytes();
    assert_eq!(
        incremental, rebuilt,
        "the two builders disagree about what they would persist"
    );
    // Not vacuous: every table this build fills has something in it. A gate that passed on two
    // empty indexes would pass on no index at all. `redactions` is the exception and is
    // checked in the next test — a grown library has no redaction in it, and putting one here
    // would make this case the redaction case too.
    for table in Table::BUILT.iter().filter(|t| **t != Table::Redactions) {
        assert!(
            lib.index().rows(*table).unwrap_or(0) > 0,
            "{table} is empty, so the comparison did not exercise it"
        );
    }
}

/// The same, with a redaction in the middle — rule Z is the one event that *removes* rows.
#[test]
fn the_two_builders_agree_across_a_redaction() {
    let (_dir, mut lib) = grown("agree-z");
    let tid = *lib
        .index()
        .segments
        .values()
        .next()
        .map(|row| &row.tid)
        .expect("a reading");

    lib.redact(
        &tid,
        &agent(),
        Hlc::new(1_726_500_000_000, 0, agent()),
        None,
    )
    .expect("redacted");

    let incremental = lib.index().to_bytes();
    let rebuilt = lib.reindex().to_bytes();
    assert_eq!(
        incremental, rebuilt,
        "the incremental builder did not learn of the redaction"
    );
    assert!(
        lib.index().redactions.contains_key(&tid),
        "the redaction is not in the table"
    );
    assert!(
        !lib.index().segments.values().any(|row| row.tid == tid),
        "a reading of a redacted part is still in `segments`"
    );
    // With the redaction in, all seven tables this build fills have rows, so the byte
    // comparison above covered every one of them.
    for table in Table::BUILT {
        assert!(
            lib.index().rows(*table).unwrap_or(0) > 0,
            "{table} is empty, so the comparison did not exercise it"
        );
    }
}

// ---------------------------------------------------------------------------
// The exit's other two clauses
// ---------------------------------------------------------------------------

/// Deleting the index restores every derived table.
///
/// §6's clause names `index.redb`; what this build writes is `index/substrate.idx` in the same
/// reserved directory, so the clause is tested against the file there is.
#[test]
fn discarding_the_index_restores_every_derived_table() {
    let (dir, lib) = grown("discard");
    lib.write_index().expect("wrote the sidecar");
    let before = lib.index().to_bytes();
    assert!(Index::path(dir.path()).is_file(), "no sidecar was written");
    drop(lib);

    std::fs::remove_dir_all(dir.path().join(smysl_text::index::DIR)).expect("removed the index");
    let mut reopened = Library::open(dir.path()).expect("reopened");
    // An open builds the five tables it can afford and reports the other two as not built, so
    // this is also where that is asserted: the partial index is *visibly* partial rather than
    // quietly empty, and it answers no locator question.
    assert!(!reopened.index().is_complete());
    assert_eq!(reopened.index().rows(Table::Segments), None);
    assert_eq!(reopened.index().rows(Table::Locators), None);
    assert!(reopened.index().rows(Table::Manifests).unwrap_or(0) > 0);

    reopened.complete_index();
    assert_eq!(
        reopened.index().to_bytes(),
        before,
        "an index rebuilt after deletion is not the index that was deleted"
    );
}

/// A partial index is never persisted, so a sidecar is whole or absent.
///
/// The state this guards against: a library opened without a sidecar, written to, and its
/// half-built index saved — which the next open would load as *current*, since the catalog's
/// hash would match. The completeness flag is in the dump for this reason, and `write_index`
/// reads it.
#[test]
fn a_partial_index_is_not_written_to_the_sidecar() {
    let (dir, _lib) = grown("partial");
    std::fs::remove_dir_all(dir.path().join(smysl_text::index::DIR)).ok();

    let mut reopened = Library::open(dir.path()).expect("reopened");
    assert!(!reopened.index().is_complete());
    reopened
        .write_index()
        .expect("not an error, just not a write");
    assert!(
        !Index::path(dir.path()).is_file(),
        "a partial index reached the disk"
    );

    // Appending to a library whose index is partial leaves it partial, rather than filling the
    // object tables with the parts added since the open and none of the ones that were there.
    let tid = *reopened.index().parts.keys().next().expect("a part");
    reopened
        .append_records(&[unit_from(&tid, "L1", "a later claim", Status::Cited)])
        .expect("appended");
    assert!(!reopened.index().is_complete());
    assert!(reopened.index().segments.is_empty());

    reopened.complete_index();
    reopened.write_index().expect("wrote");
    assert!(Index::path(dir.path()).is_file());
}

/// A sidecar that describes another log is rebuilt, not trusted.
#[test]
fn a_stale_sidecar_is_rebuilt_rather_than_trusted() {
    let (dir, mut lib) = grown("stale");
    lib.write_index().expect("wrote the sidecar");
    let stale = std::fs::read(Index::path(dir.path())).expect("the sidecar");

    // Grow the library past the sidecar and leave the old bytes on disk.
    let tid = *lib.index().parts.keys().next().expect("a part");
    lib.append_records(&[unit_from(&tid, "L1", "a later claim", Status::Cited)])
        .expect("appended");
    let grown_bytes = lib.index().to_bytes();
    std::fs::write(Index::path(dir.path()), &stale).expect("put the stale sidecar back");
    drop(lib);

    let mut reopened = Library::open(dir.path()).expect("reopened");
    assert!(
        !reopened.index().is_complete(),
        "a stale sidecar must be dropped whole, not patched"
    );
    reopened.complete_index();
    assert_eq!(
        reopened.index().to_bytes(),
        grown_bytes,
        "the stale sidecar was trusted"
    );
}

/// A sidecar this build cannot read is a rebuild and never a refusal.
#[test]
fn an_unreadable_sidecar_does_not_stop_a_library_opening() {
    let (dir, lib) = grown("unreadable");
    lib.write_index().expect("wrote the sidecar");
    let good = lib.index().to_bytes();
    drop(lib);

    for junk in [
        b"not an index at all".to_vec(),
        {
            // Right magic, a version nothing here knows.
            let mut b = good.clone();
            b[8] = 0xff;
            b[9] = 0xff;
            b
        },
        good[..good.len() / 2].to_vec(),
    ] {
        assert!(
            Index::from_bytes(&junk).is_err(),
            "that should not have parsed"
        );
        std::fs::write(Index::path(dir.path()), &junk).expect("wrote junk");
        let mut reopened = Library::open(dir.path()).expect("a bad cache is not a bad library");
        reopened.complete_index();
        assert_eq!(reopened.index().to_bytes(), good, "it did not rebuild");
    }
}

/// The journal's recomputable half comes back from the units alone.
#[test]
fn the_journal_is_rebuilt_by_re_checking_stored_units() {
    let (_dir, mut lib) = grown("journal");
    let rows = lib.index().journal.clone();
    assert_eq!(
        rows.len(),
        4,
        "four units name a part: three of the notes and one of the letter"
    );
    assert!(
        rows.windows(2).all(|w| w[0] < w[1]),
        "the journal is not in order"
    );
    assert_eq!(
        rows.iter().filter(|r| r.status == Status::Measured).count(),
        1
    );
    // Every row names a part some manifest records, which is what makes the journal a view of
    // the corpus rather than a list of strings that parsed.
    for row in &rows {
        assert!(lib.index().parts.contains_key(&row.tid), "{:?}", row.tid);
    }
    assert_eq!(lib.reindex().journal, rows, "the rebuild lost a unit");
}

// ---------------------------------------------------------------------------
// The four shapes the gate chose
// ---------------------------------------------------------------------------

/// `head` is a fact about *other* manifests, so a successor's arrival changes an older row.
///
/// The case the incremental builder gets wrong by default: it is handed the new manifest and
/// nothing about the old one, and the field that has to move belongs to the old one.
#[test]
fn the_arrival_of_a_successor_unseats_a_head() {
    let (_dir, mut lib) = grown("head");
    let heads: BTreeSet<_> = lib
        .index()
        .manifests
        .iter()
        .filter(|(_, row)| row.head && row.alias == "notes")
        .map(|(mid, _)| *mid)
        .collect();
    assert_eq!(
        heads.len(),
        1,
        "`notes` was appended to, so it has one head"
    );

    let superseded: Vec<_> = lib
        .index()
        .manifests
        .iter()
        .filter(|(_, row)| !row.head)
        .map(|(mid, _)| *mid)
        .collect();
    assert_eq!(superseded.len(), 1, "exactly one manifest was superseded");
    assert_eq!(
        lib.reindex()
            .manifests
            .iter()
            .filter(|(_, row)| !row.head)
            .map(|(mid, _)| *mid)
            .collect::<Vec<_>>(),
        superseded,
        "the two builders disagree about which manifest is a head"
    );
}

/// `by_part` keeps its mids sorted, and two manifests that agree about a part are one entry.
#[test]
fn by_part_keeps_its_manifests_sorted_and_two_agreeing_records_as_one() {
    let (_dir, lib) = grown("parts");
    // Appending `notes` re-read an unchanged prefix, so some tid is in both manifests.
    let shared: Vec<_> = lib
        .index()
        .by_part
        .iter()
        .filter(|(_, mids)| mids.len() > 1)
        .collect();
    assert!(
        !shared.is_empty(),
        "no part is shared, so there is nothing here to be sorted"
    );
    for (tid, mids) in shared {
        assert!(
            mids.windows(2).all(|w| w[0] < w[1]),
            "`by_part` is not sorted"
        );
        let row = &lib.index().parts[tid];
        // The two manifests agree about this part — same bytes, same reader, same reading — so
        // the set holds one entry. Two agreeing records are one fact.
        assert_eq!(row.entries.len(), 1, "two agreeing records became two");
        assert_eq!(row.lengths().len(), 1);
    }
}

/// A part two **readers** disagree about appears with both records, in identity order.
///
/// The case the `parts` table's shape exists for, and the one the fixture above cannot reach:
/// `txt/1` is the only reader a grown library here uses, so every manifest records every
/// shared part identically and a last-wins table would agree with a rebuild by accident. Two
/// readers over one part is the real case — the same bytes have one tid and two readings — and
/// it is reached here by appending a manifest directly, which is what a second reader's `add`
/// would have produced.
///
/// What the gate catches without this test is nothing; what it catches with it is a table that
/// holds *a* manifest's record of a part rather than the set of them, since a rebuild walks
/// manifests in mid order and an append walks them in arrival order.
#[test]
fn a_part_two_readers_disagree_about_appears_with_both_records() {
    let (_dir, mut lib) = grown("two-readers");
    let mid = lib.head("notes").expect("a head");
    let entry = lib.store().manifest(&mid).expect("a manifest").parts[0].clone();

    // The same part, the same length — the bytes are the bytes — read by something else, so
    // the structure hash and the reading identity differ.
    let other = PartEntry::new(
        entry.tid,
        entry.length,
        [0x5a; 32],
        Rdid::from_bytes([0x5a; 32]),
    );
    let manifest = ManifestBuilder::new(
        "notes-elsewhere",
        LangTag::new("en").expect("a tag"),
        "md/1",
        "CC0-1.0",
        &policy(),
    )
    .expect("a builder")
    .part(other.clone())
    .carry(Carry::Text)
    .build()
    .expect("a manifest");
    let second = manifest.mid();
    lib.append_records(&[Record::Manifest(manifest)])
        .expect("appended");

    let row = &lib.index().parts[&entry.tid];
    assert_eq!(
        row.entries.len(),
        2,
        "two readers disagree about this part and the table holds one record"
    );
    assert!(
        row.entries.windows(2).all(|w| w[0] < w[1]),
        "the set is not in order, so it has an arrival order in it"
    );
    assert_eq!(lib.index().by_part[&entry.tid].len(), 3);

    let incremental = lib.index().to_bytes();
    assert_eq!(
        incremental,
        lib.reindex().to_bytes(),
        "the two builders disagree about a part two manifests record differently"
    );

    // And the reading nothing wrote is simply not in `segments`, which is the leniency rule
    // doing its job rather than a missing object failing an append.
    assert!(!lib.index().segments.contains_key(&other.rdid));
    assert!(
        !lib.index().has_locators(&second),
        "a manifest whose reading cannot be read keeps no locator table"
    );
}

/// The locator table answers exactly what the search answered.
///
/// Not "answers something": the index is not allowed to change a result, so this compares it
/// against the structures it was built from, part by part.
#[test]
fn the_locator_table_answers_what_the_search_answered() {
    let (_dir, lib) = grown("locators");
    let mid = lib.head("notes").expect("a head");
    assert!(
        lib.index().has_locators(&mid),
        "every part read, so the table should be complete"
    );

    for n in 1..=10 {
        let loc = locator::parse(&format!("L{n}")).expect("a locator");
        let from_index = lib.index().resolve(&mid, &loc.to_string());

        // The search, done here rather than by `resolve`, which now consults the table.
        let manifest = lib.store().manifest(&mid).expect("the manifest").clone();
        let mut searched = None;
        for entry in &manifest.parts {
            let structure = lib
                .structure(&mid, &entry.tid, &Caps::DEFAULT)
                .expect("a structure");
            if let Some(range) = structure.resolve(&loc) {
                searched = Some((entry.tid, range));
                break;
            }
        }

        match (from_index, searched) {
            (Some(at), Some((tid, range))) => {
                assert_eq!((at.tid, at.start..at.end), (tid, range), "L{n}");
            }
            (None, None) => panic!("L{n} should resolve in the ten-line version"),
            (a, b) => panic!("L{n}: the index says {a:?} and the search says {b:?}"),
        }
        // And the method the rest of the crate calls agrees with both.
        assert_eq!(
            lib.resolve(&mid, &loc, &Caps::DEFAULT).expect("resolved"),
            from_index.map(|at| (at.tid, at.start..at.end)),
            "L{n}"
        );
    }

    // A locator that names nothing is a miss and not a fallback: the table is complete for
    // this mid, so `None` is an answer.
    let absent = locator::parse("L99").expect("a locator");
    assert_eq!(lib.index().resolve(&mid, &absent.to_string()), None);
    assert_eq!(lib.resolve(&mid, &absent, &Caps::DEFAULT), Ok(None));
}

/// A redacted part makes its manifests incomplete, so `resolve` goes back to the search —
/// which refuses, as it did before the table existed.
#[test]
fn a_redaction_sends_resolve_back_to_the_search_that_refuses() {
    let (_dir, mut lib) = grown("z-resolve");
    let mid = lib.head("notes").expect("a head");
    let tid = lib.store().manifest(&mid).expect("a manifest").parts[0].tid;
    let loc = locator::parse("L1").expect("a locator");
    assert!(lib.resolve(&mid, &loc, &Caps::DEFAULT).is_ok());

    lib.redact(
        &tid,
        &agent(),
        Hlc::new(1_726_500_000_000, 0, agent()),
        None,
    )
    .expect("redacted");

    assert!(
        !lib.index().has_locators(&mid),
        "a manifest with an unreadable part must not keep a locator table"
    );
    assert!(
        lib.resolve(&mid, &loc, &Caps::DEFAULT).is_err(),
        "the index turned a refusal into an answer"
    );
}

// ---------------------------------------------------------------------------
// The table enumeration, and the encoding
// ---------------------------------------------------------------------------

/// Every table of §3.8 is named, and says which TX-P4 step builds it.
#[test]
fn every_table_is_named_and_says_which_step_builds_it() {
    assert_eq!(Table::ALL.len(), 12, "§3.8 names twelve tables");
    assert_eq!(
        Table::BUILT.len(),
        Table::ALL.len(),
        "seven in step 1, postings in step 2, three range tables in step 3, align in step 4 — \
         every table §3.8 names is built"
    );
    assert_eq!(
        Table::ON_OPEN.len(),
        8,
        "an open affords everything but the two object tables and postings"
    );
    for table in Table::ON_OPEN {
        assert!(
            table.is_built(),
            "{table} is built on open and not built at all"
        );
    }

    let mut names: Vec<&str> = Table::ALL.iter().map(|t| t.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), Table::ALL.len(), "two tables share a name");

    for table in Table::ALL {
        assert_eq!(Table::parse(table.as_str()), Some(*table));
        assert!(
            (1..=4).contains(&table.step()),
            "{table} claims step {}",
            table.step()
        );
        assert_eq!(
            table.is_built(),
            table.step() <= 4,
            "{table} is built by step {} and reports is_built() = {}",
            table.step(),
            table.is_built()
        );
        // Every built table has its own bit, and an unbuilt one still has a distinct position:
        // the mask in the sidecar is read by position in `ALL`, so two tables sharing a bit
        // would make a partial index claim the wrong table.
        assert_ne!(table.bit(), 0, "{table} has no bit");
        assert_eq!(
            Table::ALL
                .iter()
                .filter(|other| other.bit() == table.bit())
                .count(),
            1,
            "{table} shares its bit with another table"
        );
    }
    assert_eq!(Table::parse("vectors"), None, "that one is SMYSL-2.5's");

    // A table nobody built reports `None` rather than zero rows: an empty table and a table
    // that does not exist are different sentences. A default index has built nothing at all, so
    // every one of the twelve says so — which is the state `Index::default()` should be in and
    // the state a half-written sidecar would decode into.
    let empty = Index::default();
    assert_eq!(empty.built, Built::empty());
    assert!(!empty.is_complete());
    assert!(
        empty.postings().is_none(),
        "an unbuilt table is not an empty one"
    );
    for table in Table::ALL {
        assert_eq!(empty.rows(*table), None, "{table}");
    }

    // The mask round-trips, and a bit from a build that knows more tables than this one is
    // dropped rather than refused.
    let open = Built::of(Table::ON_OPEN);
    assert!(open.has(Table::Manifests) && !open.has(Table::Postings));
    assert_eq!(open.tables().count(), 8);
    assert_eq!(Built::from_bits(open.bits()), open);
    assert_eq!(Built::from_bits(0xffff), Built::of(Table::ALL));
    assert_eq!(Built::of(Table::ALL), Built::of(Table::BUILT));
    assert!(Built::of(Table::BUILT).is_complete());
    assert!(!open.is_complete());
}

/// The sidecar round-trips.
#[test]
fn the_sidecar_round_trips_through_its_own_encoding() {
    let (_dir, lib) = grown("round-trip");
    let bytes = lib.index().to_bytes();
    let back = Index::from_bytes(&bytes).expect("parsed");
    assert_eq!(back.to_bytes(), bytes, "the encoding is not a round trip");
    assert_eq!(&back, lib.index(), "the decode lost a field");

    // Every prefix is either refused or a shorter index, never a panic. The sidecar is a file
    // on disk and a truncated write is the ordinary way one ends.
    for n in 0..bytes.len() {
        let _ = Index::from_bytes(&bytes[..n]);
    }
}

/// A partition nobody is in any more is gone, not empty.
///
/// The probe that found this: removing `on_append`'s final `retain` leaves a vacated partition
/// behind as a row with no documents, and the byte comparison in `grown`'s fixture does **not**
/// notice — there the supersession moves two of three units and the third keeps the old
/// partition alive. So the case needs a corpus where *every* unit changes hands, which is one
/// unit on a part the successor also records.
#[test]
fn a_partition_nobody_is_in_any_more_is_gone_rather_than_empty() {
    let dir = Scratch::new("vacated");
    let mut lib = Library::create(dir.path()).expect("created");
    let added = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &spec("notes"),
            &Caps::DEFAULT,
        )
        .expect("added");
    lib.complete_index();
    lib.append_records(&[unit_from(
        &added.parts[0],
        "L1",
        "the notes open",
        Status::Cited,
    )])
    .expect("appended a unit");
    let before: Vec<String> = lib.index().postings.keys().map(|p| p.to_string()).collect();
    assert_eq!(before.len(), 1, "one text, one partition");

    lib.append(
        &Input::new(NOTES_V2.as_bytes()),
        "notes",
        None,
        &Caps::DEFAULT,
    )
    .expect("appended a second version");

    let after: Vec<String> = lib.index().postings.keys().map(|p| p.to_string()).collect();
    assert_eq!(
        after.len(),
        1,
        "the unit moved to the successor's partition and the vacated one is still here: {after:?}"
    );
    assert_ne!(
        before, after,
        "the unit did not change hands, so this case is not the case it says it is"
    );
    assert_eq!(
        lib.index().to_bytes(),
        lib.reindex().to_bytes(),
        "a vacated partition left behind is a byte the rebuild would not write"
    );
}

// ---------------------------------------------------------------------------
// The postings table against the index it replaces
// ---------------------------------------------------------------------------

/// The exit clause's second half, from the library's side.
///
/// `tests/postings.rs` in `smysl-retrieve` compares its own two paths; this compares the
/// *persistent* table against the in-memory one built from the same store under the same chain.
/// If these two disagree, the table is wrong even though both scorers are right.
#[test]
fn the_persistent_table_answers_what_an_in_memory_one_does() {
    use smysl_retrieve::{Bm25, MemPostings, Postings as _, Query, Retriever as _};
    use smysl_text::analyze::Chain;
    use smysl_text::index::postings;

    let (_dir, lib) = grown("as-memory");
    let chain = Chain::new(&LangTag::new("en").expect("a tag"));
    assert_eq!(
        postings::analyzers(&lib.index().postings),
        vec![chain.id()],
        "every partition of a single-language library is under that language's chain"
    );

    let view = lib.index().postings().expect("the table is built");
    let memory = MemPostings::index(lib.store(), &chain);

    assert_eq!(
        view.docs(),
        memory.docs(),
        "a different number of documents"
    );
    assert!(
        view.docs() > 0,
        "nothing was indexed, so nothing is compared"
    );
    assert_eq!(view.total_len(), memory.total_len());
    for term in ["the", "notes", "line", "second"] {
        assert_eq!(view.df(term), memory.df(term), "df of `{term}`");
        assert_eq!(
            view.postings(term),
            memory.postings(term),
            "postings of `{term}`"
        );
    }

    // And the scores the two produce, which is what a caller actually sees.
    for text in ["the notes", "second line", "close", "kangaroo"] {
        let q = Query::new(text, 10);
        assert_eq!(
            Bm25::over(&view, &chain, lib.store()).search(&q),
            Bm25::over(&memory, &chain, lib.store()).search(&q),
            "the persistent table and the in-memory one disagree about `{text}`"
        );
    }
    assert!(
        Bm25::over(&view, &chain, lib.store())
            .search(&Query::new("the notes", 10))
            .len()
            >= 2,
        "the comparison matched almost nothing, so it compared almost nothing"
    );
}

/// A partition is the manifest whose reading the units came out of, and it moves with the head.
#[test]
fn a_unit_belongs_to_the_partition_of_the_manifest_that_records_its_part() {
    use smysl_text::index::postings::Partition;

    let (_dir, lib) = grown("partitions");
    let heads: Vec<&Mid> = lib
        .index()
        .manifests
        .iter()
        .filter(|(_, row)| row.head)
        .map(|(mid, _)| mid)
        .collect();
    assert_eq!(heads.len(), 2, "two texts, two heads");

    for partition in lib.index().postings.keys() {
        match partition {
            Partition::Text { mid, lang } => {
                assert_eq!(lang, "en");
                assert!(
                    lib.index().manifests.contains_key(mid),
                    "a partition names a manifest the catalog does not hold"
                );
            }
            // Every unit in the fixture names a part, so a loose partition here would mean the
            // partition lookup stopped finding the part.
            // `Partition` is `#[non_exhaustive]`, so this arm is both the loose case and
            // whatever a later step adds — either way, a unit with a part does not belong here.
            _ => panic!("a unit with a part landed outside its text's partition"),
        }
    }
}

// ---------------------------------------------------------------------------
// IDF scope (OQ-14)
// ---------------------------------------------------------------------------

/// A filter never moves a score; a scope does. That is OQ-14's invariant, and it is the one part
/// of the question that is not a matter of taste.
///
/// `within`, `--kind`, `--min-status`, `--schema` and `--payload` say which units may be
/// *returned*; `from collection:` and `find --library` say what corpus the question is **about**.
/// The first kind leaves every term's weight alone — the rule 1.5 wrote into `Query::within` —
/// and the second is what IDF is computed over. Under that distinction RFC SMYSL-2.5's "default
/// collection" and `within`'s "IDF still comes from the whole index" were never in conflict.
#[test]
fn a_filter_leaves_every_score_alone_and_a_scope_does_not() {
    use smysl_core::KernelType;
    use smysl_retrieve::{Bm25, Postings as _, Query, Retriever as _};
    use smysl_text::analyze::Chain;
    use smysl_text::index::postings::View;

    let (_dir, lib) = grown("idf-scope");
    let chain = Chain::new(&LangTag::new("en").expect("a tag"));
    let table = &lib.index().postings;
    let partitions: Vec<_> = table.keys().cloned().collect();
    // Three: the letter, and the notes' two manifests — the parts the successor shares moved to
    // it and the one it does not record stayed with the first.
    assert_eq!(partitions.len(), 3, "the fixture needs several partitions");

    let library = View::all(table);
    let plain = Bm25::over(&library, &chain, lib.store()).search(&Query::new("the notes", 10));
    assert!(plain.len() >= 2, "the query matched {}", plain.len());

    // A filter: `claim` admits every unit of the fixture, and a status floor admits some of them.
    // Neither may change a surviving hit's score by a single bit.
    for filtered in [
        Query::new("the notes", 10).kinds([KernelType::Claim]),
        Query::new("the notes", 10).min_status(Status::Cited),
        Query::new("the notes", 10).within(plain.iter().map(|h| h.uid)),
    ] {
        let hits = Bm25::over(&library, &chain, lib.store()).search(&filtered);
        for hit in &hits {
            let before = plain
                .iter()
                .find(|h| h.uid == hit.uid)
                .expect("a filter returned a unit the unfiltered query did not");
            assert_eq!(
                hit.score, before.score,
                "a filter re-weighted a term: {} was {} and is {}",
                hit.uid, before.score, hit.score
            );
            assert_eq!(hit.terms, before.terms, "a filter moved a decomposition");
        }
        assert!(
            !hits.is_empty(),
            "the filter admitted nothing, so it proved nothing"
        );
    }

    // A scope: the same query over one partition is a different corpus, so `docs` and therefore
    // every `df`-derived weight differ. This is the half of OQ-14 that is a real choice.
    let one = View::of(table, &partitions[..1]);
    assert!(
        one.docs() < library.docs(),
        "the narrower scope is not narrower"
    );
    let narrow = Bm25::over(&one, &chain, lib.store()).search(&Query::new("the notes", 10));
    let moved = narrow.iter().any(|hit| {
        plain
            .iter()
            .find(|h| h.uid == hit.uid)
            .is_some_and(|before| before.score != hit.score)
    });
    assert!(
        moved,
        "narrowing the scope changed no score, so the scope is not what IDF is computed over"
    );

    // And every scope is exact from the one table, because the counts are over disjoint sets.
    assert_eq!(
        View::of(table, &partitions).docs(),
        library.docs(),
        "the sum of the partitions is not the library"
    );
    assert_eq!(
        partitions
            .iter()
            .map(|p| View::of(table, std::slice::from_ref(p)).docs())
            .sum::<u32>(),
        library.docs()
    );
}

/// Why the default scope is not the partition: a partition of one document weighs every term
/// alike.
///
/// With `N = 1` and `df = 1`, `idf` is `ln(1 + 0.5/1.5)` for every term the document holds — so a
/// term the whole library shares and a term unique to that document contribute **the same**, and
/// the ranking inside the partition is by length normalisation alone. Under RFC SMYSL-2.5 §3.4's
/// RRF fusion that unit then takes rank 1 of its partition and a full `1/(k+1)`, which is how
/// per-partition IDF comes to favour small texts systematically.
#[test]
fn a_one_document_scope_cannot_tell_a_common_term_from_a_rare_one() {
    use smysl_retrieve::{Bm25, Postings as _, Query, Retriever as _};
    use smysl_text::analyze::Chain;
    use smysl_text::index::postings::{Partition, View};

    let (_dir, lib) = grown("idf-degenerate");
    let chain = Chain::new(&LangTag::new("en").expect("a tag"));
    let table = &lib.index().postings;

    // The letter's partition holds the one unit drawn from the letter.
    // The partition with the letter's one unit in it, found by the term only that unit holds —
    // the fixture has another single-document partition, and which of the two this is matters.
    let letter = table
        .iter()
        .find(|(_, rows)| rows.docs() == 1 && rows.df("engine") == 1)
        .map(|(p, _)| p.clone())
        .expect("one partition holds the letter's only unit");
    assert!(matches!(letter, Partition::Text { .. }));

    let contributions = |view: &View<'_>| -> Vec<(String, f32)> {
        Bm25::over(view, &chain, lib.store())
            .search(&Query::new("the engine", 10))
            .into_iter()
            .find(|hit| view.doc_len(&hit.uid).is_some())
            .map(|hit| hit.terms)
            .unwrap_or_default()
    };

    // `the` is in every unit of the fixture and `engine` is in one of them.
    let library = contributions(&View::all(table));
    let common = library
        .iter()
        .find(|(t, _)| t == "the")
        .expect("`the` matched");
    let rare = library
        .iter()
        .find(|(t, _)| t == "engine")
        .expect("`engine` matched");
    assert!(
        rare.1 > common.1 * 4.0,
        "over the library a term in one unit should outweigh one in every unit: {rare:?} vs {common:?}"
    );

    let alone = contributions(&View::of(table, &[letter]));
    let common = alone
        .iter()
        .find(|(t, _)| t == "the")
        .expect("`the` matched");
    let rare = alone
        .iter()
        .find(|(t, _)| t == "engine")
        .expect("`engine` matched");
    assert_eq!(
        common.1, rare.1,
        "a one-document scope distinguished two terms it has no statistics to distinguish"
    );
}
