//! TX-P4 step 4's exit clause: KJV ↔ Synodal, Psalm 14 ↔ Psalm 13.
//!
//! # What is real here and what is not
//!
//! The **numbering** is the fact under test and it is a published convention, not a dataset:
//! from Psalm 10 to Psalm 147 the Hebrew numbering and the Septuagint numbering the Synodal
//! Bible follows differ by one, so the psalm an English Bible calls 14 a Russian Bible calls 13.
//! The map in this file states that for the two psalms it touches, and nothing else.
//!
//! The **verse text** is synthetic, and that is said rather than smuggled: the KJV and the
//! Synodal translation are texts this repository does not hold, which is GE-T1's standing debt
//! (§5.5), and a fixture that recited them from memory would be a corpus claim nobody could
//! check. What an alignment maps is locators, so the content behind them is irrelevant to the
//! claim and a line saying which verse it is standing in is the honest filling.
//!
//! OQ-4 — the source and licence of *shipped* versification maps — is therefore untouched by
//! this test and stays open. A map authored here is a map installed in a library, which is the
//! mechanism; a map distributed with the crate is a dataset, which is the question.
//!
//! # The two mechanisms the map exercises
//!
//! `Ps.14 → Ps.13` is a **chapter row**, and `Ps.14.1` reaches `Ps.13.1` through the carry: most
//! of a versification difference is a whole-chapter offset, and a map that had to state every
//! verse of every psalm would be thousands of rows to say one thing. Psalm 51 is the other case
//! — in the Septuagint numbering the Hebrew title is verse 1, so every later verse moves as well
//! — and its verses are stated outright, which is the exact row beating the carry.

#![cfg(feature = "reader-usfm")]

use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Mid};
use smysl_core::types::Carry;
use smysl_text::index::align;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::locator;
use smysl_text::readers::Input;
use smysl_text::versification::{self, Scheme};

/// Hebrew numbering: 14, 51 and a 15 the map says nothing about.
const KJV: &str = "\\
\\id PSA The Book of Psalms
\\h Psalms
\\c 14
\\p
\\v 1 synthetic text standing in for the first verse of the fourteenth psalm
\\v 2 synthetic text standing in for the second verse of the fourteenth psalm
\\v 3 synthetic text standing in for the third verse of the fourteenth psalm
\\c 15
\\p
\\v 1 synthetic text standing in for the first verse of the fifteenth psalm
\\c 51
\\p
\\v 1 synthetic text standing in for the first verse of the fifty-first psalm
\\v 2 synthetic text standing in for the second verse of the fifty-first psalm
\\v 3 synthetic text standing in for the third verse of the fifty-first psalm
";

/// Septuagint numbering: the same three psalms, each one lower, and 50 with the title as verse 1.
const SYNODAL: &str = "\\
\\id PSA The Book of Psalms
\\h Psalms
\\c 13
\\p
\\v 1 synthetic text standing in for the first verse of the thirteenth psalm
\\v 2 synthetic text standing in for the second verse of the thirteenth psalm
\\v 3 synthetic text standing in for the third verse of the thirteenth psalm
\\c 14
\\p
\\v 1 synthetic text standing in for the first verse of the fourteenth psalm
\\c 50
\\p
\\v 1 synthetic text standing in for the title, which this numbering counts as a verse
\\v 2 synthetic text standing in for the second verse of the fiftieth psalm
\\v 3 synthetic text standing in for the third verse of the fiftieth psalm
\\v 4 synthetic text standing in for the fourth verse of the fiftieth psalm
";

/// The map. Two chapter rows and three verse rows, which is the whole of what it claims.
const MAP: &str = "smysl/versification/1
from\tkjv
to\tsynodal
# The psalms shift by one from 10 to 147; these two are inside that range.
Ps.14\tPs.13
Ps.51\tPs.50
# Psalm 51's verses move as well, because the Septuagint numbering counts the Hebrew title.
Ps.51.1\tPs.50.2
Ps.51.2\tPs.50.3
Ps.51.3\tPs.50.4
";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("smysl-align-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
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

fn spec(alias: &str, lang: &str, scheme: &str) -> AddSpec {
    let mut spec = AddSpec::new("usfm/1", alias, "public-domain")
        .with_lang(LangTag::new(lang).expect("a tag"))
        .with_carry(Carry::Text);
    spec.versification = Some(scheme.to_string());
    spec
}

/// Two psalters in two numberings, with the map installed.
fn two_psalters(tag: &str) -> (Scratch, Library, Mid, Mid) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("created");
    versification::install(dir.path(), "kjv-synodal", MAP).expect("installed the map");

    lib.add(
        &Input::new(KJV.as_bytes()),
        &spec("kjv", "en", "kjv"),
        &Caps::DEFAULT,
    )
    .expect("added the hebrew-numbered psalter");
    lib.add(
        &Input::new(SYNODAL.as_bytes()),
        &spec("synodal", "ru", "synodal"),
        &Caps::DEFAULT,
    )
    .expect("added the septuagint-numbered psalter");
    lib.complete_index();

    let kjv = lib.head("kjv").expect("a head");
    let synodal = lib.head("synodal").expect("a head");
    (dir, lib, kjv, synodal)
}

/// The clause.
#[test]
fn psalm_fourteen_aligns_with_psalm_thirteen() {
    let (_dir, lib, kjv, synodal) = two_psalters("psalms");
    let table = &lib.index().align;

    assert_eq!(
        align::locate(table, &kjv, &synodal, "Ps.14"),
        Some("Ps.13".to_string()),
        "the chapter the clause names is not aligned"
    );
    for verse in 1..=3 {
        assert_eq!(
            align::locate(table, &kjv, &synodal, &format!("Ps.14.{verse}")),
            Some(format!("Ps.13.{verse}")),
            "verse {verse} of the fourteenth psalm"
        );
    }

    // Both sides resolve to text, which is what makes the alignment an address rather than a
    // string rewrite: the two locators name places in two different parts of two different
    // expressions, and each of them is somewhere.
    let here = lib
        .passage(
            "kjv",
            &locator::parse("Ps.14.1").expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("the fourteenth psalm's first verse");
    let there = lib
        .passage(
            "synodal",
            &locator::parse("Ps.13.1").expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("the thirteenth psalm's first verse");
    assert_ne!(here.tid, there.tid, "the two psalters share a part");
    assert!(here.text.contains("fourteenth"), "{}", here.text);
    assert!(there.text.contains("thirteenth"), "{}", there.text);
}

/// An exact row beats the chapter carry, which is what a shifted verse numbering needs.
#[test]
fn a_stated_verse_beats_the_chapter_carry() {
    let (_dir, lib, kjv, synodal) = two_psalters("carry");
    let table = &lib.index().align;

    // The carry alone would say `Ps.50.1`, and the map says otherwise because the Septuagint
    // numbering counts the title.
    assert_eq!(
        align::locate(table, &kjv, &synodal, "Ps.51.1"),
        Some("Ps.50.2".to_string())
    );
    assert_eq!(
        align::locate(table, &kjv, &synodal, "Ps.51.3"),
        Some("Ps.50.4".to_string())
    );
    // And the chapter itself still aligns by its own row.
    assert_eq!(
        align::locate(table, &kjv, &synodal, "Ps.51"),
        Some("Ps.50".to_string())
    );
}

/// The reverse direction is in the table, built from the same map.
#[test]
fn the_table_holds_both_directions() {
    let (_dir, lib, kjv, synodal) = two_psalters("both");
    let table = &lib.index().align;

    assert_eq!(
        align::locate(table, &synodal, &kjv, "Ps.13.2"),
        Some("Ps.14.2".to_string()),
        "the reverse of a chapter row"
    );
    assert_eq!(
        align::locate(table, &synodal, &kjv, "Ps.50.2"),
        Some("Ps.51.1".to_string()),
        "the reverse of a stated verse"
    );
    assert_eq!(table.len(), 2, "two manifests, two ordered pairs");
}

/// `SMY-W451`: a locator the scheme does not cover is counted, not quietly dropped.
#[test]
fn an_uncovered_locator_is_counted() {
    let (_dir, lib, kjv, synodal) = two_psalters("uncovered");
    let table = &lib.index().align;
    let rows = table.get(&(kjv, synodal)).expect("the forward pair");

    assert!(
        rows.rows.len() >= 8,
        "only {} rows aligned",
        rows.rows.len()
    );
    // Psalm 15 and the book-level locator are not in the map, and the map's own `Ps.51.*` rows
    // have targets the other psalter holds — so the count is the uncovered ones and no more.
    assert!(
        rows.uncovered >= 2,
        "the fifteenth psalm and the book locator are not covered, and {} was counted",
        rows.uncovered
    );
    assert_eq!(
        rows.scheme, "kjv-synodal",
        "the map is not named in the row"
    );
    assert!(
        align::locate(table, &kjv, &synodal, "Ps.15.1").is_none(),
        "a locator the map says nothing about was aligned anyway"
    );
    assert!(align::uncovered(table) >= rows.uncovered);
}

/// The table is derived from the library, so discarding it loses nothing.
#[test]
fn discarding_the_table_rebuilds_it() {
    let (_dir, mut lib, kjv, synodal) = two_psalters("rebuild");
    let before = lib.index().align.clone();
    let rebuilt = lib.reindex().align.clone();
    assert_eq!(before, rebuilt, "the rebuild produced a different table");
    assert_eq!(
        align::locate(&rebuilt, &kjv, &synodal, "Ps.14.1"),
        Some("Ps.13.1".to_string())
    );
}

/// A map nobody installed aligns nothing, rather than aligning by guess.
#[test]
fn without_a_map_nothing_is_aligned() {
    let dir = Scratch::new("nomap");
    let mut lib = Library::create(dir.path()).expect("created");
    lib.add(
        &Input::new(KJV.as_bytes()),
        &spec("kjv", "en", "kjv"),
        &Caps::DEFAULT,
    )
    .expect("added");
    lib.add(
        &Input::new(SYNODAL.as_bytes()),
        &spec("synodal", "ru", "synodal"),
        &Caps::DEFAULT,
    )
    .expect("added");
    lib.complete_index();
    assert!(
        lib.index().align.is_empty(),
        "two numberings and no map between them produced rows"
    );
}

/// A manifest that declares no numbering is not aligned, because it has not said anything to
/// align against.
#[test]
fn a_manifest_with_no_declaration_is_not_aligned() {
    let dir = Scratch::new("undeclared");
    let mut lib = Library::create(dir.path()).expect("created");
    versification::install(dir.path(), "kjv-synodal", MAP).expect("installed the map");
    let mut undeclared = spec("kjv", "en", "kjv");
    undeclared.versification = None;
    lib.add(&Input::new(KJV.as_bytes()), &undeclared, &Caps::DEFAULT)
        .expect("added");
    lib.add(
        &Input::new(SYNODAL.as_bytes()),
        &spec("synodal", "ru", "synodal"),
        &Caps::DEFAULT,
    )
    .expect("added");
    lib.complete_index();
    assert!(lib.index().align.is_empty());
}

/// The map format, at its edges.
#[test]
fn a_map_is_read_strictly_or_refused() {
    assert!(Scheme::parse("x", "not a map\nfrom\ta\nto\tb\n").is_err());
    assert!(Scheme::parse("x", "smysl/versification/1\nfrom\ta\n").is_err());
    assert!(
        Scheme::parse("x", "smysl/versification/1\nfrom\ta\nto\ta\n").is_err(),
        "a map from a numbering to itself is not a map"
    );
    assert!(
        Scheme::parse("x", "smysl/versification/1\nfrom\ta\nto\tb\nPs.1 Ps.2\n").is_err(),
        "a row separated by a space is not a row"
    );
    assert!(
        Scheme::parse(
            "x",
            "smysl/versification/1\nfrom\ta\nto\tb\nPs.1\tPs.2\nPs.1\tPs.3\n"
        )
        .is_err(),
        "a locator mapped twice is a disagreement, not a last-wins"
    );

    let good = Scheme::parse("x", MAP).expect("the fixture map parses");
    assert_eq!(good.len(), 5);
    assert!(good.connects("kjv", "synodal"));
    assert!(!good.connects("synodal", "kjv"));
    assert_eq!(good.map("Ps.14.9"), Some("Ps.13.9".to_string()));
    assert_eq!(good.map("Ps.14"), Some("Ps.13".to_string()));
    // One level of carry and no more: a book-level row would otherwise renumber verses through
    // two levels of guessing.
    assert_eq!(good.map("Ps.99.1"), None);
    assert_eq!(good.map("Gen.1.1"), None);

    let back = good.reversed().expect("the fixture map is injective");
    assert!(back.connects("synodal", "kjv"));
    assert_eq!(back.map("Ps.13.4"), Some("Ps.14.4".to_string()));

    // A map two of whose rows point at one locator has no reverse, because a reverse would have
    // to pick one of them.
    let ambiguous = Scheme::parse(
        "y",
        "smysl/versification/1\nfrom\ta\nto\tb\nPs.1\tPs.9\nPs.2\tPs.9\n",
    )
    .expect("it parses");
    assert!(ambiguous.reversed().is_none());
}

/// Installing a map writes it where a rebuild will find it, and refuses a malformed one.
#[test]
fn installing_a_map_puts_it_where_the_rebuild_looks() {
    let dir = Scratch::new("install");
    assert!(versification::installed(dir.path()).is_empty());
    versification::install(dir.path(), "kjv-synodal", MAP).expect("installed");
    assert!(versification::path(dir.path(), "kjv-synodal").exists());

    let found = versification::installed(dir.path());
    assert_eq!(found.len(), 1);
    assert_eq!(found["kjv-synodal"].len(), 5);

    assert!(
        versification::install(dir.path(), "broken", "nonsense\n").is_err(),
        "a malformed map was installed"
    );
    assert_eq!(
        versification::installed(dir.path()).len(),
        1,
        "a refused map left a file behind for every later rebuild to skip"
    );
}

// ---------------------------------------------------------------------------
// The `anchored` same-as engine over this alignment (TX-P7 step 2, §3.6)
// ---------------------------------------------------------------------------

/// A unit drawn from `alias` at `locator`, saying `gist`.
///
/// Through `provenance::stamp`, so the reference and the manifest are written by the one
/// function allowed to write them — an engine reading a reference the test had assembled by
/// hand would be an engine tested against this file's spelling of A-2.2.
fn unit_at(lib: &Library, alias: &str, locator_text: &str, gist: &str) -> smysl_core::UnitCore {
    use smysl_core::{KernelType, Status, UnitCoreBuilder};
    use smysl_text::provenance::{stamp, Drawn};

    let mid = lib.head(alias).expect("a head");
    let manifest = lib.store().manifest(&mid).expect("its manifest").clone();
    let at = lib
        .passage(
            alias,
            &locator::parse(locator_text).expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("the passage");
    let lang = manifest.lang.clone();
    let b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited);
    stamp(
        b,
        &Drawn {
            tid: at.tid,
            locator: Some(locator_text),
            manifest: &manifest,
            observed: None,
            span: Some((at.range.start, at.range.end)),
            lang: Some(&lang),
        },
    )
    .build()
    .expect("a unit")
}

/// The engine proposes across the alignment and nowhere else.
///
/// This is the clause §3.6 means by "`anchored` alone is used for the Bibles, and it is also
/// the right engine there": the two gists share no term, so `lexical` cannot see the pair at
/// all, and the alignment sees it exactly. The two psalters' parts have different tids, so
/// `identical-span` cannot see it either — which is the whole reason there are three engines.
#[test]
fn the_anchored_engine_proposes_across_the_versification() {
    use smysl_text::proposition::{Anchored, Engine, IdenticalSpan, Lexical};

    let (_dir, mut lib, _kjv, _synodal) = two_psalters("anchored");

    let english = unit_at(
        &lib,
        "kjv",
        "Ps.14.1",
        "The fool says in his heart there is no God.",
    );
    let russian = unit_at(
        &lib,
        "synodal",
        "Ps.13.1",
        "Сказал безумец в сердце своём: нет Бога.",
    );
    // A third place, aligned to nothing in the map: Psalm 15 has no row, so no proposal may
    // reach it however much it looks like the others.
    let unaligned = unit_at(
        &lib,
        "kjv",
        "Ps.15.1",
        "The fool says in his heart there is no God.",
    );
    let (a, b, c) = (
        smysl_core::canonical_uid(&english),
        smysl_core::canonical_uid(&russian),
        smysl_core::canonical_uid(&unaligned),
    );
    lib.append_records(&[
        smysl_core::Record::Unit(english),
        smysl_core::Record::Unit(russian),
        smysl_core::Record::Unit(unaligned),
    ])
    .expect("the units append");

    let table = lib.index().align.clone();
    let found = Anchored::new(&table).propose(lib.store(), None);
    let pairs: Vec<(_, _)> = found.iter().map(|p| (p.a, p.b)).collect();
    let expected = if a <= b { (a, b) } else { (b, a) };
    assert_eq!(
        pairs,
        vec![expected],
        "exactly the aligned pair: {:#?}",
        found
    );
    assert!(
        found[0].why.contains("Ps.14.1") && found[0].why.contains("Ps.13.1"),
        "a proposal has to name the two places: {}",
        found[0].why
    );
    assert!(
        found[0].why.contains("kjv-synodal"),
        "and the map that aligned them: {}",
        found[0].why
    );
    assert!(
        !found.iter().any(|p| p.a == c || p.b == c),
        "Psalm 15 is in no row of the map"
    );

    // The other two engines, on the same store, for the reason there are three of them.
    assert!(
        IdenticalSpan.propose(lib.store(), None).is_empty(),
        "two psalters do not share a part"
    );
    assert!(
        Lexical::default().propose(lib.store(), None).is_empty(),
        "an English gist and a Russian one share no term, and are in different language groups"
    );
}
