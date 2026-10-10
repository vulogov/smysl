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
use smysl_core::ids::{AgentId, LangTag, Tid};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{Carry, PartEntry, Record, SourceKind, SourceRef};
use smysl_core::{KernelType, Status, UnitCoreBuilder};
use smysl_text::index::{Index, Table};
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

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

/// A unit whose source names a part, which is what puts a row in the journal.
fn unit_from(tid: &Tid, locator: &str, gist: &str, status: Status) -> Record {
    let core = UnitCoreBuilder::new(KernelType::Claim, gist, status)
        .source(SourceRef::new(
            SourceKind::Doc,
            format!("{}#{locator}", tid.canonical()),
        ))
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

    let notes = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &spec("notes"),
            &Caps::DEFAULT,
        )
        .expect("added notes");
    assert!(notes.parts.len() >= 3, "the policy should cut nine lines");

    lib.add(
        &Input::new(LETTER.as_bytes()),
        &spec("letter"),
        &Caps::DEFAULT,
    )
    .expect("added the letter");

    // A second version of `notes`, so `manifests.head` has something to be wrong about and
    // `by_part` has a tid two manifests record.
    lib.append(
        &Input::new(NOTES_V2.as_bytes()),
        "notes",
        None,
        &Caps::DEFAULT,
    )
    .expect("appended notes");

    lib.append_records(&[
        unit_from(&notes.parts[0], "L1", "the notes open", Status::Cited),
        unit_from(&notes.parts[0], "L2", "the second line", Status::Cited),
        unit_from(
            notes.parts.last().expect("a part"),
            "L9",
            "the notes close",
            Status::Measured,
        ),
    ])
    .expect("appended units");

    // What `text add` leaves behind: the two object-derived tables built once, under the lock,
    // by the command that is already writing. `Library::open` does not build them - see
    // `Index::objects_read` - so a test that did not do this would be testing the partial index.
    lib.complete_index();
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
    assert_eq!(rows.len(), 3, "three units name a part");
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
    assert_eq!(Table::BUILT.len(), 7, "step 1 builds seven of them");

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
            table.step() == 1,
            "{table} is built by step {} and reports is_built() = {}",
            table.step(),
            table.is_built()
        );
    }
    assert_eq!(Table::parse("vectors"), None, "that one is SMYSL-2.5's");

    // A table nobody built reports `None` rather than zero rows: an empty table and a table
    // that does not exist are different sentences.
    // A default index has read no object, so the two tables that need one report themselves as
    // not built — which is the same sentence `postings` gets, for a different reason, and both
    // are better than `0 rows` on a library full of readings.
    let empty = Index::default();
    for table in Table::ALL {
        let expected = table.is_built() && !matches!(table, Table::Segments | Table::Locators);
        assert_eq!(empty.rows(*table).is_some(), expected, "{table}");
    }
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
