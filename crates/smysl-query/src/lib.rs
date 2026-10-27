//! `sq/1`: the smysl query language — lexer, parser, syntax tree, canonical printer, planner
//! and executor.
//!
//! The grammar is RFC SMYSL-2.9 and what each stage *means* is SMYSL-2.5 §3.3. TX-P6 step 1
//! built everything up to the tree; step 2 the [`ports::Scope`] port and planner phases 1 to 4
//! with [`explain()`]; step 3 the lexical source and RRF; step 4 filters through outputs; step
//! 5 the supersession pass; step 6 the ports the library layer binds — [`ports::Times`],
//! [`ports::Entities`], and a scope that knows its languages. TX-P8 step 1 added [`saved`]:
//! a query saved as an `x.query/saved` unit after it has run, and run again with the data
//! effect reported. Step 3 added [`dossier`]: a question classified by a rule table, its
//! subject resolved, and a fixed set of slot queries run and saved, as a `dossier` thread and
//! a view. A stage this build cannot lower
//! is **refused** with `SMY-E461` and what it is waiting for named, never ignored: a plan that
//! quietly dropped a stage would answer a different question from the one asked, and
//! SMYSL-2.5 §3.6 only lets a query be saved after it has planned and run.
//!
//! The crate stays pure, and it reaches no catalogue. `smysl-core` gives spans and
//! diagnostics, `smysl-graph` the store, `smysl-retrieve` the scorer, `smysl-pack` the packer,
//! a normalisation quick-check serves `SMY-W510`, and that is the whole dependency list;
//! `smysl-text` depends on *this* crate for the port traits, which is why nothing here can
//! name a library.
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
pub mod dossier;
pub mod exec;
pub mod explain;
pub mod fold;
pub mod fuse;
pub mod lex;
pub mod nl;
pub mod parse;
pub mod plan;
pub mod pointer;
pub mod ports;
pub mod pred;
pub mod print;
pub mod resolve;
pub mod saved;
pub mod store_scope;

pub use ast::Query;
pub use dossier::{ask, AskError, AskOptions, Dossier, Intent, SlotResult};
pub use exec::{run, Answer, Run};
pub use explain::explain;
pub use explain::why;
pub use fuse::{Fused, Placement, Ranked, RRF_K};
pub use nl::{accept, Accepted, TranslateContext, TranslateError, Translator};
pub use parse::{parse, Parsed};
pub use plan::{
    plan, ClosurePlan, Direction, EntitySource, Env, Lexical, OrderPlan, OutputPlan, PipelinePlan,
    Plan, PlanExpr, PlanOptions, PlanStep, Purity, ShapePlan, Source, TimePlan, TraversalPlan,
};
pub use ports::{
    routes_to_lexical, Axis, Bounds, Candidates, Entities, IdfScope, Partition, PartitionKey,
    RetrieverSource, Scope, SemanticSource, Times,
};
pub use print::print;
pub use saved::{bind, load, rerun, to_records, Recipe, Rerun, SavedQuery, SetDiff};
pub use store_scope::StoreScope;

/// The language versions this build speaks. A `using sq/N` naming anything else is
/// `SMY-E427` (SMYSL-2.5 §3.1).
pub const SUPPORTED_SQ: &[&str] = &["sq/1"];
