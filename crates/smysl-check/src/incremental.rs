//! A store's check, kept as derived state (AC-8, RFC SMYSL-3 §17.1).
//!
//! OKO appends one ledger operation at a time and stages each against the whole store, so
//! [`check`] ran over the store and the batch on every commit: O(store) per append, 30 ms at
//! 6 × 10³ units. [`Checked`] holds the report of the store as it stands, so a batch can be
//! checked against it by recomputing only what the batch can change.
//!
//! **The contract is exact.** For a store `S` that `checked` describes and any batch `B`,
//! `checked.with_batch(S, B, opts)` equals `check(&Overlay::new(S, B), opts)`, byte for byte
//! once sorted. `tests/incremental.rs` holds it over the fixture stores dealt into batches,
//! over generated stores, and `examples/oko_ledger --verify` holds it after every operation.
//! Anything the recomputation cannot bound falls back to the full check, so a fallback costs
//! time and never correctness.
//!
//! **What it is valid for.** One store and one set of options. [`Checked::with_batch`] refuses
//! (falls back) when the options are not the ones it was built with, and [`Checked::advance`]
//! is the caller's statement that the batch was appended and nothing else was: a store that
//! grows by any other path needs a new [`Checked`].

use smysl_core::diag::Report;
use smysl_core::Record;
use smysl_graph::{Overlay, StoreRead};

use crate::{check, CheckOptions};

/// The options a report depends on, comparable. `parts` is an external resolver, so it is
/// compared by identity: the same resolver, not an equal one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    options: String,
    parts: Option<usize>,
}

impl Key {
    fn of(opts: &CheckOptions) -> Key {
        Key {
            options: format!(
                "{:?}|{:?}|{:?}|{:?}|{:?}",
                opts.granularity, opts.labels, opts.only, opts.consumer, opts.estimator
            ),
            parts: opts
                .parts
                .as_ref()
                .map(|p| std::sync::Arc::as_ptr(p) as *const () as usize),
        }
    }
}

/// How a batch's report was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Path {
    /// Recomputed where the batch can reach.
    Incremental,
    /// The full check, and why.
    Full(Fallback),
}

/// Why a batch was checked in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fallback {
    /// The options are not the ones the cache was built with.
    Options,
    /// No incremental path yet (AC-8 step 1): every batch.
    Unimplemented,
}

/// A batch checked against a store: the report of the store with the batch, and how it was
/// reached.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BatchReport {
    pub report: Report,
    pub path: Path,
    /// The options it was reached under, which the cache takes on when it advances.
    key: Key,
}

/// A store's check report, kept so a batch can be checked against it.
#[derive(Debug, Clone)]
pub struct Checked {
    key: Key,
    report: Report,
}

impl Checked {
    /// The full check of `store`.
    pub fn new(store: &(impl StoreRead + ?Sized), opts: &CheckOptions) -> Checked {
        Checked {
            key: Key::of(opts),
            report: check(store, opts.clone()),
        }
    }

    /// The store's report.
    pub fn report(&self) -> &Report {
        &self.report
    }

    /// The report of `store` with `batch` added, which [`check`] over an [`Overlay`] of the two
    /// would give.
    pub fn with_batch(
        &self,
        store: &(impl StoreRead + ?Sized),
        batch: Vec<Record>,
        opts: &CheckOptions,
    ) -> BatchReport {
        let key = Key::of(opts);
        let why = if key != self.key {
            Fallback::Options
        } else {
            Fallback::Unimplemented
        };
        BatchReport {
            report: check(&Overlay::new(store, batch), opts.clone()),
            path: Path::Full(why),
            key,
        }
    }

    /// The batch was appended: its report is the store's now.
    pub fn advance(&mut self, batch: BatchReport) {
        self.key = batch.key;
        self.report = batch.report;
    }
}
