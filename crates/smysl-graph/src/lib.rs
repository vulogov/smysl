//! `smysl-graph` - the append-only store, its derived index, adjacency, merge, lineage,
//! and salience (§16).
//!
//! Pure and synchronous. Every operation here is a bit-reproducible function of its
//! inputs (rule D); the purpose-built adjacency store (D-1) exists precisely so that
//! traversal order is structural rather than defensive.
//!
//! Filled by SM-P3 (store, index, adjacency), SM-P6 (merge), SM-P7 (lineage),
//! SM-P8 (salience).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod adjacency;
pub mod store;
pub mod traverse;

pub use adjacency::{Adjacency, Edge, EdgeKind, EdgeSet, NodeId};
pub use smysl_core::error::MergeError;
pub use store::index::{Cached, Entry, Index, IndexError};
pub use store::{AppendReport, OpenReport, Store, StoreOptions};
pub use traverse::{closure, cycles, rebuttals_of, reverse_closure, topo, Scratch, TopoOrder};
