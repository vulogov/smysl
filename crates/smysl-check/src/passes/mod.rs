//! The check passes of §17, one module each.
//!
//! Every pass takes the store and a `Report` and appends to it. None of them return a
//! result, because none of them may stop the pipeline.

pub mod closure;
pub mod commitment;
pub mod epistemics;
pub mod extension;
pub mod granularity;
pub mod integrity;
pub mod library;
pub mod shape;
/// Pass 13, behind the `text` feature: rule E's engine lives in `smysl-text`.
#[cfg(feature = "text")]
pub mod time;
pub mod trust;

use std::borrow::Cow;
use std::collections::BTreeSet;

use smysl_core::{Record, Uid};
use smysl_graph::StoreRead;

/// What a pass looks at (AC-8, RFC SMYSL-3 §17.1).
///
/// `All` is the whole store, as every pass always looked. `Batch` is a kept check's recomputation:
/// the per-unit rules over the units a batch can change, and the per-record rules over the
/// records it adds. The store is still the whole store, so every lookup a rule makes (a ground,
/// a prefix, a cited payload) is answered as before; only what the pass iterates is narrowed.
/// Rules over the whole store (the aggregates, integrity's graph, rule E) do not run in a
/// `Batch`: `crate::incremental` keeps them.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Scope<'a> {
    All,
    Batch {
        units: &'a BTreeSet<Uid>,
        records: &'a [Record],
    },
}

impl<'a> Scope<'a> {
    pub(crate) fn is_all(self) -> bool {
        matches!(self, Scope::All)
    }

    /// The units the per-unit rules judge: every unit, or the batch's dirty units the store
    /// holds, in uid order.
    pub(crate) fn units<'s, S: StoreRead + ?Sized>(
        self,
        store: &'s S,
    ) -> Box<dyn Iterator<Item = (&'s Uid, S::UnitRef<'s>)> + 's>
    where
        'a: 's,
    {
        match self {
            Scope::All => Box::new(store.units()),
            Scope::Batch { units, .. } => {
                Box::new(units.iter().filter_map(|u| store.get(u).map(|x| (u, x))))
            }
        }
    }

    /// The records the per-record rules judge: every record, or the batch's new ones.
    pub(crate) fn records<'s, S: StoreRead + ?Sized>(
        self,
        store: &'s S,
    ) -> Box<dyn Iterator<Item = Cow<'s, Record>> + 's>
    where
        'a: 's,
    {
        match self {
            Scope::All => Box::new(store.records()),
            Scope::Batch { records, .. } => Box::new(records.iter().map(Cow::Borrowed)),
        }
    }
}
