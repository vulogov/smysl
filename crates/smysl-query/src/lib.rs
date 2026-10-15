//! `sq/1`: the smysl query language — lexer, parser, syntax tree and canonical printer.
//!
//! The grammar is RFC SMYSL-2.9 and what each stage *means* is SMYSL-2.5 §3.3. This crate is
//! TX-P6 step 1 and stops at the tree: there is no planner, no executor and no store here, and
//! the crate is pure — `smysl-core` for spans and diagnostics, and a normalisation quick-check,
//! which is the whole dependency list.
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
pub mod lex;
pub mod parse;
pub mod print;

pub use ast::Query;
pub use parse::{parse, Parsed};
pub use print::print;

/// The language versions this build speaks. A `using sq/N` naming anything else is
/// `SMY-E427` (SMYSL-2.5 §3.1).
pub const SUPPORTED_SQ: &[&str] = &["sq/1"];
