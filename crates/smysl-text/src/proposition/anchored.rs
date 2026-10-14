//! `anchored`: two places a versification alignment says are one place (§3.6).
//!
//! # The cross-lingual engine, and the reason it is not a similarity
//!
//! Genesis 3:5 in the King James and Бытие 3:5 in the Synodal are the same verse. Nothing
//! about the two strings says so — they share no terms, and an embedding that bridges them
//! does it at a markedly lower score than it bridges two English sentences (SMYSL-2.2 §3.7,
//! C2). The alignment says so exactly, and it says so from the library alone: two manifests'
//! `versification` declarations, the installed maps, and the locator tables
//! ([`crate::index::align`]).
//!
//! So §3.6's ruling is that `anchored` alone is used for the Bibles, and that it is also the
//! *right* engine there rather than a stopgap, "since a versification alignment is exact where
//! a cosine is not". Cross-lingual classes wait for TX-P10's S1 embeddings; cross-lingual
//! *anchored* classes do not wait for anything.
//!
//! # A candidate generator, and it says so
//!
//! This engine proposes every pair of units at aligned places. Over five Bibles that is a
//! great many proposals, and almost all of them are right about the *place* and unexamined
//! about the *proposition*: two units at Genesis 3:5 may be extracting different claims from
//! one verse. That is what S0's C3 was — a candidate source feeding a judge — and it is why
//! nothing here accepts its own output. `attested:2` (§3.6's default for corpus measures) is
//! the shape of the answer: `anchored` saying "same verse" and another engine saying "same
//! sentence" is two agents, and one of them alone is a proposal.
//!
//! # Both directions, equal or adjacent
//!
//! The table records both directions ([`crate::index::align`]), and the engine asks in the
//! direction it holds. A pair is proposed when the alignment maps one locator to the other, or
//! to its neighbour — the same C3 rule S0 measured ("the same or adjacent verse"), which
//! absorbs the one-verse disagreements two versifications have at the seams of a psalm.
//!
//! # Within a part, the span is not consulted
//!
//! Two units at the same aligned verse are proposed whatever bytes they name. A verse is the
//! unit of alignment; a span inside it is how much of the verse an extractor read, and two
//! extractors reading different halves of one verse are still talking about that verse.
//! [`super::IdenticalSpan`] is the engine that cares about bytes, and it cares because it has
//! the same bytes to care about.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{Mid, Uid};
use smysl_graph::Store;

use crate::index::align;

use super::{adjacent_locators, candidates, place, Engine, Proposal};

/// The `anchored` engine over a built alignment table.
///
/// **Not §4.2's `Anchored<'l> { lib: &'l Library, scheme: String }`**, and the departure is the
/// point of the table. A `Library` plus a scheme name means rebuilding the alignment inside
/// `propose`, which costs every locator table in the corpus and makes the engine's answer
/// depend on a map installed since the index was built. [`crate::index::Index`] already holds
/// the table, §3.8's gate is that the table is reproducible from the library alone, and an
/// engine handed the table is pure in the arithmetic sense: the same table and the same store
/// give the same proposals. The scheme is per manifest pair and is in the table's own rows, so
/// naming one here would have let a caller ask for an alignment that was not the one computed.
#[derive(Debug, Clone, Copy)]
pub struct Anchored<'t> {
    /// The alignment, as [`crate::index::align::rebuild`] produced it.
    pub table: &'t align::Table,
}

impl<'t> Anchored<'t> {
    pub fn new(table: &'t align::Table) -> Anchored<'t> {
        Anchored { table }
    }
}

impl Engine for Anchored<'_> {
    fn id(&self) -> &'static str {
        "anchored"
    }

    fn propose(&self, store: &Store, scope: Option<&BTreeSet<Uid>>) -> Vec<Proposal> {
        // Units by manifest, each with the locator it names. A unit with no locator cannot be
        // anchored: this engine's evidence is a named place, and `SMY-W405` exists because a
        // span and a locator can disagree — the span wins there, and here there is nothing to
        // win against.
        let mut by_mid: BTreeMap<Mid, Vec<(Uid, String)>> = BTreeMap::new();
        for (uid, unit) in candidates(store, scope) {
            let Some(at) = place(unit) else {
                continue;
            };
            if let (Some(mid), Some(locator)) = (at.mid, at.locator) {
                by_mid.entry(mid).or_default().push((uid, locator));
            }
        }

        let mut out: BTreeSet<Proposal> = BTreeSet::new();
        let mids: Vec<&Mid> = by_mid.keys().collect();
        for (i, ma) in mids.iter().enumerate() {
            for mb in &mids[i + 1..] {
                let Some(rows) = self.table.get(&(**ma, **mb)) else {
                    continue;
                };
                // The other manifest's units indexed by locator, so each of this manifest's
                // locators is one lookup rather than a scan.
                let mut there: BTreeMap<&str, Vec<Uid>> = BTreeMap::new();
                for (uid, locator) in &by_mid[*mb] {
                    there.entry(locator.as_str()).or_default().push(*uid);
                }
                for (uid, locator) in &by_mid[*ma] {
                    let Some(target) = rows.rows.get(locator) else {
                        continue;
                    };
                    for (other, uids) in &there {
                        if !adjacent_locators(target, other) {
                            continue;
                        }
                        let same = target == other;
                        for ub in uids {
                            let why = if same {
                                format!(
                                    "{locator} and {other} are one place under {} ({} ↔ {})",
                                    rows.scheme,
                                    ma.short(),
                                    mb.short()
                                )
                            } else {
                                format!(
                                    "{locator} aligns to {target}, adjacent to {other}, under {} ({} ↔ {})",
                                    rows.scheme,
                                    ma.short(),
                                    mb.short()
                                )
                            };
                            if let Some(p) = Proposal::new(*uid, *ub, why) {
                                out.insert(p);
                            }
                        }
                    }
                }
            }
        }
        // One proposal per pair, for the reason `lexical` deduplicates: a pair reachable both
        // exactly and through a neighbour is one claim.
        let mut once: BTreeMap<(Uid, Uid), Proposal> = BTreeMap::new();
        for p in out {
            once.entry((p.a, p.b)).or_insert(p);
        }
        once.into_values().collect()
    }
}
