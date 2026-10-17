//! `sq/1`: the smysl query language — lexer, parser, syntax tree, canonical printer, and the
//! first four phases of the planner.
//!
//! The grammar is RFC SMYSL-2.9 and what each stage *means* is SMYSL-2.5 §3.3. TX-P6 step 1
//! built everything up to the tree; step 2 added the [`ports::Scope`] port, a planner over
//! SMYSL-2.5 §3.2's phases 1 to 4 — header, resolution, scope, partitions — and [`explain()`].
//! There is still no executor. A stage the remaining five phases own is **refused** with
//! `SMY-E461` and the phase that will lower it named, never ignored: a plan that quietly
//! dropped a stage would answer a different question from the one asked, and SMYSL-2.5 §3.6
//! only lets a query be saved after it has planned and run.
//!
//! The crate stays pure. `smysl-core` gives spans and diagnostics, `smysl-graph` gives the
//! store phase 3 resolves against, a normalisation quick-check serves `SMY-W510`, and that is
//! the whole dependency list.
//!
//! ```
//! let parsed = smysl_query::parse("from library | find \"faith\" | limit 10").unwrap();
//! assert_eq!(
//!     smysl_query::print(&parsed.query),
//!     "from library\nfind \"faith\"\nlimit 10"
//! );
//! ```
//!
//! Two properties hold over every accepted query, and the grammar fixtures assert both
//! (SMYSL-2.9 QG-7):
//!
//! - `parse(print(parse(q)))` equals `parse(q)` — printing loses nothing the tree holds;
//! - `print(parse(print(parse(q))))` equals `print(parse(q))` — printing is idempotent, so a
//!   query has exactly one canonical text and a saved query has one identity.

#![forbid(unsafe_code)]

pub mod ast;
pub mod exec;
pub mod explain;
pub mod fuse;
pub mod lex;
pub mod parse;
pub mod plan;
pub mod ports;
pub mod pred;
pub mod print;
pub mod store_scope;

pub use ast::Query;
pub use exec::{run, Answer, Run};
pub use explain::explain;
pub use explain::why;
pub use fuse::{Fused, Placement, Ranked, RRF_K};
pub use parse::{parse, Parsed};
pub use plan::{
    plan, ClosurePlan, Direction, Env, Lexical, OrderPlan, OutputPlan, PipelinePlan, Plan,
    PlanExpr, PlanOptions, PlanStep, Purity, ShapePlan, Source, TraversalPlan,
};
pub use ports::{Candidates, IdfScope, Partition, PartitionKey, Scope};
pub use print::print;
pub use store_scope::StoreScope;

/// The language versions this build speaks. A `using sq/N` naming anything else is
/// `SMY-E427` (SMYSL-2.5 §3.1).
pub const SUPPORTED_SQ: &[&str] = &["sq/1"];
