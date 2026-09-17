//! Which unit a label names in a store.
//!
//! A store holds labels as `LabelBinding` records, and nothing stops a merged store binding one
//! label to several units: three extraction runs of one commit, each naming its decision
//! `d/g90ec2f7-1`, bind that label three times. `merge` reports it as a `label-collision`
//! contention. What it must not do is resolve: every caller used to build its label map by
//! inserting bindings into a `BTreeMap`, so the last binding in record order won, and
//! `smysl retract` by that label retracted a unit the caller never chose.
//!
//! Within a single surface document ownership is already decided — the last declaration wins,
//! with `SMY-W054` — so a store built from one parse never binds a label twice. This is for
//! stores that were merged.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use smysl_core::{Label, Record, Uid};

use crate::Store;

/// Why a label does not name exactly one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LabelError {
    /// Nothing in the store is bound to it.
    Unbound,
    /// It is bound to several units, sorted by uid. The store will not choose between them.
    Ambiguous(Vec<Uid>),
}

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LabelError::Unbound => write!(f, "nothing in this store is bound to it"),
            LabelError::Ambiguous(uids) => {
                write!(f, "it is bound to {} different units", uids.len())
            }
        }
    }
}

impl std::error::Error for LabelError {}

/// Every uid this store binds the label to, sorted and without repeats.
///
/// Repeats are collapsed because a store merged from two copies of one document carries the
/// same binding twice, and one name for one unit is not an ambiguity.
pub fn label_bindings(store: &Store, label: &Label) -> Vec<Uid> {
    store
        .iter()
        .filter_map(|r| match r {
            Record::LabelBinding(b) if &b.label == label => Some(b.uid),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every label this store binds to a uid, sorted and without repeats (1.5).
///
/// The other direction, and the one anything that *prints* a store needs: a report names
/// `p/g90ec2f781421-4-1`, not `b3:xkcd…`. Until now a consumer kept its own map, filled as it
/// staged, and a store read back from disk had none without walking every record — which is what
/// this does once, rather than each caller writing it again.
///
/// A unit may carry more than one label: identity is content, so two declarations with the same
/// gist, status and grounds are one unit under two names.
pub fn labels_of(store: &Store, uid: &Uid) -> Vec<Label> {
    store
        .iter()
        .filter_map(|r| match r {
            Record::LabelBinding(b) if &b.uid == uid => Some(b.label.clone()),
            _ => None,
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every binding in the store, uid to labels, in one pass (1.5).
///
/// For a printer that needs them all: one walk of the log rather than one per unit.
pub fn label_index(store: &Store) -> BTreeMap<Uid, Vec<Label>> {
    let mut out: BTreeMap<Uid, BTreeSet<Label>> = BTreeMap::new();
    for r in store.iter() {
        if let Record::LabelBinding(b) = r {
            out.entry(b.uid).or_default().insert(b.label.clone());
        }
    }
    out.into_iter()
        .map(|(u, labels)| (u, labels.into_iter().collect()))
        .collect()
}

/// The one uid this label names, or why there is not exactly one.
pub fn resolve_label(store: &Store, label: &Label) -> Result<Uid, LabelError> {
    match label_bindings(store, label).as_slice() {
        [] => Err(LabelError::Unbound),
        [only] => Ok(*only),
        many => Err(LabelError::Ambiguous(many.to_vec())),
    }
}
