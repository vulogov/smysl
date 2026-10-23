//! The interpretive layer (SMYSL-2.8 §3.1): what a store holds *about* its units rather than
//! the units — relations, withdrawals, resolutions, commitments, recorded contentions, threads,
//! views.
//!
//! One crate-private struct and one fold, so that every implementation of
//! [`crate::store::StoreRead`] — the in-memory store today, a paged one and a union of shards
//! later — derives this layer the same way. It is proportional to judgements, not to text, which
//! is why a paged store can keep it hot.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{
    to_cbor, AgentId, Commit, Contention, Record, Relation, Resolution, Thread, ThreadId, Uid,
    View, ViewId, Withdrawal,
};

/// A relation's key: `(kind, from, to)`, the canonical relation order.
pub(crate) type RelKey = (String, Uid, Uid);

#[derive(Debug, Clone, Default)]
pub(crate) struct Interpretive {
    pub(crate) relations: BTreeMap<RelKey, Relation>,
    /// Each relation's rid, to its key in `relations`.
    pub(crate) rids: BTreeMap<Uid, RelKey>,
    pub(crate) threads: BTreeMap<(ThreadId, AgentId), Thread>,
    pub(crate) views: BTreeMap<ViewId, View>,
    pub(crate) contentions: Vec<Contention>,
    /// Withdrawals by the rid they name, whether or not that relation has arrived (1.4).
    pub(crate) withdrawals: BTreeMap<Uid, BTreeSet<Withdrawal>>,
    pub(crate) resolutions: BTreeSet<Resolution>,
    /// Commitments by the unit they name, whether or not that unit has arrived (1.7).
    pub(crate) commits: BTreeMap<Uid, BTreeSet<Commit>>,
}

pub(crate) fn rel_key(r: &Relation) -> RelKey {
    (r.kind.as_str().to_string(), r.from, r.to)
}

impl Interpretive {
    /// Fold one record into the layer, and say whether it belonged here. Order-independent by
    /// construction, as `Store::absorb` is (rule U).
    pub(crate) fn absorb_hot(&mut self, r: &Record) -> bool {
        match r {
            Record::Relation(rel) => {
                let key = rel_key(rel);
                self.rids.insert(rel.uid(), key.clone());
                match self.relations.get_mut(&key) {
                    Some(existing) => {
                        existing
                            .attestations
                            .extend(rel.attestations.iter().cloned());
                    }
                    None => {
                        self.relations.insert(key, rel.clone());
                    }
                }
            }
            Record::Thread(t) => {
                let key = (t.id.clone(), t.owner.clone());
                // Last writer wins *within* the key; across owners there is no
                // conflict to resolve (§5.2).
                //
                // A tie on the HLC is broken by encoded bytes, which makes the
                // register a maximum over a *total* order. Without that, two peers
                // merging the same pair of simultaneous writes in opposite orders
                // would keep different threads, and merge would not be commutative.
                let replace = match self.threads.get(&key) {
                    Some(existing) => match existing.ts.cmp(&t.ts) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Greater => false,
                        std::cmp::Ordering::Equal => {
                            to_cbor(&Record::Thread(t.clone()))
                                > to_cbor(&Record::Thread(existing.clone()))
                        }
                    },
                    None => true,
                };
                if replace {
                    self.threads.insert(key, t.clone());
                }
            }
            Record::View(v) => {
                self.views.insert(v.id.clone(), v.clone());
            }
            // Idempotent by id: a contention already recorded is not recorded twice,
            // which is what makes replaying a log a no-op rather than a duplication.
            Record::Contention(c) => {
                if !self.contentions.iter().any(|e| e.id == c.id) {
                    self.contentions.push(c.clone());
                }
            }
            Record::Withdrawal(w) => {
                self.withdrawals
                    .entry(w.relation)
                    .or_default()
                    .insert(w.clone());
            }
            Record::Resolution(r) => {
                self.resolutions.insert(r.clone());
            }
            Record::Commit(c) => {
                self.commits.entry(c.unit).or_default().insert(c.clone());
            }
            _ => return false,
        }
        true
    }

    /// The relation whose rid this is.
    pub(crate) fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        self.rids.get(rid).and_then(|k| self.relations.get(k))
    }
}
