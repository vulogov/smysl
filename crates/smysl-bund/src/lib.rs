//! smysl's Bund2 bridge: RFC SMYSL-4 (`Documentation/SMYSL-4_RFC/SMYSL-4_BUND2_BRIDGE.md`).
//!
//! A Bund2 script runs over a disposable copy of a library, in a separate `bund2` process, and
//! its answer comes back as a [`QueryValue`]. Bund2 is never linked (D-B1): it pins every smysl
//! crate at `=1.15.0`, so the two meet at `smysl/1.0` and a process boundary.
//!
//! - [`policy`]: [`QueryPolicy`], [`WordPolicy`] and [`Trust`], with the RFC's defaults.
//! - [`words`]: Bund2 0.56.0's words, which of them a script may name, and why the rest may not.
//! - [`gate`]: the word gate (§6). A first check that names a mistake early. **It is not the
//!   boundary** (D-B2): Bund2 can build a word's name at run time and call it.
//! - [`decode`]: the answer. Bund2 cannot write a string as JSON, and a script cannot walk a
//!   map's keys, so the bridge reads the value as `debug.display_stack` renders it: one line per
//!   stack value, Rust-escaped strings, floats that round-trip, a type on every node (§7).
//! - [`error`]: [`BundError`], and a refusal's class read from its text. Bund2 prints a
//!   refusal's text and not its `SMnn` code (§8).
//! - [`probe`]: what `bund2 --version` says about a binary.
//! - [`snapshot`]: the library the process sees, written fresh in a private directory (§4).
//! - [`run`]: the wrapper, the process, the wall clock; [`eval_store`], [`eval_library`] and
//!   [`eval_over_sq`] (§5, §7). BQ-P2 runs `Trust::Trusted` only; the sandbox is BQ-P3.

#![forbid(unsafe_code)]

pub mod decode;
pub mod error;
pub mod gate;
pub mod policy;
pub mod probe;
pub mod run;
pub mod snapshot;
pub mod value;
pub mod words;

pub use error::BundError;
pub use policy::{QueryPolicy, Trust, WordPolicy};
pub use probe::{probe_bund_binary, BundBinaryInfo};
pub use run::{eval_library, eval_over_sq, eval_store, BundQueryResult};
pub use value::QueryValue;

/// The oldest `bund2` this bridge runs: the release its word lists and its decoder were read
/// against.
pub const MIN_BUND2: (u64, u64, u64) = (0, 56, 0);
