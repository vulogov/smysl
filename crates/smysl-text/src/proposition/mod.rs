//! Same-as engines: what *proposes* the edges a class is built from (SMYSL-2.4 §3.6, TX-P7
//! step 2).
//!
//! A class is counted in [`smysl_graph::proposition`], and the reason it is counted there is
//! written at the top of that module: counting needs records and nothing else, and
//! `smysl-check` has to be able to reach it. Proposing is the other half, and it needs what
//! this crate has — retrieval for `lexical`, a versification alignment for `anchored` — so it
//! is here, as §4.1's tree places it.
//!
//! # Three engines, and why three rather than one good one
//!
//! Each answers "are these two units the same proposition?" from different evidence, and the
//! S0 spike (SMYSL-2.2 §3.7) measured what each is worth:
//!
//! | engine | evidence | what S0 found |
//! |---|---|---|
//! | [`IdenticalSpan`] | the same bytes of the same part, read under two manifests | exact; S0's C0 was auto-accepted at rung `computed` |
//! | [`Lexical`] | BM25 neighbours in the same language, filtered by adjacency | precision 0.789 → **0.835** with adjacency |
//! | [`Anchored`] | a versification alignment says the two places are one place | the cross-lingual engine, 0.427 → **0.712** with adjacency |
//!
//! None of them reached the 0.9 bar draft 1 set, which is why §3.6 says class measures ship as
//! exploration only and corpus measures default to `attested:2`. That is a statement about
//! what has been measured against a gold of α 0.600, not a verdict on the engines — and
//! TX-P7's own exit (GE-T5, §5.5) is the restated question: whether an engine is at the
//! ceiling the coders themselves are at.
//!
//! # Proposing is not accepting
//!
//! Every engine here is **pure** and none of them writes anything. A proposal becomes an edge
//! when something stages it — `smysl_ingest::same_as` — and the edge then carries an
//! attestation at rung `computed` by the engine's own agent, so a store can always be asked
//! which engine said so and `attested:n` can count engines that agree. OQ-44 is the open
//! question of whether a `lexical` proposal may be auto-accepted; this module takes no
//! position, because a module that accepted its own proposals would make that question
//! unaskable.
//!
//! # Two departures from §4.2's sketch, both about what a caller can check
//!
//! The sketch is `fn propose(&self, store: &Store, scope: &BTreeSet<Uid>) -> Vec<(Uid, Uid)>`.
//!
//! `scope` is an [`Option`], for the reason [`smysl_graph::proposition::classes`] documents at
//! length: with a bare set the empty one has to mean either "every unit" or "no unit" and both
//! readings are wrong for somebody.
//!
//! And a proposal is a [`Proposal`] rather than a pair, because a pair cannot say *why*. An
//! engine that proposed 4,000 edges over five Bibles is either finding the alignment or
//! finding that every verse looks like every other verse, and the difference is one sentence
//! per edge — which `same-as propose` prints, an adjudicator reads, and GE-T5's sheet needs.
//! Carrying it costs a `String` per proposal and buys the only form of review these engines
//! can get before OQ-44 is settled.

use std::collections::BTreeSet;

use smysl_core::ids::{AgentId, Uid};
use smysl_graph::Store;

pub mod anchored;
pub mod identical;
pub mod lexical;

pub use anchored::Anchored;
pub use identical::IdenticalSpan;
pub use lexical::Lexical;

/// Counting is `smysl-graph`'s, and this is where callers of the engines reach for it.
///
/// A re-export and not a second implementation: two spellings of A-12.4 would be two answers
/// to "how many propositions does this store hold?", which is the denominator of every corpus
/// measure.
pub use smysl_graph::proposition::{classes, Class, Policy, SAME_AS};

/// One claim that two units are the same proposition, with the evidence that produced it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Proposal {
    /// The smaller uid.
    ///
    /// `x.text/same-as` is symmetric, so a proposal that could arrive either way round would
    /// be two proposals for one claim and `attested:n` would count one engine as two. The
    /// order is the uid order, which is the order A-12.4 already works in.
    pub a: Uid,
    /// The larger uid.
    pub b: Uid,
    /// Why this engine says so, in one line, for a person deciding whether to accept it.
    pub why: String,
}

impl Proposal {
    /// A proposal between two distinct units, in uid order.
    ///
    /// `None` for a self-pair: a unit is the same proposition as itself, which is true, is not
    /// a claim, and is excluded from the vertex set by A-12.4 anyway.
    pub fn new(x: Uid, y: Uid, why: impl Into<String>) -> Option<Proposal> {
        use std::cmp::Ordering;
        let (a, b) = match x.cmp(&y) {
            Ordering::Less => (x, y),
            Ordering::Greater => (y, x),
            Ordering::Equal => return None,
        };
        Some(Proposal {
            a,
            b,
            why: why.into(),
        })
    }
}

/// An engine that proposes `x.text/same-as` edges over a store.
pub trait Engine {
    /// The engine's name, as `same-as propose --engine` spells it.
    fn id(&self) -> &'static str;

    /// Every proposal the engine makes, in [`Proposal`] order and without duplicates.
    ///
    /// `scope` restricts the units considered; `None` is every unit the store holds.
    fn propose(&self, store: &Store, scope: Option<&BTreeSet<Uid>>) -> Vec<Proposal>;

    /// The agent an accepted proposal is attested by: `tool:smysl-same-as-<id>` (§3.6).
    ///
    /// Provided rather than per engine, so that the spelling cannot drift between three
    /// implementations of one sentence of the RFC. Unverified in A-9's sense — nothing signs a
    /// computation — and `computed` is the rung that says so.
    fn agent(&self) -> AgentId {
        AgentId::new(format!("tool:smysl-same-as-{}", self.id()))
            .expect("an engine id is lowercase ASCII with hyphens, which is an agent id tail")
    }
}

/// The units an engine looks at: in scope, present, and in uid order.
///
/// Shared by all three engines because all three have the same answer to "which units?", and a
/// disagreement between them about that would make one engine's proposals incomparable with
/// another's — which is exactly what `attested:n` reads.
pub(crate) fn candidates<'s>(
    store: &'s Store,
    scope: Option<&BTreeSet<Uid>>,
) -> Vec<(Uid, &'s smysl_core::types::Unit)> {
    store
        .units()
        .filter(|(uid, _)| scope.is_none_or(|s| s.contains(*uid)))
        .map(|(uid, u)| (*uid, u))
        .collect()
}

/// Where a unit says it came from, as A-2.2 spells it into one string.
///
/// All three engines ask the same four questions of a unit's `source` — which part, which
/// manifest, which named place, which bytes — so they ask them in one place. An engine that
/// parsed the reference itself would be a second reading of A-2.2's `t3:…#locator` grammar,
/// and the engines' proposals are meant to be comparable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Place {
    pub tid: String,
    pub mid: Option<smysl_core::ids::Mid>,
    pub locator: Option<String>,
    pub span: Option<(u64, u64)>,
}

/// A unit's place, or `None` for a unit with no `source` at all.
pub(crate) fn place(unit: &smysl_core::types::Unit) -> Option<Place> {
    let source = unit.core.source.as_ref()?;
    let (tid, locator) = match source.reference.split_once('#') {
        Some((t, l)) => (t.to_string(), Some(l.to_string())),
        None => (source.reference.clone(), None),
    };
    Some(Place {
        tid,
        mid: source.manifest().cloned(),
        locator,
        span: source.span(),
    })
}

/// Whether two locators name the same place or neighbouring ones (S0's C3).
///
/// Same stem, and a final component that is equal or one apart: `Gen.3.5` is adjacent to
/// `Gen.3.4` and `Gen.3.6`, and not to `Gen.4.5` or `Ex.3.5`. S0 measured this filter lifting
/// cross-lingual precision from 0.427 to 0.712, which is why §3.6 calls it load-bearing rather
/// than a refinement: it does not rank better, it removes the pairs a ranker is worst at.
///
/// A chapter boundary is a real boundary here. `Gen.3.24` and `Gen.4.1` are consecutive verses
/// of scripture and are **not** adjacent by this rule, because the rule cannot know that
/// Genesis 3 has 24 verses without reading the structure — and a rule that guessed would be
/// wrong at the end of every chapter in the other direction. Shrinking the candidate set is
/// the whole point; missing the one pair that straddles a chapter break costs less than
/// admitting every pair whose numbers happen to be 1 apart.
pub(crate) fn adjacent_locators(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let split = |s: &str| -> Option<(String, u64)> {
        let (stem, last) = s.rsplit_once('.')?;
        Some((stem.to_string(), last.parse().ok()?))
    };
    match (split(a), split(b)) {
        (Some((sa, na)), Some((sb, nb))) => sa == sb && na.abs_diff(nb) == 1,
        _ => false,
    }
}

/// The engine `name` spells, or `None`.
///
/// `anchored` is absent here because it needs an alignment table the caller has to supply;
/// `semantic` is absent because it is SMYSL-2.5's and naming it here would make a reserved
/// word look like an unimplemented one.
pub fn engine(name: &str) -> Option<Box<dyn Engine>> {
    match name {
        "identical-span" => Some(Box::new(IdenticalSpan)),
        "lexical" => Some(Box::new(Lexical::default())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proposal_is_in_uid_order_whichever_way_it_was_made() {
        let a = Uid::from_bytes([1; 32]);
        let b = Uid::from_bytes([2; 32]);
        let one = Proposal::new(a, b, "because").expect("two distinct units");
        let other = Proposal::new(b, a, "because").expect("two distinct units");
        assert_eq!(one, other, "a symmetric claim has one spelling");
        assert_eq!(one.a, a);
    }

    #[test]
    fn a_unit_is_not_proposed_against_itself() {
        let a = Uid::from_bytes([7; 32]);
        assert!(Proposal::new(a, a, "trivially").is_none());
    }

    #[test]
    fn a_locator_is_adjacent_to_itself_and_to_its_neighbours() {
        assert!(adjacent_locators("Gen.3.5", "Gen.3.5"));
        assert!(adjacent_locators("Gen.3.5", "Gen.3.4"));
        assert!(adjacent_locators("Gen.3.5", "Gen.3.6"));
        assert!(!adjacent_locators("Gen.3.5", "Gen.3.7"));
        assert!(
            !adjacent_locators("Gen.3.5", "Gen.4.5"),
            "a different chapter"
        );
        assert!(!adjacent_locators("Gen.3.5", "Ex.3.5"), "a different book");
        // The chapter seam, which this rule deliberately does not cross: see the function's
        // own docs for why a rule that guessed Genesis 3 has 24 verses would be worse.
        assert!(!adjacent_locators("Gen.3.24", "Gen.4.1"));
        // Nothing to compare: a locator with no final numeric component is not adjacent to
        // anything but itself.
        assert!(!adjacent_locators("Gen", "Ex"));
        assert!(adjacent_locators("Gen", "Gen"));
    }

    #[test]
    fn every_engine_names_an_agent_the_format_accepts() {
        for name in ["identical-span", "lexical"] {
            let e = engine(name).unwrap_or_else(|| panic!("{name} is an engine"));
            assert_eq!(
                e.agent().as_str(),
                format!("tool:smysl-same-as-{name}"),
                "§3.6 spells the agent this way"
            );
        }
    }
}
