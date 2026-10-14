//! `identical-span`: the same bytes of the same part, read under two manifests (§3.6).
//!
//! # The one engine that is not a guess
//!
//! A tid is BLAKE3 over a part's normalised bytes, so two manifests carrying the same tid
//! carry the *same text*, not similar text. If two units then name the same byte range of it
//! and say the same thing, they are not two propositions — they are one proposition extracted
//! twice, which is what happens the moment a passage is reprinted. An anthology that includes
//! a story, and the collection that story first appeared in, are two expressions over one
//! part; so are two editions of a Bible that share a preface, and so is the same corpus
//! ingested twice under two aliases.
//!
//! That is why S0 auto-accepted this engine's proposals (its C0) at rung `computed` while
//! sending every other engine's to a judge. There is no threshold here and nothing to tune.
//!
//! # Under *different* manifests, and why that is a restriction rather than an oversight
//!
//! Two units with the same tid, span and gist under the **same** manifest are the same
//! extraction run twice over one place — a duplicate, which `merge` is for and which a same-as
//! edge would dress up as a finding. §3.6 says "under different manifests" and the
//! implementation holds to it: a store that ingested one alias twice gets no edges from this
//! engine, and its duplicate units are visible as what they are.
//!
//! # What is compared, and what deliberately is not
//!
//! The key is the **tid, the span and the gist**. Not the locator: an anthology may number the
//! paragraph differently from the collection, and refusing the pair for that would make the
//! engine fail in exactly the case it exists for. The locators go in the proposal's `why`
//! where they differ, because a reader deciding whether to accept wants to see that the same
//! bytes were called two names.
//!
//! Not the language either, and the reason is the same tid: the same bytes are in one
//! language. A unit *about* those bytes may have a gist written in another (`SMY-W436`), and
//! two units whose gists are byte-identical are written in the same language whatever the
//! manifest says.
//!
//! Not `status`, `deps` or `payload`. Two units can agree on what a passage says and disagree
//! about how sure they are; the same-as claim is about the proposition, and the rest is what
//! the units carry about it.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{Mid, Uid};
use smysl_graph::Store;

use super::{candidates, place, Engine, Proposal};

/// The `identical-span` engine. Pure, and needs nothing but the store.
#[derive(Debug, Clone, Copy, Default)]
pub struct IdenticalSpan;

/// What makes two extractions the same extraction: one part, one range, one sentence.
type Key = (String, u64, u64, String);

impl Engine for IdenticalSpan {
    fn id(&self) -> &'static str {
        "identical-span"
    }

    fn propose(&self, store: &Store, scope: Option<&BTreeSet<Uid>>) -> Vec<Proposal> {
        // Grouped rather than compared pairwise: the key is exact, so a `BTreeMap` finds every
        // group in one pass instead of asking `n²` questions whose answer is almost always no.
        let mut groups: BTreeMap<Key, Vec<(Uid, Mid, Option<String>)>> = BTreeMap::new();
        for (uid, unit) in candidates(store, scope) {
            let Some(at) = place(unit) else {
                continue;
            };
            // No span, no claim: this engine's whole argument is "these bytes", and a unit
            // that cannot say which bytes it read is outside it. `anchored` is the engine for
            // a unit located by name. A unit with a span but no manifest cannot happen through
            // `provenance::stamp` — `in_part` sets the pair — but a decoded store is not
            // required to have come from this build.
            let (Some((start, end)), Some(mid)) = (at.span, at.mid) else {
                continue;
            };
            groups
                .entry((at.tid, start, end, unit.core.gist.clone()))
                .or_default()
                .push((uid, mid, at.locator));
        }

        let mut out: BTreeSet<Proposal> = BTreeSet::new();
        for ((tid, start, end, _), members) in &groups {
            for (i, (ua, ma, la)) in members.iter().enumerate() {
                for (ub, mb, lb) in &members[i + 1..] {
                    if ma == mb {
                        // Same manifest: a duplicate extraction, not two expressions.
                        continue;
                    }
                    let places = match (la.as_deref(), lb.as_deref()) {
                        (Some(x), Some(y)) if x != y => format!(", called {x} and {y}"),
                        _ => String::new(),
                    };
                    let why = format!(
                        "the same bytes {start}..{end} of {tid} under {} and {}{places}",
                        ma.short(),
                        mb.short()
                    );
                    if let Some(p) = Proposal::new(*ua, *ub, why) {
                        out.insert(p);
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}
