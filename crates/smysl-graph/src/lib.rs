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
pub mod lineage;
pub mod merge;
pub mod relink;
pub mod salience;
pub mod store;
pub mod traverse;

pub use adjacency::{Adjacency, Edge, EdgeKind, EdgeSet, NodeId};
pub use lineage::{
    dependents, diff, hop_diff, membership, trace, AgentActivity, HopDiff, Lineage, LineageNode,
    RecipeChange, RecipeChangeKind, StoreDiff, TraceKind, Via,
};
pub use merge::{
    effective_status, merge, plan_retraction, DetectionContext, EffectiveStatus, MergeOptions,
    MergeReport, RetractionAuthority, RetractionPlan, RetractionPolicy, SupersessionPolicy,
};
pub use salience::{
    salience, view_roots, SalienceReport, SalienceRequest, SalienceTerms, SalienceWeights,
};
pub use smysl_core::error::MergeError;
pub use store::index::{Cached, Entry, Index, IndexError};
pub use store::{AppendReport, OpenReport, Store, StoreOptions};
pub use traverse::{closure, cycles, rebuttals_of, reverse_closure, topo, Scratch, TopoOrder};
