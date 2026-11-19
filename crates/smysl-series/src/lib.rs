//! `smysl-series` — the series substrate (RFC SMYSL-3 §19, TL-P3).
//!
//! **What it is.** A set of samples per (series, instrument) stream, stored well enough to
//! materialise window evidence, answer "the value in force at t", and re-derive detectors. It is
//! not a time-series database and does not serve dashboards (§19.1, §27).
//!
//! **What is compared, and what is not.**
//! - A [`Sample`] is (`ts_ms`, `ns`, value, flags), and the substrate is a **set** of them: two
//!   equal samples are one, and adding a sample twice changes nothing (§9.1).
//! - The [`digest`] of a window is a function of its samples alone. It is the only thing two
//!   peers compare.
//! - [`chunk`]s are a local layout: their bytes depend on the span chosen and on the encoder,
//!   so they enter no identity (§19.2).
//!
//! **Where it lives.** [`Substrate`] over a [`Backend`]: [`MemBackend`] in memory, or, behind
//! feature `store-redb`, [`redb::RedbBackend`] in `<library>/index/series.redb` (§19.3).
//!
//! **Pure.** No clock, no network, no generator: every read is a function of the samples
//! added. On `xtask check-purity`'s list, with `smysl-tel`'s PRNG ban.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

use core::fmt;

pub mod chunk;
#[cfg(feature = "store-redb")]
pub mod redb;
pub mod sample;
pub mod store;

pub use chunk::Span;
pub use sample::{conflicts, digest, flags, Sample, SampleError};
pub use store::{
    AddReport, Backend, Coverage, EvictReport, MemBackend, SeriesInfo, SeriesRow, Substrate,
    WindowRead,
};

/// Why a substrate operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// The backend could not read or write.
    Backend(String),
    /// Stored bytes are not what this crate writes.
    Corrupt(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Backend(m) => write!(f, "series store: {m}"),
            Error::Corrupt(m) => write!(f, "series store: corrupt: {m}"),
        }
    }
}

impl std::error::Error for Error {}
