//! The executor (SMYSL-2.5 §4.1 `src/exec.rs`, TX-P6 step 3).
//!
//! Step 2 planned and stopped. This runs what it planned, for the one source phase 5 can lower
//! in a pure build: `find "…" ~lex`.
//!
//! # Why there is no second index
//!
//! SMYSL-2.5 §3.4 asks for IDF that is the partition's, and asked for it as
//! `Bm25::index_subset` — one BM25 index per partition. TX-P4 step 2 made that unnecessary
//! before anybody wrote it, and TX-P6's reading of §3.4 is the one this file implements: one
//! postings table over the store, and a [`Subset`] view of it per
//! partition. Every statistic BM25 reads — `N`, `df`, `avgdl`, the candidate lists — comes
//! through the `Postings` trait, so restricting the answers restricts the statistics exactly,
//! which `smysl-retrieve`'s own `tests/postings.rs` asserts bit for bit against a table built
//! from those units alone.
//!
//! # Two restrictions that are not the same restriction
//!
//! A partition decides *which units may be returned*; the IDF scope decides *what the term
//! statistics are counted over*. They are different sets whenever the scope is not
//! `partition`, and `smysl-retrieve` already separates them: `Query::within` restricts
//! candidates and says in its own documentation that IDF still comes from the whole index. So
//! the view sets the universe and `within` sets the partition, and the default IDF scope —
//! `collection`, per SMYSL-2.5 §3.2 phase 4, the only one of the three without a measured
//! defect — scores every partition against the whole candidate set while still ranking within
//! each.
//!
//! # What limits the hits
//!
//! `limit` is a shaping stage and shaping is phase 9, which step 4 lowers — so a query cannot
//! yet say how many hits it wants. [`PlanOptions::limit`] is that number until it can, and its
//! default is 10 because that is `find --limit`'s default and this step's exit is that the two
//! agree. It is applied per partition before fusion, which is how RRF is ordinarily run and
//! which for a single partition is the same list either way.

use std::collections::BTreeSet;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::ids::Uid;
use smysl_retrieve::{Bm25, MemPostings, Query as RQuery, Retriever, Subset, Tokenizer};

use crate::ast::Atom;
use crate::fuse::{rrf, Fused, Ranked};
use crate::plan::{Env, Lexical, Plan, PlanOptions, Source};
use crate::ports::IdfScope;

/// What running a plan produced.
///
/// A query with no source is not a degenerate ranking of everything: it named a scope and
/// nothing to rank it by, so what it asks for is the scope. Keeping the two apart means
/// `--why` has nothing to invent for a query that never scored anything.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Answer {
    /// No source: the candidate set, in uid order.
    Scope(Vec<Uid>),
    /// A source: fused, highest first.
    Ranked(Vec<Fused>),
}

impl Answer {
    /// The units this answer names, in the order it names them.
    pub fn uids(&self) -> Vec<Uid> {
        match self {
            Answer::Scope(uids) => uids.clone(),
            Answer::Ranked(fused) => fused.iter().map(|f| f.uid).collect(),
        }
    }
}

/// Run a plan.
pub fn run(plan: &Plan, env: &Env<'_>, opts: &PlanOptions) -> Result<Answer, Vec<Diagnostic>> {
    let Some(source) = &plan.source else {
        return Ok(Answer::Scope(plan.candidates.iter().copied().collect()));
    };

    match source {
        Source::Lexical(lex) => lexical(plan, lex, env, opts),
    }
}

/// Phase 5 for `~lex`, then §3.4's fusion.
fn lexical(
    plan: &Plan,
    lex: &Lexical,
    env: &Env<'_>,
    opts: &PlanOptions,
) -> Result<Answer, Vec<Diagnostic>> {
    let text = match &lex.text {
        Atom::Str(s) => s.clone(),
        // SMYSL-2.9 §4.10: an unbound parameter is `SMY-E460` *at execution*, with the span of
        // the `$name` — which is this site and not the planner, because binding is something a
        // caller does between planning and running.
        Atom::Param(name) => match opts.params.get(name) {
            Some(bound) => bound.clone(),
            None => {
                return Err(vec![Diagnostic::at(Code::E460, lex.span).with_message(
                    format!("`${name}` is not bound, so there is no text to search for"),
                )])
            }
        },
    };

    // One table for the store, built once however many partitions read it. This is the work
    // TX-P4 step 2 moved off the query path, and building it per partition is exactly the cost
    // §3.4 declines to pay.
    let analyzer = Tokenizer::plain();
    let whole = MemPostings::index(env.store, &analyzer);

    let mut lists: Vec<Ranked> = Vec::new();
    for partition in &plan.partitions {
        let universe: BTreeSet<Uid> = match opts.idf_scope {
            IdfScope::Partition => partition.uids.clone(),
            IdfScope::Collection => plan.candidates.clone(),
            // `Library` is a wider corpus than the query's scope by definition, and until
            // `smysl-text` supplies a library the widest corpus here is the store.
            IdfScope::Library => env.store.units().map(|(uid, _)| *uid).collect(),
        };

        let view = Subset::of(&whole, &universe);
        let scorer = Bm25::over(&view, &analyzer, env.store);
        let query = RQuery::new(&text, opts.limit).within(partition.uids.iter().copied());
        lists.push(Ranked {
            key: partition.key.clone(),
            hits: scorer.search(&query),
        });
    }

    Ok(Answer::Ranked(rrf(&lists)))
}
