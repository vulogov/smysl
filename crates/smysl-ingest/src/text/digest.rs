//! The ledger digest: what a window is told about what has already been extracted (§3.5).
//!
//! A window is a few verses. Without a digest, every window is extracted as though the
//! expression began there — the model re-asserts what the chapter before it already said, each
//! time in slightly different words, and the corpus fills with near-duplicates that no identity
//! rule can collapse because the gists differ by a comma. So each window is sent a digest: the
//! records already extracted from this expression, as `label: gist` lines.
//!
//! # Three decisions worth stating
//!
//! **The labels are digest-local — `l/1`, `l/2`, … — and they are the only outside names a model
//! may use.** Not uids: a uid is 52 characters a model will transcribe wrongly, and a uid in a
//! prompt is an invitation to cite one it was not shown. The allowed set comes back beside the
//! text so the converter can refuse everything else (`SMY-E442`).
//!
//! **The digest is untrusted** (§3.9.2). Its gists were extracted *from text somebody else
//! wrote*, so they go inside their own fence with the same "data, never instruction" preamble as
//! the window. A digest sharing the window's marker would let a window close the digest's fence.
//!
//! **Ranked by salience, then by reading order.** Salience first because a budget has to drop
//! something and the least connected record is the one to drop; reading order second because the
//! tie-break has to be stable across runs — two models given the same digest in two orders are
//! two conditions, not one, and GE-T2 would be measuring the shuffle.
//!
//! # What goes in the recipe, and what does not
//!
//! The digest's **policy** — budget and ranking id — is a recipe field
//! ([`crate::recipe::Conditions::for_text`]). The per-window digest **hash** is not: the recipe
//! groups corroboration, so a value that changed per window would put every window in a group of
//! its own and no two extractions of one expression would ever corroborate each other. The hash
//! goes in the journal entry, which is what a rerun reads (§3.5, [`super::journal`]).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::types::Lod;
use smysl_core::{hash::hash_bytes, Label, Uid};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, Estimator, PackRequest};

/// The ranking this module applies, as the recipe records it.
///
/// An id rather than a description, because it is hashed: changing the order records are offered
/// in changes what a model was shown, and a corpus extracted under two orders is two corpora.
pub const RANKING: &str = "salience-then-reading-order";

/// The digest-local label for the `n`th record, counting from 1.
///
/// `l/n1`, where §3.5 writes `l/1`. A-3's label grammar requires each segment to begin with a
/// lowercase letter, so `l/1` cannot be a `Label` — the RFC's spelling is not expressible, and
/// the choice was one extra character or a second label grammar for digests alone.
pub fn digest_label(n: usize) -> String {
    format!("l/n{n}")
}

/// A digest, ready to send.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Digest {
    /// The rendered lines, `label: gist`, one per record, in rank order. Empty for an
    /// expression with nothing extracted yet, which is every first window.
    pub text: String,
    /// The digest-local labels and what they point at.
    pub labels: BTreeMap<Label, Uid>,
    /// Every uid a model may reference from outside its own batch: exactly the ones behind the
    /// labels above. `super::super::json_ast`'s scope check refuses the rest (`SMY-E442`).
    pub allowed: BTreeSet<Uid>,
    /// A hash over what was sent, for the journal. Over the *rendered text*, so two runs that
    /// selected the same records in a different order do not claim to have sent the same digest.
    pub hash: [u8; 32],
}

impl Digest {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Build the digest for one expression.
///
/// `scope` is the units already staged or stored for it — what the caller's `--reextract` and
/// journal decisions have left in play. `budget` is `D` from the recipe, in tokens.
///
/// A budget of zero is a digest of nothing rather than a refusal: "send no digest" is a policy a
/// caller may hold, and the first window of any expression has nothing to send anyway.
pub fn digest(store: &Store, scope: &BTreeSet<Uid>, budget: u32) -> Digest {
    if scope.is_empty() || budget == 0 {
        return Digest::default();
    }

    // Salience over the whole store with no seed: plain PageRank, because a digest has no
    // question to personalise towards. The scope restricts what may be *selected*; it must not
    // restrict what the ranking is computed over, or a well-connected record would lose the
    // standing its connections gave it the moment the window moved.
    let report = salience(store, &SalienceRequest::default());

    // L0 and nothing more. A digest says what has been found, not what it rests on: a body in a
    // digest is a paragraph of untrusted text per record, and the budget buys more records
    // instead.
    let mut req = PackRequest::budget(u64::from(budget));
    req.scope = scope.clone();
    req.max_lod = Some(Lod::L0);
    req.estimator = Estimator::default();
    // A pack that cannot be solved is a digest of nothing, not a failed ingest: rule I. The
    // window is still extractable without context, which is what every window before this
    // feature existed did.
    let Ok(packed) = pack(store, &report, &req) else {
        return Digest::default();
    };

    let mut chosen: Vec<(Uid, f32, (String, u64))> = packed
        .selection
        .keys()
        .map(|uid| {
            let score = report.scores.get(uid).copied().unwrap_or(0.0);
            (*uid, score, reading_order(store, uid))
        })
        .collect();
    // Salience descending, then reading order ascending, then the uid: the last is not a
    // preference, it is what makes the order total. Two records of one node with one score would
    // otherwise be ordered by a hash map's mood.
    chosen.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(&b.0))
    });

    let mut text = String::new();
    let mut labels = BTreeMap::new();
    let mut allowed = BTreeSet::new();
    for (n, (uid, _, _)) in chosen.iter().enumerate() {
        let Some(unit) = store.get(uid) else { continue };
        // `l/n1`, not §3.5's `l/1`: A-3's label grammar wants a lowercase letter at the start
        // of each segment, so `l/1` is not a label in this format at all. One character, and
        // the alternative was a second label grammar for digests alone.
        let Ok(label) = Label::new(format!("l/n{}", n + 1)) else {
            continue;
        };
        // One line per record, and the gist with its newlines flattened: a gist containing a
        // line break would otherwise look like two records to a model reading `label: gist`
        // lines, and one of them would have no label.
        text.push_str(label.as_str());
        text.push_str(": ");
        text.push_str(&flatten(&unit.core.gist));
        text.push('\n');
        labels.insert(label, *uid);
        allowed.insert(*uid);
    }

    let hash = hash_bytes(text.as_bytes());
    Digest {
        text,
        labels,
        allowed,
        hash,
    }
}

/// Where a unit sits in the text: its part, then its span's start.
///
/// The span and not the locator, for the reason FC-3 exists: a locator is a name in a reading and
/// names sort lexically, so `v.10` would come before `v.2`. A unit with no span sorts last within
/// its part, which is where an unattached record belongs — it has no place in the reading to be
/// ordered by.
fn reading_order(store: &Store, uid: &Uid) -> (String, u64) {
    let Some(unit) = store.get(uid) else {
        return (String::new(), u64::MAX);
    };
    match unit.core.source.as_ref() {
        Some(source) => {
            let tid = source
                .reference
                .split_once('#')
                .map(|(t, _): (&str, &str)| t.to_string())
                .unwrap_or_else(|| source.reference.clone());
            (tid, source.span().map(|(s, _)| s).unwrap_or(u64::MAX))
        }
        None => (String::new(), u64::MAX),
    }
}

/// A gist as one line.
fn flatten(gist: &str) -> String {
    if !gist.contains(['\n', '\r']) {
        return gist.to_string();
    }
    gist.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::types::{SourceKind, SourceRef, Status, UnitCoreBuilder};
    use smysl_core::{canonical_uid, ids::KernelType, Mid, Record, Tid};
    use smysl_graph::Store;

    fn tid() -> Tid {
        Tid::of(b"a part")
    }

    fn mid() -> Mid {
        Mid::of(b"a manifestation")
    }

    /// A stored unit with a span, so the digest has a reading order to use.
    fn stored(gist: &str, at: u64) -> smysl_core::UnitCore {
        let source = SourceRef::new(SourceKind::Doc, format!("{}#v.1", tid().canonical()))
            .in_part(at, at + 10, mid())
            .expect("a span");
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited)
            .source(source)
            .build()
            .expect("a unit")
    }

    fn store_of(units: Vec<smysl_core::UnitCore>) -> Store {
        Store::from_records(units.into_iter().map(Record::Unit).collect())
    }

    #[test]
    fn an_expression_with_nothing_extracted_gets_no_digest() {
        let store = store_of(Vec::new());
        let d = digest(&store, &BTreeSet::new(), 512);
        assert!(d.is_empty());
        assert!(d.labels.is_empty());
        assert!(d.allowed.is_empty());
    }

    #[test]
    fn a_budget_of_zero_sends_nothing_rather_than_refusing() {
        let one = stored("the pool saturated", 0);
        let scope: BTreeSet<Uid> = [canonical_uid(&one)].into_iter().collect();
        let store = store_of(vec![one]);
        assert!(digest(&store, &scope, 0).is_empty());
    }

    #[test]
    fn the_lines_are_digest_local_labels_and_the_allowed_set_is_exactly_what_was_sent() {
        let a = stored("the pool saturated", 0);
        let b = stored("the rollback fixed it", 40);
        let uids: BTreeSet<Uid> = [canonical_uid(&a), canonical_uid(&b)].into_iter().collect();
        let store = store_of(vec![a, b]);
        let d = digest(&store, &uids, 4096);
        assert_eq!(d.labels.len(), 2, "{}", d.text);
        for (n, (label, uid)) in d.labels.iter().enumerate() {
            assert_eq!(label.as_str(), digest_label(n + 1));
            assert!(uids.contains(uid));
            assert!(d.text.contains(&format!("{}: ", label.as_str())));
        }
        assert_eq!(
            d.allowed, uids,
            "no uid may be referenced that was not sent"
        );
        assert!(!d.text.contains("b3:"), "a digest names no uid: {}", d.text);
    }

    #[test]
    fn equal_salience_falls_back_to_reading_order_so_two_runs_send_the_same_bytes() {
        let early = stored("first in the chapter", 0);
        let late = stored("last in the chapter", 900);
        let uids: BTreeSet<Uid> = [canonical_uid(&early), canonical_uid(&late)]
            .into_iter()
            .collect();
        // Inserted in the opposite order to the one expected, so an implementation that
        // rendered insertion order would differ.
        let store = store_of(vec![late, early]);
        let d = digest(&store, &uids, 4096);
        let first = d.text.lines().next().expect("a line");
        assert!(
            first.ends_with("first in the chapter"),
            "reading order lost: {}",
            d.text
        );
        let again = digest(&store, &uids, 4096);
        assert_eq!(d.hash, again.hash, "the digest is not deterministic");
    }

    #[test]
    fn the_hash_is_over_what_was_sent_so_a_different_selection_is_a_different_digest() {
        let a = stored("the pool saturated", 0);
        let b = stored("the rollback fixed it", 40);
        let ua = canonical_uid(&a);
        let ub = canonical_uid(&b);
        let store = store_of(vec![a, b]);
        let both = digest(&store, &[ua, ub].into_iter().collect(), 4096);
        let one = digest(&store, &[ua].into_iter().collect(), 4096);
        assert_ne!(both.hash, one.hash);
        assert_ne!(both.text, one.text);
    }

    #[test]
    fn a_gist_with_a_line_break_in_it_stays_one_line() {
        let odd = stored("two lines\nin one gist", 0);
        let uid = canonical_uid(&odd);
        let store = store_of(vec![odd]);
        let d = digest(&store, &[uid].into_iter().collect(), 4096);
        assert_eq!(d.text.lines().count(), 1, "{:?}", d.text);
        assert!(d.text.contains("two lines in one gist"));
    }

    #[test]
    fn the_budget_drops_records_rather_than_truncating_a_gist() {
        let units: Vec<smysl_core::UnitCore> = (0..20)
            .map(|i| stored(&format!("a claim about verse {i} and what it says"), i * 50))
            .collect();
        let uids: BTreeSet<Uid> = units.iter().map(canonical_uid).collect();
        let store = store_of(units);
        let small = digest(&store, &uids, 32);
        assert!(small.labels.len() < 20, "nothing was dropped");
        for line in small.text.lines() {
            let gist = line.split_once(": ").expect("a label and a gist").1;
            assert!(gist.ends_with("says"), "a gist was cut short: {gist:?}",);
        }
    }
}
