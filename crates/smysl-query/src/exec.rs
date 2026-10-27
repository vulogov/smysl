//! The executor (SMYSL-2.5 §4.1 `src/exec.rs`, TX-P6 steps 3 and 4).
//!
//! Step 2 planned and stopped. Step 3 ran one source. Step 4 runs the rest of what a pure
//! build can run: filters, traversals, closures, set operations, ordering, shaping and the
//! output stage's choice of writer.
//!
//! # The order is the written order, and that is not an accident
//!
//! SMYSL-2.9 §4.2 fixes the stage order in the *type* — a `where` after an `expand` has
//! nowhere to live in `Pipeline` — so the planner's phase order and the executor's loop are the
//! same order, and neither has to validate it. That is why a filter runs before a traversal
//! here: it is what the query said, and a traversal over a filtered set is a different question
//! from a filter over a traversed one.
//!
//! # Why there is no second index
//!
//! SMYSL-2.5 §3.4 asks for IDF that is the partition's, and asked for it as
//! `Bm25::index_subset` — one BM25 index per partition. TX-P4 step 2 made that unnecessary
//! before anybody wrote it, and TX-P6's reading of §3.4 is the one this file implements: one
//! postings table over the store, and a [`Subset`] view of it per partition. Every statistic
//! BM25 reads — `N`, `df`, `avgdl`, the candidate lists — comes through the `Postings` trait,
//! so restricting the answers restricts the statistics exactly, which `smysl-retrieve`'s own
//! `tests/postings.rs` asserts bit for bit against a table built from those units alone.
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
//! `limit` is a shaping stage, and step 4 lowers it — so a query can now say how many hits it
//! wants, and [`PlanOptions::limit`] is what a source returns *per partition before fusion*,
//! which is a different number. The default is 10 because that is `find --limit`'s default and
//! step 3's exit is that the two agree.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::ids::Uid;
use smysl_core::types::RelKind;
use smysl_graph::{SalienceRequest, Store};
use smysl_retrieve::{
    Analyze, Bm25, MemPostings, Postings, Query as RQuery, Retriever, Subset, Tokenizer,
};

use crate::ast::TimeOp;
use crate::ast::{Atom, Count, Dir};
use crate::fuse::{rrf, Fused, Ranked};
use crate::plan::{
    ClosurePlan, Direction, EntitySource, Env, Lexical, OrderPlan, OutputPlan, PipelinePlan, Plan,
    PlanExpr, PlanOptions, ShapePlan, Source, TimePlan,
};
use crate::ports::{Bounds, IdfScope};
use crate::pred::{holds_in, Field, Reader};

/// What running a plan produced.
///
/// A query with no source is not a degenerate ranking of everything: it named a scope and
/// nothing to rank it by, so what it asks for is the scope. Keeping the two apart means
/// `--why` has nothing to invent for a query that never scored anything.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Answer {
    /// No source, or a source that selects rather than ranks (`about entity`): the units, in
    /// uid order.
    Scope(Vec<Uid>),
    /// A source: fused, highest first.
    Ranked(Vec<Fused>),
    /// `count`.
    Count(usize),
    /// `group by` or `compare by`: one group per distinct value, in value order, with the units
    /// of each in the order they had before grouping.
    Groups(Vec<(String, Vec<Uid>)>),
    /// `pack N tokens`: the selection, in the order the packer chose.
    Packed(Vec<Uid>),
}

impl Answer {
    /// The units this answer names, in the order it names them.
    ///
    /// `Count` names none: it is a number *about* units, and returning the units it counted
    /// would make `count` and `units` the same answer with different labels.
    pub fn uids(&self) -> Vec<Uid> {
        match self {
            Answer::Scope(uids) | Answer::Packed(uids) => uids.clone(),
            Answer::Ranked(fused) => fused.iter().map(|f| f.uid).collect(),
            Answer::Groups(groups) => groups.iter().flat_map(|(_, u)| u.clone()).collect(),
            Answer::Count(_) => Vec::new(),
        }
    }
}

/// An answer and what was worth saying about how it was reached.
///
/// Warnings travel beside the answer rather than in it. SMYSL-2.5 §8 allocates `W463` for a
/// traversal that stopped at `depth` with an unexplored frontier, and that is information about
/// a *successful* run: the result is correct for the depth asked, and the caller should know
/// the graph continued. A type that could only report diagnostics by failing would have to
/// choose between saying nothing and refusing a good answer.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Run {
    pub answer: Answer,
    pub warnings: Vec<Diagnostic>,
}

/// Run a plan.
pub fn run(plan: &Plan, env: &Env<'_>, opts: &PlanOptions) -> Result<Run, Vec<Diagnostic>> {
    let mut warnings = Vec::new();
    // Counts are per run: an earlier run over the same environment (`--seed-check` runs
    // twice) must not add its units to this one's.
    env.reader.take_unmappable();
    let result = run_expr(&plan.expr, env, opts, &mut warnings)?;

    // --- phase 9 -----------------------------------------------------------------------------
    let mut answer = result;
    for stage in &plan.ordering {
        answer = order(answer, stage, env)?;
    }
    for stage in &plan.shaping {
        answer = shape(answer, stage, env, opts)?;
    }
    if let Some(OutputPlan::Count) = plan.output {
        // The one output that changes the answer rather than the writer. `units`, `json` and
        // `show` are three layouts of the same list, and laying it out is step 7's.
        answer = Answer::Count(count_of(&answer));
    }
    lens_warnings(env, &mut warnings);
    Ok(Run { answer, warnings })
}

/// `SMY-W420` for every field a lens could not carry on some unit, with the count (§3.9: the
/// unit is excluded from predicates on that field, and counted).
fn lens_warnings(env: &Env<'_>, warnings: &mut Vec<Diagnostic>) {
    for (key, uids) in env.reader.take_unmappable() {
        warnings.push(Diagnostic::new(Code::W420).with_message(format!(
            "{} unit(s) unmappable on `{key}`: no lens path carries it to the current \
                 revision, so they were excluded from predicates on it",
            uids.len()
        )));
    }
}

/// How many units an answer is about, which is not always how many it names.
fn count_of(answer: &Answer) -> usize {
    match answer {
        Answer::Count(n) => *n,
        Answer::Groups(groups) => groups.iter().map(|(_, u)| u.len()).sum(),
        other => other.uids().len(),
    }
}

/// Phases 3 to 8 for an expression.
fn run_expr(
    expr: &PlanExpr,
    env: &Env<'_>,
    opts: &PlanOptions,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Answer, Vec<Diagnostic>> {
    match expr {
        PlanExpr::Pipeline(p) => run_pipeline(p, env, opts, warnings),
        PlanExpr::Set { op, left, right } => {
            let a = run_expr(left, env, opts, warnings)?;
            let b = run_expr(right, env, opts, warnings)?;
            Ok(Answer::Scope(set_op(*op, &a, &b)))
        }
    }
}

/// Phase 8. `BTreeSet` operations, which is what §3.3 asks for — and what it does *not* ask
/// for is a fused ranking.
///
/// A set operation discards the operands' order. Two rankings produced over two scopes are
/// scored against two different corpora, so combining their positions would be the mistake RRF
/// exists to avoid making within one query — and §3.4 is explicit that RRF is for partitions,
/// never for fusing two things that were ranked separately. So the result is a set, in uid
/// order, and a query that wants it ordered says `order by`.
fn set_op(op: crate::ast::SetOp, a: &Answer, b: &Answer) -> Vec<Uid> {
    let left: BTreeSet<Uid> = a.uids().into_iter().collect();
    let right: BTreeSet<Uid> = b.uids().into_iter().collect();
    match op {
        crate::ast::SetOp::Union => left.union(&right).copied().collect(),
        crate::ast::SetOp::Intersect => left.intersection(&right).copied().collect(),
        crate::ast::SetOp::Except => left.difference(&right).copied().collect(),
    }
}

/// Phases 5 to 7 for one pipeline.
fn run_pipeline(
    p: &PipelinePlan,
    env: &Env<'_>,
    opts: &PlanOptions,
    warnings: &mut Vec<Diagnostic>,
) -> Result<Answer, Vec<Diagnostic>> {
    // --- phase 5 -----------------------------------------------------------------------------
    let mut answer = match &p.source {
        Some(Source::Lexical(lex)) => lexical(p, lex, env, opts)?,
        Some(Source::Entity(e)) => entity(p, e, env, opts)?,
        Some(Source::Pointer(pp)) => {
            let (uids, unmarked) = crate::pointer::select(env.store, pp, &p.candidates);
            if unmarked > 0 {
                warnings.push(Diagnostic::new(Code::W475).with_message(format!(
                    "{unmarked} unit(s) are from snapshots with no version marker, and were \
                     matched against `{}` as written, without a pointer lens",
                    pp.pointer
                )));
            }
            Answer::Scope(uids)
        }
        None => Answer::Scope(p.candidates.iter().copied().collect()),
    };

    // --- phase 6 -----------------------------------------------------------------------------
    //
    // Filtering keeps the order it was given, which for a ranked answer is the ranking. A
    // filter that re-sorted would silently discard what the source computed.
    if !p.filters.is_empty() {
        answer = match answer {
            Answer::Ranked(fused) => Answer::Ranked(
                fused
                    .into_iter()
                    .filter(|f| {
                        p.filters
                            .iter()
                            .all(|pred| holds_in(&env.reader, &f.uid, pred, &opts.params))
                    })
                    .collect(),
            ),
            other => Answer::Scope(
                other
                    .uids()
                    .into_iter()
                    .filter(|uid| {
                        p.filters
                            .iter()
                            .all(|pred| holds_in(&env.reader, uid, pred, &opts.params))
                    })
                    .collect(),
            ),
        };
    }

    // Time stages, after the filters and in the order written. A unit with no time on the
    // axis asked is not a unit that failed the comparison — A-2.4 rule 5 makes an undated
    // interval neither early nor late — so it is set aside and counted, and `SMY-W415` reports
    // the count beside the answer rather than letting a `during` that lost half its units
    // look like one that found half.
    if !p.times.is_empty() {
        let port = env
            .times
            .expect("a time stage lowered, so the port was bound");
        let mut undated = 0usize;
        let keep = |uid: &Uid| -> bool {
            p.times.iter().all(|t| {
                let at = port.of(uid, t.axis);
                at.is_undated() || time_holds(&at, t)
            })
        };
        let mut dated = |uid: &Uid| -> bool {
            let any_undated = p.times.iter().any(|t| port.of(uid, t.axis).is_undated());
            if any_undated {
                undated += 1;
            }
            !any_undated
        };
        answer = match answer {
            Answer::Ranked(fused) => Answer::Ranked(
                fused
                    .into_iter()
                    .filter(|f| keep(&f.uid) && dated(&f.uid))
                    .collect(),
            ),
            other => Answer::Scope(
                other
                    .uids()
                    .into_iter()
                    .filter(|uid| keep(uid) && dated(uid))
                    .collect(),
            ),
        };
        if undated > 0 {
            let span = p.times[0].span;
            warnings.push(Diagnostic::at(Code::W415, span).with_message(format!(
                "{undated} unit(s) excluded for having no time on the axis asked"
            )));
        }
    }

    // --- phase 7 -----------------------------------------------------------------------------
    //
    // A traversal and a closure both *add* to the set they were given — `expand ->grounds` asks
    // for the result and what it rests on, not for the grounds alone. So the ranking of a
    // ranked answer no longer covers every unit in it, and the honest shape for the result is a
    // set. A query that wants the enlarged set ordered says so.
    if !p.traversals.is_empty() || !p.closures.is_empty() {
        let mut reached: BTreeSet<Uid> = answer.uids().into_iter().collect();
        let seeds: Vec<Uid> = reached.iter().copied().collect();

        for t in &p.traversals {
            let depth = match &t.depth {
                Some(Count::Int(n)) => Some(u32::try_from(*n).unwrap_or(u32::MAX)),
                Some(Count::Param(name)) => match opts.params.get(name) {
                    Some(bound) => Some(bound.parse::<u32>().map_err(|_| {
                        vec![Diagnostic::at(Code::E460, t.span).with_message(format!(
                            "`${name}` is bound to `{bound}`, which is not a depth"
                        ))]
                    })?),
                    None => {
                        return Err(vec![Diagnostic::at(Code::E460, t.span).with_message(
                            format!("`${name}` is not bound, so there is no depth"),
                        )])
                    }
                },
                None => None,
            };
            let mut stopped = 0usize;
            for seed in &seeds {
                let walk = match t.direction {
                    Direction::Out => smysl_graph::trace_via(env.store, *seed, &t.edges, depth),
                    Direction::In => {
                        smysl_graph::dependents_trace(env.store, *seed, &t.edges, depth)
                    }
                    Direction::Both => smysl_graph::trace_both(env.store, *seed, &t.edges, depth),
                };
                for node in &walk.nodes {
                    reached.insert(node.uid);
                }
                if let Some(limit) = depth {
                    // `W463`: the walk stopped at `depth` and the graph went on. Detected by
                    // asking whether any unit at the limit has a neighbour the walk did not
                    // report — which is the only way to know, because a `Lineage` records what
                    // was reached and not what was left.
                    let at_limit: Vec<Uid> = walk
                        .nodes
                        .iter()
                        .filter(|n| n.depth == limit)
                        .map(|n| n.uid)
                        .collect();
                    let seen: BTreeSet<Uid> = walk.nodes.iter().map(|n| n.uid).collect();
                    for uid in at_limit {
                        let onward = match t.direction {
                            Direction::In => {
                                smysl_graph::dependents_trace(env.store, uid, &t.edges, Some(1))
                            }
                            Direction::Both => {
                                smysl_graph::trace_both(env.store, uid, &t.edges, Some(1))
                            }
                            Direction::Out => {
                                smysl_graph::trace_via(env.store, uid, &t.edges, Some(1))
                            }
                        };
                        stopped += onward
                            .nodes
                            .iter()
                            .filter(|n| n.depth == 1 && !seen.contains(&n.uid))
                            .count();
                    }
                }
            }
            if stopped > 0 {
                warnings.push(Diagnostic::at(Code::W463, t.span).with_message(format!(
                    "`expand {}` stopped at the depth asked; {stopped} unit(s) beyond the \
                     frontier were not explored",
                    t.label
                )));
            }
        }

        for c in &p.closures {
            for seed in &seeds {
                match c {
                    ClosurePlan::Rebuttals => {
                        reached.extend(env.store.rebuttals_of(seed));
                    }
                    ClosurePlan::Grounds => {
                        if let Some(unit) = env.store.get(seed) {
                            reached.extend(unit.core.grounds.iter().copied());
                        }
                    }
                }
            }
        }
        answer = Answer::Scope(reached.into_iter().collect());
    }

    // --- the supersession pass (SMYSL-2.5 §3.2, §3.3) ----------------------------------------
    //
    // "After sources and traversals", which is here and not in phase 9: a traversal that
    // reaches a superseded unit should hand the pass its replacement, and an ordering should
    // sort what the pass left rather than what it removed.
    if !opts.all_versions {
        answer = supersede(answer, env, warnings);
    }

    Ok(answer)
}

/// Replace superseded units by their newest head; keep every successor of a fork.
///
/// §3.2: *"units with a single newest successor are replaced by it, as relink resolves chains;
/// a fork keeps every successor and raises `SMY-W464`"*. The head computation is
/// `smysl-graph`'s `supersession_heads` — the function `relink` uses, made public in this step
/// for the reason SMYSL-2.5 §4.2 gives: a query and a repair must not disagree about which
/// unit is current.
///
/// # What a fork does, and why it is not "keep the old one"
///
/// A fork is two units superseding one target with nothing ordering them. The single case
/// *replaces*, so the parallel reading replaces too — with all of the successors rather than
/// one, because choosing would be adjudicating a contention that the store has recorded and
/// not settled. The superseded unit goes, as it does in the single case; what the reader gets
/// is both of its replacements and a warning saying so, which is the honest shape of "this was
/// revised twice and the revisions disagree".
///
/// `--all-versions` turns the whole pass off, which is the escape hatch for a reader who wants
/// the history rather than the current reading.
fn supersede(answer: Answer, env: &Env<'_>, warnings: &mut Vec<Diagnostic>) -> Answer {
    let (newest, forked) = smysl_graph::relink::supersession_heads(env.store);
    if newest.is_empty() && forked.is_empty() {
        return answer;
    }
    let forked: BTreeSet<Uid> = forked.into_iter().collect();

    let mut forks_seen = 0usize;
    let mut map = |uid: Uid, out: &mut Vec<Uid>| {
        if let Some(head) = newest.get(&uid) {
            out.push(*head);
        } else if forked.contains(&uid) {
            forks_seen += 1;
            out.extend(direct_successors(env.store, &uid));
        } else {
            out.push(uid);
        }
    };

    let replaced = match &answer {
        // A ranked answer loses its ranking here, and has to: a head that replaced two
        // different hits has no one score, and inventing one would be inventing a ranking.
        // The set is what survives, which is what an ordering stage sorts.
        Answer::Ranked(_) | Answer::Scope(_) | Answer::Packed(_) => {
            let mut out = Vec::new();
            for uid in answer.uids() {
                map(uid, &mut out);
            }
            let deduped: BTreeSet<Uid> = out.into_iter().collect();
            Some(Answer::Scope(deduped.into_iter().collect()))
        }
        // A count or a grouping is produced in phase 9, after this pass; reaching here with
        // one would mean the pipeline ran out of order.
        Answer::Count(_) | Answer::Groups(_) => None,
    };

    if forks_seen > 0 {
        warnings.push(
            Diagnostic::at(Code::W464, smysl_core::diag::Span::new(0, 0)).with_message(format!(
                "{forks_seen} unit(s) in scope are superseded by two or more units that nothing                  orders; no single head exists, so every successor is kept"
            )),
        );
    }
    replaced.unwrap_or(answer)
}

/// The units that directly supersede this one.
///
/// Read from the store rather than returned by `supersession_heads`, which answers "what is
/// the newest" and has nothing to say for a unit whose answer is "two things". Asking the
/// store directly here keeps the contentious computation — which unit wins — in one place.
fn direct_successors(store: &Store, uid: &Uid) -> Vec<Uid> {
    let mut out: Vec<Uid> = store
        .relations_of_kind(&smysl_core::types::RelKind::Supersedes)
        .into_iter()
        .filter(|r| r.to == *uid)
        .map(|r| r.from)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Phase 5 for `~lex`, then §3.4's fusion.
fn lexical(
    p: &PipelinePlan,
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

    // `lang:<code>`: the units in that language, asked of the scope now rather than at
    // planning, because a plan is a function of the query and not of one store's uid sets.
    // `None` cannot happen here — the planner refused the option when the scope could not
    // answer it — but a port is a port, and an answer that changed between planning and
    // running is reported as an empty result rather than guessed around.
    let in_lang: Option<BTreeSet<Uid>> = lex.lang.as_ref().map(|code| {
        env.scope
            .restrict_lang(code, &p.candidates)
            .unwrap_or_default()
    });

    // One table per *analyzer*, built once however many partitions read it. This is the work
    // TX-P4 step 2 moved off the query path, and building it per partition is exactly the cost
    // §3.4 declines to pay. What §3.4 does ask for is that each partition be searched under
    // its own analyzer, and a table is a function of the analyzer that cut its terms — so a
    // catalogued scope over three languages builds three tables, keyed by chain id, and a
    // plain store builds one. The partition's own `chain` is recorded in the plan; the scope
    // supplies the analyzer behind it.
    let plain = Tokenizer::plain();
    let mut tables: BTreeMap<String, MemPostings> = BTreeMap::new();
    // The scope's own postings, when it has them (OQ-91): one table, each unit cut by its own
    // partition's analyzer, over the widest universe this query's IDF scope reads.
    let library_wide: BTreeSet<Uid>;
    let needed = match opts.idf_scope {
        IdfScope::Library => {
            library_wide = env.store.units().map(|(uid, _)| *uid).collect();
            &library_wide
        }
        IdfScope::Partition | IdfScope::Collection => &p.candidates,
    };
    let shared = env.scope.postings(needed);

    let mut lists: Vec<Ranked> = Vec::new();
    // One table for every partition: each term's postings read once for the query and dealt to
    // the partitions, and the statistics a partition is scored by counted once, rather than the
    // whole universe's list filtered, and its statistics counted again, for every partition.
    if let Some(table) = &shared {
        let fan = Fanout::new(&**table, &p.partitions, needed, opts.idf_scope);
        for (idx, partition) in p.partitions.iter().enumerate() {
            let analyzer: &dyn Analyze = env.scope.analyzer(&partition.key).unwrap_or(&plain);
            let within: BTreeSet<Uid> = match &in_lang {
                Some(allowed) => partition.uids.intersection(allowed).copied().collect(),
                None => partition.uids.clone(),
            };
            if within.is_empty() {
                continue;
            }
            let share = fan.share(idx, &partition.uids);
            let scorer = Bm25::over(&share, analyzer, env.store);
            let query = RQuery::new(&text, opts.limit).within(within.iter().copied());
            lists.push(Ranked {
                key: partition.key.clone(),
                hits: scorer.search(&query),
            });
        }
        return Ok(Answer::Ranked(rrf(&lists)));
    }
    for partition in &p.partitions {
        let analyzer: &dyn Analyze = env.scope.analyzer(&partition.key).unwrap_or(&plain);
        let table: &dyn smysl_retrieve::Postings = match &shared {
            Some(t) => &**t,
            None => tables
                .entry(analyzer.id().to_string())
                .or_insert_with(|| MemPostings::index(env.store, analyzer)),
        };

        let universe: BTreeSet<Uid> = match opts.idf_scope {
            IdfScope::Partition => partition.uids.clone(),
            IdfScope::Collection => p.candidates.clone(),
            // `Library` is a wider corpus than the query's scope by definition, and the widest
            // corpus a scope here can name is the store it was opened over.
            IdfScope::Library => env.store.units().map(|(uid, _)| *uid).collect(),
        };

        let within: BTreeSet<Uid> = match &in_lang {
            Some(allowed) => partition.uids.intersection(allowed).copied().collect(),
            None => partition.uids.clone(),
        };
        if within.is_empty() {
            // A partition the language selector emptied contributes nothing to the fusion,
            // and an empty list would still be a list — RRF sums over partitions, and one
            // that ranked nothing has no ranks to sum.
            continue;
        }

        let view = Subset::of(table, &universe);
        let scorer = Bm25::over(&view, analyzer, env.store);
        let query = RQuery::new(&text, opts.limit).within(within.iter().copied());
        lists.push(Ranked {
            key: partition.key.clone(),
            hits: scorer.search(&query),
        });
    }

    Ok(Answer::Ranked(rrf(&lists)))
}

/// A term's postings dealt out by partition: one list a partition, in uid order.
type Dealt = std::rc::Rc<Vec<Vec<(Uid, u32)>>>;

/// One query's postings, read from one table and dealt out among its partitions (OQ-91).
///
/// Exactly what each partition's `Subset` of the table over the IDF universe, filtered to the
/// partition, answers — the same `df`, `docs`, total length, lengths and term frequencies — and
/// `tests` below holds it to that. What changes is the work: a term's postings are read once and
/// split by partition, and the universe's statistics counted once, where they were read and
/// counted once per partition, which over the thousand partitions of a 10⁶-unit library was most
/// of a query (SMYSL-2.8's GE-T7 stand-in).
struct Fanout<'a> {
    table: &'a dyn smysl_retrieve::Postings,
    idf: IdfScope,
    /// The universe's statistics, for an IDF scope wider than the partition.
    universe: Option<Subset<'a>>,
    /// Which partition each candidate is in.
    part_of: BTreeMap<Uid, usize>,
    parts: usize,
    split: std::cell::RefCell<BTreeMap<String, Dealt>>,
    df: std::cell::RefCell<BTreeMap<String, u32>>,
}

impl<'a> Fanout<'a> {
    fn new(
        table: &'a dyn smysl_retrieve::Postings,
        partitions: &[crate::ports::Partition],
        universe: &'a BTreeSet<Uid>,
        idf: IdfScope,
    ) -> Fanout<'a> {
        let mut part_of = BTreeMap::new();
        for (i, p) in partitions.iter().enumerate() {
            for uid in &p.uids {
                part_of.insert(*uid, i);
            }
        }
        Fanout {
            table,
            idf,
            universe: (idf != IdfScope::Partition).then(|| Subset::of(table, universe)),
            part_of,
            parts: partitions.len(),
            split: Default::default(),
            df: Default::default(),
        }
    }

    /// A term's postings, by partition.
    fn split(&self, term: &str) -> Dealt {
        if let Some(s) = self.split.borrow().get(term) {
            return s.clone();
        }
        let mut out: Vec<Vec<(Uid, u32)>> = vec![Vec::new(); self.parts];
        for (uid, tf) in self.table.postings(term) {
            if let Some(&i) = self.part_of.get(&uid) {
                out[i].push((uid, tf));
            }
        }
        let out = std::rc::Rc::new(out);
        self.split
            .borrow_mut()
            .insert(term.to_string(), out.clone());
        out
    }

    fn universe_df(&self, term: &str) -> u32 {
        if let Some(df) = self.df.borrow().get(term) {
            return *df;
        }
        let df = self.universe.as_ref().map_or(0, |u| u.df(term));
        self.df.borrow_mut().insert(term.to_string(), df);
        df
    }

    /// Partition `idx`, scored by the statistics its IDF scope names.
    fn share(&'a self, idx: usize, uids: &BTreeSet<Uid>) -> Share<'a> {
        let (docs, total_len) = match &self.universe {
            Some(u) => (u.docs(), u.total_len()),
            None => {
                let mut docs = 0u32;
                let mut total = 0u64;
                for uid in uids {
                    if let Some(len) = self.table.doc_len(uid) {
                        docs += 1;
                        total += u64::from(len);
                    }
                }
                (docs, total)
            }
        };
        Share {
            fan: self,
            idx,
            docs,
            total_len,
        }
    }
}

/// One partition's share of a [`Fanout`].
struct Share<'a> {
    fan: &'a Fanout<'a>,
    idx: usize,
    docs: u32,
    total_len: u64,
}

impl smysl_retrieve::Postings for Share<'_> {
    fn df(&self, term: &str) -> u32 {
        match self.fan.idf {
            IdfScope::Partition => self.fan.split(term)[self.idx].len() as u32,
            IdfScope::Collection | IdfScope::Library => self.fan.universe_df(term),
        }
    }
    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        self.fan.split(term)[self.idx].clone()
    }
    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        match &self.fan.universe {
            Some(u) => u.doc_len(uid),
            None if self.fan.part_of.get(uid) == Some(&self.idx) => self.fan.table.doc_len(uid),
            None => None,
        }
    }
    /// From this partition's own share of the term's postings, which holds every unit of the
    /// partition that has the term: the table would be asked in every partition it spans, and a
    /// library's thousand partitions asked once a hit was most of a query read from disk.
    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        if self.fan.part_of.get(uid) == Some(&self.idx) {
            let share = self.fan.split(term);
            let list = &share[self.idx];
            return list
                .binary_search_by(|(u, _)| u.cmp(uid))
                .map_or(0, |i| list[i].1);
        }
        match &self.fan.universe {
            Some(u) => u.tf(term, uid),
            None => 0,
        }
    }
    fn docs(&self) -> u32 {
        self.docs
    }
    fn total_len(&self) -> u64 {
        self.total_len
    }
    fn analyzer(&self) -> Option<&str> {
        self.fan.table.analyzer()
    }
}

/// `about entity("…")`: resolve the name through the port, then walk `x.text/mentions`.
///
/// The walk is the store's own relation list rather than an `Adjacency` traversal, because the
/// question has depth one and a fixed direction — which units point *at* these entities — and
/// a one-step reverse lookup over one relation kind is a filter over the edges, not a walk.
/// The result stays within the pipeline's candidates: a mention from a unit outside the scope
/// is a fact about the store and not an answer to this query.
fn entity(
    p: &PipelinePlan,
    e: &EntitySource,
    env: &Env<'_>,
    opts: &PlanOptions,
) -> Result<Answer, Vec<Diagnostic>> {
    let name = match &e.name {
        Atom::Str(s) => s.clone(),
        Atom::Param(name) => match opts.params.get(name) {
            Some(bound) => bound.clone(),
            None => {
                return Err(vec![Diagnostic::at(Code::E460, e.span).with_message(
                    format!("`${name}` is not bound, so there is no entity to resolve"),
                )])
            }
        },
    };
    let port = env
        .entities
        .expect("an entity source lowered, so the port was bound");
    let entities: BTreeSet<Uid> = port.resolve(&name).into_iter().collect();
    if entities.is_empty() {
        return Ok(Answer::Scope(Vec::new()));
    }
    let Ok(kind) = RelKind::parse(smysl_core::schemas::text::MENTIONS) else {
        return Ok(Answer::Scope(Vec::new()));
    };
    let mentioning: BTreeSet<Uid> = env
        .store
        .relations_of_kind(&kind)
        .into_iter()
        .filter(|r| entities.contains(&r.to) && p.candidates.contains(&r.from))
        .map(|r| r.from)
        .collect();
    Ok(Answer::Scope(mentioning.into_iter().collect()))
}

/// One time stage against one dated interval.
fn time_holds(at: &Bounds, t: &TimePlan) -> bool {
    match t.op {
        TimeOp::During => at.during(&t.range),
        TimeOp::Overlaps => at.overlaps(&t.range),
        TimeOp::Before => at.before(&t.range),
        TimeOp::After => at.after(&t.range),
    }
}

/// Phase 9: one ordering stage.
fn order(answer: Answer, stage: &OrderPlan, env: &Env<'_>) -> Result<Answer, Vec<Diagnostic>> {
    let uids = answer.uids();
    Ok(match stage {
        OrderPlan::Salience { dir } => {
            // The existing function, seeded with the result and renormalised over it, which is
            // exactly what §3.3 names. A second salience scorer would be a second answer to a
            // question the store already answers.
            let report = smysl_graph::salience(
                env.store,
                &SalienceRequest::default().seeded(uids.iter().copied()),
            );
            let scores = report.renormalise(&uids);
            let mut sorted = uids.clone();
            sorted.sort_by(|a, b| {
                let (x, y) = (
                    scores.get(a).copied().unwrap_or(0.0),
                    scores.get(b).copied().unwrap_or(0.0),
                );
                // Descending by score for `ascending`? No: `order by salience` ascending means
                // least salient first, as written. The default is ascending for every key
                // uniformly (SMYSL-2.9 OQ-81), and a reader who wants the salient ones first
                // says `descending`.
                let cmp = x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal);
                match dir {
                    Dir::Ascending => cmp.then_with(|| a.cmp(b)),
                    Dir::Descending => cmp.reverse().then_with(|| a.cmp(b)),
                }
            });
            Answer::Scope(sorted)
        }
        OrderPlan::Field { field, dir } => {
            let mut sorted = uids.clone();
            sorted.sort_by(|a, b| {
                let cmp = key_of(&env.reader, a, field).cmp(&key_of(&env.reader, b, field));
                match dir {
                    Dir::Ascending => cmp.then_with(|| a.cmp(b)),
                    Dir::Descending => cmp.reverse().then_with(|| a.cmp(b)),
                }
            });
            Answer::Scope(sorted)
        }
        OrderPlan::GroupBy { field } | OrderPlan::CompareBy { field } => {
            let mut groups: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
            for uid in uids {
                groups
                    .entry(group_key(&env.reader, &uid, field))
                    .or_default()
                    .push(uid);
            }
            Answer::Groups(groups.into_iter().collect())
        }
    })
}

/// A sort key for a field: ordered fields sort on their ladder, the rest on their text.
///
/// The pair is `(rank, text)` so one comparator serves both: an ordered field fills the rank
/// and leaves the text empty, an unordered one the reverse. A unit without the field sorts
/// first under `ascending`, which is where "nothing" belongs on a ladder that starts at
/// something.
fn key_of(r: &Reader<'_>, uid: &Uid, field: &Field) -> (Option<i64>, String) {
    crate::pred::sort_key_in(r, uid, field).unwrap_or_default()
}

/// The value a unit is grouped under, with a name for having none.
fn group_key(r: &Reader<'_>, uid: &Uid, field: &Field) -> String {
    crate::pred::field_text_in(r, uid, field).unwrap_or_else(|| "(none)".to_string())
}

/// Phase 9: one shaping stage.
fn shape(
    answer: Answer,
    stage: &ShapePlan,
    env: &Env<'_>,
    opts: &PlanOptions,
) -> Result<Answer, Vec<Diagnostic>> {
    match stage {
        ShapePlan::Limit { count } => {
            let n = resolve_count(count, opts)?;
            Ok(match answer {
                // Truncation respects whatever order the answer is in, which is `Vec::truncate`
                // and is what §3.3 names. A `limit` before an `order by` is impossible: the
                // grammar puts ordering before shaping (SMYSL-2.9 §4.2).
                Answer::Ranked(mut fused) => {
                    fused.truncate(n);
                    Answer::Ranked(fused)
                }
                Answer::Groups(mut groups) => {
                    // Per group, because a limit after a grouping is about the groups' members:
                    // truncating the group list instead would drop whole values from a
                    // comparison, which is the one thing `compare by` exists to show.
                    for (_, members) in &mut groups {
                        members.truncate(n);
                    }
                    Answer::Groups(groups)
                }
                other => {
                    let mut uids = other.uids();
                    uids.truncate(n);
                    Answer::Scope(uids)
                }
            })
        }
        ShapePlan::Pack { tokens } => {
            let budget = u64::try_from(resolve_count(tokens, opts)?).unwrap_or(u64::MAX);
            let scope: Vec<Uid> = answer.uids();
            let report = smysl_graph::salience(
                env.store,
                &SalienceRequest::default().seeded(scope.iter().copied()),
            );
            let request = smysl_pack::PackRequest::budget(budget)
                .scoped(scope.iter().copied())
                .focusing(scope.iter().copied());
            // A pack can be infeasible — a budget smaller than the closure a unit obliges.
            // `SMY-E461` would be wrong (the stage *is* available); the packer's own error is
            // the honest one, carried through as a diagnostic so a query says what `pack` says.
            let packed = smysl_pack::pack(env.store, &report, &request).map_err(|e| {
                vec![
                    Diagnostic::at(Code::E460, smysl_core::diag::Span::new(0, 0))
                        .with_message(format!("`pack` cannot satisfy this budget: {e}")),
                ]
            })?;
            // `Selection` is `uid -> level of detail`, which is a map and therefore in uid
            // order. The packer's own ordering of a selection is the salience it was given, so
            // the units come back sorted by that and ties by uid — a function of the store and
            // the budget, which is what rule D asks of a stage.
            let mut chosen: Vec<Uid> = packed.selection.keys().copied().collect();
            let scores = report.renormalise(&chosen);
            chosen.sort_by(|a, b| {
                let (x, y) = (
                    scores.get(a).copied().unwrap_or(0.0),
                    scores.get(b).copied().unwrap_or(0.0),
                );
                y.partial_cmp(&x)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.cmp(b))
            });
            Ok(Answer::Packed(chosen))
        }
    }
}

/// A `Count`, with `$name` bound.
fn resolve_count(count: &Count, opts: &PlanOptions) -> Result<usize, Vec<Diagnostic>> {
    match count {
        Count::Int(n) => Ok(usize::try_from(*n).unwrap_or(0)),
        Count::Param(name) => match opts.params.get(name) {
            Some(bound) => bound.parse::<usize>().map_err(|_| {
                vec![
                    Diagnostic::at(Code::E460, smysl_core::diag::Span::new(0, 0)).with_message(
                        format!("`${name}` is bound to `{bound}`, which is not a count"),
                    ),
                ]
            }),
            None => Err(vec![Diagnostic::at(
                Code::E460,
                smysl_core::diag::Span::new(0, 0),
            )
            .with_message(format!("`${name}` is not bound, so there is no count"))]),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::Partition;
    use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};

    /// The fan-out answers each partition exactly as that partition's `Subset` of the table over
    /// its IDF universe, filtered to the partition, does: the same hits, the same scores, for
    /// every IDF scope, over random stores, partitions and queries.
    #[test]
    fn a_partitions_share_scores_as_its_subset_of_the_universe_does() {
        let words = [
            "ruth", "naomi", "boaz", "field", "barley", "lord", "the", "and", "moab", "kinsman",
        ];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = |n: usize| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x % n as u64) as usize
        };
        for case in 0..24 {
            let n = 20 + next(60);
            let mut records = Vec::new();
            let mut uids = Vec::new();
            for i in 0..n {
                let gist: Vec<&str> = (0..2 + next(6)).map(|_| words[next(words.len())]).collect();
                let core = UnitCoreBuilder::new(
                    KernelType::Claim,
                    format!("{} {case} {i}", gist.join(" ")),
                    Status::Speculative,
                )
                .build()
                .unwrap();
                uids.push(canonical_uid(&core));
                records.push(Record::Unit(core));
            }
            let store = Store::from_records(records);
            let plain = Tokenizer::plain();
            let table = MemPostings::index(&store, &plain);
            let k = 1 + next(5);
            let mut sets: Vec<BTreeSet<Uid>> = vec![BTreeSet::new(); k];
            for u in &uids {
                // Some units in no partition: the candidates are not the store.
                if next(5) > 0 {
                    sets[next(k)].insert(*u);
                }
            }
            let partitions: Vec<Partition> = sets
                .into_iter()
                .enumerate()
                .filter(|(_, s)| !s.is_empty())
                .map(|(i, s)| {
                    Partition::new(
                        crate::ports::PartitionKey::Catalog {
                            manifest: format!("m{i}"),
                            lang: "en".into(),
                        },
                        s,
                        None,
                    )
                })
                .collect();
            let candidates: BTreeSet<Uid> = partitions
                .iter()
                .flat_map(|p| p.uids.iter().copied())
                .collect();
            let everything: BTreeSet<Uid> = uids.iter().copied().collect();
            for idf in [IdfScope::Partition, IdfScope::Collection, IdfScope::Library] {
                let needed = if idf == IdfScope::Library {
                    &everything
                } else {
                    &candidates
                };
                let fan = Fanout::new(&table, &partitions, needed, idf);
                for q in ["ruth", "the lord", "barley field moab", "kinsman and"] {
                    for (idx, p) in partitions.iter().enumerate() {
                        let universe = match idf {
                            IdfScope::Partition => p.uids.clone(),
                            IdfScope::Collection => candidates.clone(),
                            _ => everything.clone(),
                        };
                        let query = RQuery::new(q, 10).within(p.uids.iter().copied());
                        let subset = Subset::of(&table, &universe);
                        let want = Bm25::over(&subset, &plain, &store).search(&query);
                        let share = fan.share(idx, &p.uids);
                        let got = Bm25::over(&share, &plain, &store).search(&query);
                        assert_eq!(
                            format!("{got:?}"),
                            format!("{want:?}"),
                            "case {case}, {idf:?}, `{q}`, partition {idx}"
                        );
                    }
                }
            }
        }
    }
}
