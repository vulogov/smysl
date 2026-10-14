//! **The anthology test** — TX-P7 step 2's exit (SMYSL-2.4 §6).
//!
//! > a paragraph reprinted under two expressions forms one class by `identical-span`.
//!
//! That sentence is the whole of the clause, and everything here is in service of making it
//! mean something. The paragraph is real bytes added to a real library twice, under two
//! aliases with two licences and two readers' worth of metadata; the units are stamped by
//! `provenance::stamp`, the one function allowed to write a unit's provenance; the proposal
//! comes from the engine rather than from the test; the edge is staged through rule S; and the
//! class is counted by `smysl_graph::proposition`, which is the normative implementation of
//! A-12.4 and not a second one written here.
//!
//! # What would pass a weaker version of this test, and does not pass this one
//!
//! An engine that proposed every pair of units in the store would form one class too. So the
//! library here holds **two** paragraphs, each reprinted, and the exit asserts two classes of
//! two rather than one class of four: a reprint is the same proposition as its original and
//! not as the other story in the anthology.
//!
//! An engine that keyed on the gist alone would also pass, and would be wrong about every
//! corpus where two different passages are summarised in the same words — so the second
//! paragraph's two units carry the *same gist as each other* and a different tid from the
//! first pair, and one further unit repeats the first pair's gist against the second
//! paragraph's bytes. That unit must not join their class.
//!
//! And an engine that ignored the manifest would propose the two units this test puts on one
//! expression, which is a duplicate extraction rather than a reprint (§3.6).

#![cfg(feature = "text")]

use std::collections::BTreeSet;

use smysl_core::ids::{AgentId, LangTag, Uid};
use smysl_core::types::provenance::{Hlc, Rung};
use smysl_core::types::{Carry, RelKind};
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::proposition::{self, Policy};
use smysl_text::proposition::{Engine, IdenticalSpan, Lexical, SAME_AS};
use smysl_text::provenance::{self, Drawn};
use smysl_text::{AddSpec, Caps, Input, Library};

/// Two paragraphs, each long enough to be a paragraph and short enough to be one part.
const FIRST: &str = "The porter had been at the gate since the second watch and had seen \
nobody pass, which he said twice, and the second time he said it he was already looking at the \
mud on the step.\n";
const SECOND: &str = "A ship is a thing that is owned by somebody who has never seen it, \
sailed by somebody who will never own it, and insured by a third man who hopes very much to \
hear nothing further about it.\n";

struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(what: &str) -> Scratch {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-anthology-{what}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    Scratch(root)
}

/// Add `text` to the library under `alias` and return the manifest and its one part.
fn add(
    lib: &mut Library,
    alias: &str,
    text: &str,
    licence: &str,
) -> (smysl_core::ids::Mid, smysl_core::Tid) {
    let spec = AddSpec::new("txt/1", alias, licence)
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text);
    let added = lib
        .add(&Input::new(text.as_bytes()), &spec, &Caps::DEFAULT)
        .unwrap_or_else(|e| panic!("{alias}: {e}"));
    assert_eq!(
        added.parts.len(),
        1,
        "a paragraph is one part, or this test is about something else"
    );
    (added.mid, added.parts[0])
}

/// A unit drawn from `tid` under `mid`, spanning the whole part, saying `gist`.
fn drawn(
    lib: &Library,
    mid: &smysl_core::ids::Mid,
    tid: &smysl_core::Tid,
    gist: &str,
) -> smysl_core::UnitCore {
    let part = lib.part(tid).expect("the part's bytes");
    let manifest = lib
        .store()
        .manifest(mid)
        .expect("the manifest the library wrote")
        .clone();
    let en = LangTag::new("en").expect("a tag");
    let b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited);
    let b = provenance::stamp(
        b,
        &Drawn {
            tid: *tid,
            locator: Some("p.1"),
            manifest: &manifest,
            observed: None,
            span: Some((0, part.text.len() as u64)),
            lang: Some(&en),
        },
    );
    b.build().expect("a unit")
}

fn hlc() -> Hlc {
    Hlc::new(1, 0, AgentId::new("tool:smysl-test").expect("an agent"))
}

/// The library the exit test runs over, and the uids of the units in it.
struct Anthology {
    _scratch: Scratch,
    lib: Library,
    /// The two units over the first paragraph, under two expressions.
    porter: (Uid, Uid),
    /// The two units over the second paragraph, under two expressions.
    ship: (Uid, Uid),
    /// A unit over the second paragraph carrying the *first* pair's gist.
    impostor: Uid,
    /// A second extraction of the first paragraph under one of its own expressions.
    duplicate: Uid,
}

fn anthology() -> Anthology {
    const PORTER: &str = "The porter saw nobody pass and was looking at the mud.";
    const SHIP: &str = "A ship is owned, sailed and insured by three different men.";

    let s = scratch("exit");
    let mut lib = Library::create(&s.0.join("lib")).expect("a library");

    // Two expressions over the same two paragraphs: the collection each first appeared in,
    // and the anthology that reprinted them. Different aliases, different licences — and the
    // same bytes, so the same tids.
    let (collection_a, porter_a) = add(&mut lib, "collection/porter", FIRST, "public-domain");
    let (anthology_a, porter_b) = add(&mut lib, "anthology/one", FIRST, "CC0-1.0");
    let (collection_b, ship_a) = add(&mut lib, "collection/ship", SECOND, "public-domain");
    let (anthology_b, ship_b) = add(&mut lib, "anthology/two", SECOND, "CC0-1.0");
    assert_eq!(
        porter_a, porter_b,
        "the same paragraph under two manifests is one part, which is what makes this engine exact"
    );
    assert_ne!(porter_a, ship_a, "two paragraphs are two parts");

    let units = vec![
        drawn(&lib, &collection_a, &porter_a, PORTER),
        drawn(&lib, &anthology_a, &porter_b, PORTER),
        drawn(&lib, &collection_b, &ship_a, SHIP),
        drawn(&lib, &anthology_b, &ship_b, SHIP),
        // The same words about different bytes. Nothing but the tid tells these apart.
        drawn(&lib, &collection_b, &ship_a, PORTER),
        // The same words about the same bytes under the *same* manifest: a duplicate
        // extraction. §3.6 excludes it, and this is the unit that shows the exclusion holds.
        drawn(
            &lib,
            &collection_a,
            &porter_a,
            "The porter saw nobody pass at all.",
        ),
    ];
    let uids: Vec<Uid> = units.iter().map(canonical_uid).collect();
    let records: Vec<Record> = units.into_iter().map(Record::Unit).collect();
    lib.append_records(&records).expect("the units append");

    Anthology {
        _scratch: s,
        lib,
        porter: (uids[0], uids[1]),
        ship: (uids[2], uids[3]),
        impostor: uids[4],
        duplicate: uids[5],
    }
}

/// TX-P7 step 2's exit, in the words of §6.
#[test]
fn a_paragraph_reprinted_under_two_expressions_forms_one_class() {
    let mut a = anthology();

    let found = smysl_ingest::same_as::propose(a.lib.store(), &IdenticalSpan, None, hlc());
    assert_eq!(found.engine, "identical-span");
    assert_eq!(
        found.already, 0,
        "nothing was proposed before, so nothing can be held already"
    );
    let staged = found
        .staged
        .as_ref()
        .expect("two reprints are two proposals");
    assert!(
        !staged.has_errors(),
        "the batch did not check clean: {:?}",
        staged.report
    );

    // Exactly the two reprints, and in particular not the fifth unit — whose gist matches the
    // porter pair's word for word and whose bytes are the ship's.
    let pairs: BTreeSet<(Uid, Uid)> = found.proposals.iter().map(|p| (p.a, p.b)).collect();
    let expected: BTreeSet<(Uid, Uid)> = [ordered(a.porter), ordered(a.ship)].into_iter().collect();
    assert_eq!(
        pairs, expected,
        "the engine proposed something other than the two reprints"
    );
    assert!(
        found
            .proposals
            .iter()
            .all(|p| p.a != a.impostor && p.b != a.impostor),
        "the same gist over different bytes is not the same proposition"
    );
    assert!(
        found
            .proposals
            .iter()
            .all(|p| p.a != a.duplicate && p.b != a.duplicate),
        "two extractions under one manifest are a duplicate, not a reprint (§3.6)"
    );
    for p in &found.proposals {
        assert!(
            p.why.contains("under") && p.why.contains(".."),
            "a proposal has to say which bytes and which manifests: {}",
            p.why
        );
    }

    // Through rule S, as any batch is, and then counted by the normative implementation.
    a.lib
        .append_records(&staged.records())
        .expect("the batch commits");

    let classes = proposition::classes(a.lib.store(), Policy::Strict, None);
    assert_eq!(
        classes.len(),
        2,
        "two reprints are two classes, not one class of four: {classes:?}"
    );
    for class in &classes {
        assert_eq!(class.len(), 2, "each class is a paragraph and its reprint");
    }
    let members: BTreeSet<BTreeSet<Uid>> = classes.iter().map(|c| c.members.clone()).collect();
    assert!(
        members.contains(&[a.porter.0, a.porter.1].into_iter().collect()),
        "the porter paragraph and its reprint are one class"
    );
    assert!(
        members.contains(&[a.ship.0, a.ship.1].into_iter().collect()),
        "the ship paragraph and its reprint are one class"
    );

    // Rung and agent, which is what makes `attested:n` answerable at all.
    let rid = a
        .lib
        .store()
        .relations()
        .find(|r| matches!(&r.kind, RelKind::Extension(n) if n == SAME_AS))
        .expect("the edge is in the store")
        .uid();
    let attestations = a.lib.store().attestations_of(&rid);
    assert_eq!(attestations.len(), 1, "one engine, one attestation");
    let one = attestations.iter().next().expect("the attestation");
    assert_eq!(one.rung, Rung::Computed);
    assert_eq!(
        one.agent.as_str(),
        "tool:smysl-same-as-identical-span",
        "§3.6 names the agent after the engine"
    );
    assert_eq!(
        proposition::classes(a.lib.store(), Policy::Attested(2), None).len(),
        0,
        "one engine is one agent, so `attested:2` admits nothing — which is §3.6's default \
         for corpus measures and the reason it is not satisfied by a single run"
    );
}

/// Running the engine again finds the same two pairs and stages nothing.
///
/// A rid is derived from the kind and the endpoints, so the second run produces byte-identical
/// records — and a run that staged them again would report two new edges where there are none.
#[test]
fn a_second_run_proposes_the_same_pairs_and_stages_nothing() {
    let mut a = anthology();
    let first = smysl_ingest::same_as::propose(a.lib.store(), &IdenticalSpan, None, hlc());
    a.lib
        .append_records(&first.staged.expect("a batch").records())
        .expect("the batch commits");

    let again = smysl_ingest::same_as::propose(a.lib.store(), &IdenticalSpan, None, hlc());
    assert_eq!(
        again.proposals, first.proposals,
        "the engine is deterministic"
    );
    assert_eq!(again.already, 2, "the store already holds both edges");
    assert!(again.staged.is_none(), "nothing new to stage");
    assert_eq!(again.new_edges(), 0);
}

/// A scope restricts the units considered, and `None` is every unit.
#[test]
fn a_scope_restricts_what_is_proposed() {
    let a = anthology();
    let only_the_ship: BTreeSet<Uid> = [a.ship.0, a.ship.1].into_iter().collect();
    let found =
        smysl_ingest::same_as::propose(a.lib.store(), &IdenticalSpan, Some(&only_the_ship), hlc());
    assert_eq!(found.proposals.len(), 1, "one pair is in scope");
    assert_eq!(
        (found.proposals[0].a, found.proposals[0].b),
        ordered(a.ship)
    );
}

/// `lexical` finds the reprints too, and from different evidence.
///
/// Not an exit clause — the exit names `identical-span` — but the engine that is *not* exact
/// should agree with the one that is on a case this easy, and a store where it did not would be
/// a store where its filter is wrong rather than strict.
#[test]
fn the_lexical_engine_agrees_on_the_easy_case() {
    let a = anthology();
    let found = smysl_ingest::same_as::propose(a.lib.store(), &Lexical::default(), None, hlc());
    let pairs: BTreeSet<(Uid, Uid)> = found.proposals.iter().map(|p| (p.a, p.b)).collect();
    assert!(
        pairs.contains(&ordered(a.porter)),
        "the porter pair has the same gist and the same locator: {pairs:?}"
    );
    assert!(
        pairs.contains(&ordered(a.ship)),
        "and so does the ship pair: {pairs:?}"
    );
    // Adjacency is what keeps the impostor out of `identical-span`'s result by accident and
    // out of this one on purpose: it is at `p.1` of the ship's part, so it *is* adjacent to
    // the ship pair — and it says something else, which is why BM25 is the filter there.
    assert!(
        found.proposals.iter().any(|p| p.why.contains("BM25")),
        "a lexical proposal says what found it"
    );
}

/// `--engine lexical` without adjacency is the configuration S0's 0.789 was measured in.
#[test]
fn adjacency_can_be_turned_off_so_the_measured_figure_is_reproducible() {
    let a = anthology();
    let filtered = Lexical::default();
    assert!(filtered.adjacent_only, "on by default (§3.6)");
    let unfiltered = Lexical {
        adjacent_only: false,
        ..Lexical::default()
    };
    let with = filtered.propose(a.lib.store(), None).len();
    let without = unfiltered.propose(a.lib.store(), None).len();
    assert!(
        without >= with,
        "a filter cannot admit more than no filter: {without} < {with}"
    );
}

fn ordered((x, y): (Uid, Uid)) -> (Uid, Uid) {
    if x <= y {
        (x, y)
    } else {
        (y, x)
    }
}
