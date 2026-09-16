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

use std::collections::BTreeSet;
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

/// The one uid this label names, or why there is not exactly one.
pub fn resolve_label(store: &Store, label: &Label) -> Result<Uid, LabelError> {
    match label_bindings(store, label).as_slice() {
        [] => Err(LabelError::Unbound),
        [only] => Ok(*only),
        many => Err(LabelError::Ambiguous(many.to_vec())),
    }
}
