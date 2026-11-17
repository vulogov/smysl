//! `smysl-tel` — smysl for telemetry study and research (RFC SMYSL-3).
//!
//! **TL-P2 step 1: reading an object snapshot exactly.** The canonical diff (§8) compares two
//! snapshots of one object leaf by leaf, and a change unit carries the leaf's value as text.
//! For two implementations to agree on a change's uid they must agree on every leaf's text, and
//! that starts lower than it looks:
//!
//! - [`json`] reads JSON and keeps every number **as its token**. A JSON number becomes a `dec`
//!   (§5.5), and a `dec` is read from the digits as written, never through binary64 (§5.3), so
//!   a parser that converts to `f64` on the way in has already lost the value.
//! - [`leaf`] is §5.5's canonical leaf value: a `vtype` and one text per value, so `41` and
//!   `41.0` in two snapshots are no change at all. An unkeyed array or opaque object is one
//!   leaf, written as RFC 8785 JCS with `dec` numbers ([`jcs`]).
//! - [`mod@pointer`] is RFC 6901, plus the one extension profiles use: a `*` segment that matches
//!   any one segment, with a literal `*` written `~2`.
//!
//! **Pure.** No clock, no file, no network: every function is a function of its arguments,
//! and the crate is on `xtask check-purity`'s list from its first commit.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod diff;
pub mod jcs;
pub mod json;
pub mod leaf;
pub mod pointer;
pub mod profile;
pub mod readers;
pub mod scrape;
pub mod snapshot;

pub use diff::{diff, leaves, part_text, Diff, Note, Snapshot};
pub use json::{Json, JsonError};
pub use leaf::{Leaf, VType};
pub use pointer::{Pattern, Pointer};
pub use profile::{Profile, ProfileError};
