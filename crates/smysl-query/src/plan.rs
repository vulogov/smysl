//! The planner (SMYSL-2.5 §3.2), phases 1 to 4.
//!
//! §3.2 lowers a query in nine phases and this build has the first four: header, field
//! resolution, scope, partitions. The other five arrive with the stages they lower — sources
//! in TX-P6 step 3, filters, traversals and the rest in step 4 — and **a stage this build
//! cannot lower is refused rather than ignored**, with `SMY-E461`, whose own meaning in
//! SMYSL-2.5 §8 is "a stage is not available in this build or phase". Ignoring it would make a
//! plan that silently answered a different question from the one asked, and SMYSL-2.5 §3.6 now
//! requires a successful plan *and run* before a query can be saved (OQ-82), so a half-lowered
//! plan must not exist in the first place.
//!
//! What each phase is allowed to conclude is set by what the build can read, not by what the
//! RFC describes, and the gap is in one place:
//!
//! - **Phase 1** resolves `using` pins. `using sq/N` is settled in the parser (`SMY-E427`). A
//!   schema pin is checked against the declarations the store holds — which is what
//!   `SMY-E466` says it checks — rather than against the `SchemaSet` of §4.2, because
//!   `smysl-core` has no `schema/` module yet: that is TX-P12 step 1.
//! - **Phase 2** resolves names. Relation kinds are resolvable today, from the fourteen kernel
//!   kinds and the `relations` every declaration carries, so `SMY-E462` is raised here. Field
//!   names are **not**: a declaration's `payload_shape` is opaque CBOR until FC-9 decoding
//!   lands, so no field can be refused and `SMY-E421` would be a code with nothing behind it.
//!   The plan records that the check did not run, because a resolution phase that quietly
//!   resolves nothing is worse than one that is absent. The typed path is TX-P12 step 3, which
//!   names `E421`/`E422` as its own.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::Uid;
use smysl_core::types::{Record, RelKind};
use smysl_graph::{EdgeKind, EdgeSet, Store};

use crate::ast::{
    Atom, ClosureItem, Count, Dir, EdgeSpec, Engine, Expr, LangSel, NamedEdge, Ordering, Output,
    OutputStage, Pipeline, Query, SetOp, Shaping, SortTarget, Source as AstSource, Using,
};
use crate::ports::{item_text, Candidates, IdfScope, Partition, Scope};
use crate::pred::Field;

/// What the planner reads the world through (SMYSL-2.5 §4.2).
///
/// §4.2 sketches six fields. Three of them name types this build does not have — `SchemaSet`
/// (TX-P12 step 1), `SemanticSource` (TX-P10) and `Witnesses` (TX-P8) — and `TimeIndex`
/// belongs to the time stages of step 4. They arrive with their phases; the struct is
/// `#[non_exhaustive]` so each is an addition rather than a break.
#[non_exhaustive]
pub struct Env<'a> {
    pub store: &'a Store,
    pub scope: &'a dyn Scope,
}

impl<'a> Env<'a> {
    pub fn new(store: &'a Store, scope: &'a dyn Scope) -> Env<'a> {
        Env { store, scope }
    }
}

/// Options a caller sets before planning (SMYSL-2.5 §4.2).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PlanOptions {
    pub idf_scope: IdfScope,
    pub all_versions: bool,
    /// Bindings for `$name` (SMYSL-2.9 §4.10). An unbound parameter is `SMY-E460` at
    /// execution and not at planning, because binding is what a caller does between the two —
    /// a plan is reusable across bindings and that is most of why parameters exist.
    pub params: BTreeMap<String, String>,
    /// How many hits a source may return, per partition, before fusion.
    ///
    /// Here rather than in the query because `limit` is a shaping stage, shaping is phase 9,
    /// and step 4 lowers it; this is the number until a query can say one. The default is 10
    /// because that is `find --limit`'s default and TX-P6 step 3's exit is that the two
    /// rankings agree.
    pub limit: usize,
}

impl Default for PlanOptions {
    fn default() -> PlanOptions {
        PlanOptions {
            idf_scope: IdfScope::default(),
            all_versions: false,
            params: BTreeMap::new(),
            limit: 10,
        }
    }
}

/// Whether running this plan can reach a model (SMYSL-2.5 §3.2).
///
/// Every step phases 1 to 4 produce is pure, and the stages that are not — `~sem`, `near`, a
/// bridge, an intent classifier — are refused by this build before purity is asked. The type
/// is here because the plan must carry it from the start: a plan that gains a purity field
/// later is a plan that was once saved without one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Purity {
    Pure,
    ModelDependent(Vec<&'static str>),
}

/// A lowered source (SMYSL-2.5 §3.2 phase 5).
///
/// One variant, because one source lowers in a pure build with no library: `~sem` needs a
/// model (TX-P10), `~hybrid` needs the routing decision that chooses between them, and the
/// other six sources are the library layer's or a later phase's. Each is refused by name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    Lexical(Lexical),
}

/// `find "…" ~lex` (SMYSL-2.5 §3.3, §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Lexical {
    /// The query text, or the parameter standing in for it. Unresolved on purpose: a plan is
    /// a function of the query and not of one caller's bindings.
    pub text: Atom,
    /// Whether the query wrote `~lex` or left the engine out.
    ///
    /// Recorded because the two are different queries that plan the same way, and a saved
    /// query has to give back what was written. Which they mean is settled in SMYSL-2.9 §9
    /// (OQ-83): an absent engine is lexical.
    pub explicit: bool,
    pub span: Span,
}

/// A schema pinned by a `using` clause, with the revision it resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub id: String,
    pub revision: u32,
}

/// What one phase did. `--explain` prints these in order (SMYSL-2.5 §3.2: each phase "records
/// what it did, so `--explain` prints the plan rather than reconstructing it").
///
/// Not `Eq`: phase 5 records RRF's `k`, which is an `f64`. Recording it is deliberate — §3.6
/// hashes `k` with seven other inputs into a saved query's identity, and a plan that used a
/// constant without saying which is a plan nobody can check against the recipe it was saved
/// under.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlanStep {
    /// Phase 1.
    Header { sq: u32, pins: Vec<Pin> },
    /// Phase 2. `fields_checked` is false while payload shapes are opaque.
    Resolve {
        relations: Vec<String>,
        fields_checked: bool,
    },
    /// Phase 3.
    Scope {
        scope: String,
        items: Vec<String>,
        stores: Vec<String>,
        candidates: usize,
    },
    /// Phase 4.
    Partition {
        partitions: Vec<(String, usize)>,
        idf_scope: IdfScope,
    },
    /// Phase 6.
    Filter { stages: usize },
    /// Phase 7.
    Expand {
        traversals: Vec<String>,
        closures: Vec<&'static str>,
    },
    /// The supersession pass, which runs after sources and traversals (SMYSL-2.5 §3.2).
    ///
    /// Recorded even when it is off, because "this reader asked for every version" is a fact
    /// about the answer and a plan that stayed silent would read like one where the pass found
    /// nothing to do.
    Supersede { enabled: bool },
    /// Phase 8.
    SetOp { op: &'static str, span: Span },
    /// Phase 9.
    Shape {
        ordering: Vec<String>,
        shaping: Vec<String>,
        output: Option<&'static str>,
    },
    /// Phase 5.
    Source {
        engine: &'static str,
        /// The text as written — a quoted string or a `$name` — because a plan prints what
        /// the query said and not what some binding made of it.
        text: String,
        /// Per partition, before fusion.
        limit: usize,
        /// `k`, from §3.4. Printed because it is one of the eight recipe inputs and the only
        /// one this build fixes.
        rrf_k: f64,
    },
}

/// One traversal, lowered (SMYSL-2.5 §3.3, phase 7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TraversalPlan {
    /// How it was written, for `--explain`.
    pub label: String,
    pub edges: EdgeSet,
    pub direction: Direction,
    /// Unresolved: a `$parameter` depth is bound between planning and running, like a
    /// parameter anywhere else.
    pub depth: Option<Count>,
    pub span: Span,
}

/// Which way a traversal walks.
///
/// Three directions and three graph walks, which is why `smysl-graph` grew two functions in
/// this step rather than one: `trace_via` existed, `dependents_trace` is its mirror with a
/// depth, and `trace_both` is the walk that may change direction mid-path — strictly more than
/// the union of the other two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Direction {
    /// `expand ->k`: what the unit rests on.
    Out,
    /// `expand <-k`: what rests on the unit.
    In,
    /// `expand -[k]-`: either way.
    Both,
}

/// One closure item, lowered (phase 7).
///
/// Two of the six. `holders` and `speakers` read payload uid keys *through lenses* (§3.3,
/// §3.9), which is TX-P12's, and `spans` and `datings` are SMYSL-2.4's — so four are refused by
/// name. The two here need nothing this build does not have: a rebuttal is an edge the store
/// already answers for, and grounds are a kernel field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ClosurePlan {
    Rebuttals,
    Grounds,
}

/// One pipeline, lowered: phases 3 to 7 for one branch of a set expression.
///
/// This is what `Plan` was before TX-P6 step 4. A set expression has a pipeline per branch,
/// each with its own scope, partitions and source, so the per-pipeline record had to become a
/// type of its own — `from store:a | find "x" union from store:b | find "y"` resolves two
/// scopes and two rankings before anything is combined.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PipelinePlan {
    pub scope: String,
    pub items: Vec<String>,
    pub stores: Vec<String>,
    pub candidates: BTreeSet<Uid>,
    pub partitions: Vec<Partition>,
    pub source: Option<Source>,
    /// Phase 6, in written order. Conjunctive: a pipeline's `where` stages all have to hold,
    /// which is what writing two of them means (SMYSL-2.9 §4.2).
    pub filters: Vec<crate::pred::Lowered>,
    /// Phase 7, in written order.
    pub traversals: Vec<TraversalPlan>,
    pub closures: Vec<ClosurePlan>,
}

/// A set expression over pipelines (phase 8).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlanExpr {
    /// Boxed because a `PipelinePlan` is far larger than a pair of branches, and an unboxed
    /// pair of variants that differ by an order of magnitude makes every `PlanExpr` the size
    /// of the biggest. A set expression is the rare shape; the pipeline is the common one.
    Pipeline(Box<PipelinePlan>),
    Set {
        op: SetOp,
        left: Box<PlanExpr>,
        right: Box<PlanExpr>,
    },
}

impl PlanExpr {
    /// The one pipeline, for a query that is not a set expression.
    ///
    /// Most queries are not, and a caller that wants the candidate set of a simple query
    /// should not have to walk a tree of one node to get it.
    pub fn sole(&self) -> Option<&PipelinePlan> {
        match self {
            PlanExpr::Pipeline(p) => Some(p),
            PlanExpr::Set { .. } => None,
        }
    }
}

/// An ordering stage, lowered (phase 9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OrderPlan {
    /// `order by salience`, through `smysl-graph`'s salience seeded with the result and
    /// renormalised over it — the existing function §3.3 names, not a second scorer.
    Salience {
        dir: Dir,
    },
    Field {
        field: Field,
        dir: Dir,
    },
    /// `group by <field>`: the result becomes groups, one per distinct value.
    GroupBy {
        field: Field,
    },
    /// `compare by <field>`: the same partition of the result, presented side by side. The
    /// shape is a group list because that is what the data is; what differs is how a writer
    /// lays it out, and writers are step 7's.
    CompareBy {
        field: Field,
    },
}

/// A shaping stage, lowered (phase 9).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ShapePlan {
    Limit { count: Count },
    Pack { tokens: Count },
}

/// An output stage, lowered (phase 9).
///
/// An output selects a *writer*, and writers are the CLI's — step 7's `q`. What differs here is
/// only whether the answer is a number or a list, so `Units`, `Json` and `Show` lower to the
/// same computation and differ in the tag a caller reads to decide how to print it. Saying that
/// out loud is better than three identical arms pretending to be three behaviours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputPlan {
    Count,
    Units,
    Json,
    Show,
}

/// A lowered query (SMYSL-2.5 §4.2).
///
/// `recipe_hash` is **not** here. SMYSL-2.5 §3.6 defines the recipe over eight inputs — sq
/// version, analyzer ids, IDF scope, estimator id, RRF `k`, class policy, template-set id,
/// parameters — of which this build has two. A hash over a subset would be a saved query's
/// identity changing silently when the other six arrive, which is the one thing a recipe hash
/// exists to prevent. It lands with TX-P8, where saving does.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Plan {
    pub sq: u32,
    pub purity: Purity,
    pub steps: Vec<PlanStep>,
    pub pins: Vec<Pin>,
    /// Phases 3 to 8: a pipeline, or a tree of them joined by set operators.
    pub expr: PlanExpr,
    /// Phase 9, in written order.
    pub ordering: Vec<OrderPlan>,
    pub shaping: Vec<ShapePlan>,
    pub output: Option<OutputPlan>,
}

impl Plan {
    /// The candidate set of a query that is not a set expression.
    ///
    /// Kept as a method after TX-P6 step 4 moved the field onto [`PipelinePlan`]: it is what
    /// most callers of a simple query's plan actually wanted, and a set expression has no one
    /// candidate set to return — hence the `Option` rather than a union nobody asked for.
    pub fn candidates(&self) -> Option<&BTreeSet<Uid>> {
        self.expr.sole().map(|p| &p.candidates)
    }

    /// The source of a query that is not a set expression. See [`Plan::candidates`].
    pub fn source(&self) -> Option<&Source> {
        self.expr.sole().and_then(|p| p.source.as_ref())
    }

    /// The partitions of a query that is not a set expression. See [`Plan::candidates`].
    pub fn partitions(&self) -> &[Partition] {
        self.expr.sole().map(|p| &p.partitions[..]).unwrap_or(&[])
    }
}

/// Lower a query through phases 1 to 9.
pub fn plan(q: &Query, env: &Env<'_>, opts: &PlanOptions) -> Result<Plan, Vec<Diagnostic>> {
    let mut diags = Vec::new();

    // --- phase 1: header ---------------------------------------------------------------------
    let mut sq = 1;
    let mut pins: Vec<Pin> = Vec::new();
    for using in &q.header {
        match using {
            Using::Sq { version, .. } => sq = *version,
            Using::Schema { id, revision, span } => {
                if !declares(env.store, id, *revision) {
                    diags.push(Diagnostic::at(Code::E466, *span).with_message(format!(
                        "no declaration of `{id}` at revision {revision} in this store, so the \
                         query cannot be pinned to it"
                    )));
                } else {
                    pins.push(Pin {
                        id: id.clone(),
                        revision: *revision,
                    });
                }
            }
        }
    }

    // --- phases 2 to 8 -----------------------------------------------------------------------
    let mut steps = vec![PlanStep::Header {
        sq,
        pins: pins.clone(),
    }];
    let expr = lower_expr(&q.expr, env, opts, &mut steps, &mut diags);

    // --- phase 9: ordering, shaping, output --------------------------------------------------
    let mut ordering = Vec::new();
    for o in &q.ordering {
        if let Some(lowered) = lower_ordering(o, &mut diags) {
            ordering.push(lowered);
        }
    }
    let mut shaping = Vec::new();
    for sh in &q.shaping {
        if let Some(lowered) = lower_shaping(sh, &mut diags) {
            shaping.push(lowered);
        }
    }
    let output = match &q.output {
        Some(stage) => lower_output(stage, &mut diags),
        None => None,
    };

    if !diags.is_empty() {
        return Err(diags);
    }
    let expr = expr.expect("no diagnostics means the expression lowered");

    if !ordering.is_empty() || !shaping.is_empty() || output.is_some() {
        steps.push(PlanStep::Shape {
            ordering: ordering.iter().map(order_name).collect(),
            shaping: shaping.iter().map(shape_name).collect(),
            output: output.map(output_name),
        });
    }

    Ok(Plan {
        sq,
        purity: Purity::Pure,
        steps,
        pins,
        expr,
        ordering,
        shaping,
        output,
    })
}

/// Phases 2 to 8 over a set expression, which is a tree of pipelines.
///
/// Phase 8 is the recursion itself: §3.2 makes set operations a phase, and the only thing that
/// phase does is join what two branches resolved. Each branch is planned whole — its own scope,
/// its own partitions, its own source — because that is what `union` of two queries means.
fn lower_expr(
    expr: &Expr,
    env: &Env<'_>,
    opts: &PlanOptions,
    steps: &mut Vec<PlanStep>,
    diags: &mut Vec<Diagnostic>,
) -> Option<PlanExpr> {
    match expr {
        Expr::Pipeline(p) => {
            lower_pipeline(p, env, opts, steps, diags).map(|p| PlanExpr::Pipeline(Box::new(p)))
        }
        Expr::Set {
            op,
            left,
            right,
            span,
        } => {
            let a = lower_expr(left, env, opts, steps, diags);
            let b = lower_expr(right, env, opts, steps, diags);
            steps.push(PlanStep::SetOp {
                op: set_name(*op),
                span: *span,
            });
            Some(PlanExpr::Set {
                op: *op,
                left: Box::new(a?),
                right: Box::new(b?),
            })
        }
    }
}

/// Phases 2 to 7 for one pipeline.
fn lower_pipeline(
    pipeline: &Pipeline,
    env: &Env<'_>,
    opts: &PlanOptions,
    steps: &mut Vec<PlanStep>,
    diags: &mut Vec<Diagnostic>,
) -> Option<PipelinePlan> {
    // --- phase 2: resolution -----------------------------------------------------------------
    let relations = resolve_relations(env.store, pipeline, diags);
    steps.push(PlanStep::Resolve {
        relations,
        fields_checked: false,
    });

    // --- phase 3: scope ----------------------------------------------------------------------
    let mut items: Vec<String> = Vec::new();
    let candidates = match &pipeline.scope {
        Some(scope) => {
            let mut found = Candidates::new();
            for item in &scope.items {
                items.push(item_text(item));
                match env.scope.resolve(item, scope.span) {
                    Ok(c) => found.absorb(c),
                    Err(d) => diags.push(d),
                }
            }
            found
        }
        // SMYSL-2.9 OQ-79: `from` is optional and the resolved scope is what a saved query
        // records, so an absent `from` resolves to the whole scope here rather than staying
        // absent and being re-resolved somewhere else later.
        None => env.scope.everything(),
    };
    steps.push(PlanStep::Scope {
        scope: env.scope.name().to_string(),
        items,
        stores: candidates.stores.clone(),
        candidates: candidates.uids.len(),
    });

    // --- phase 4: partitions -----------------------------------------------------------------
    let partitions = env.scope.partition(&candidates.uids);
    steps.push(PlanStep::Partition {
        partitions: partitions
            .iter()
            .map(|p| (crate::explain::partition_name(&p.key), p.uids.len()))
            .collect(),
        idf_scope: opts.idf_scope,
    });

    // --- phase 5: sources --------------------------------------------------------------------
    let source = match &pipeline.source {
        Some(stage) => lower_source(stage, diags),
        None => None,
    };
    if let Some(Source::Lexical(lex)) = &source {
        steps.push(PlanStep::Source {
            engine: "lexical",
            text: match &lex.text {
                Atom::Str(s) => format!("{s:?}"),
                Atom::Param(n) => format!("${n}"),
            },
            limit: opts.limit,
            rrf_k: crate::fuse::RRF_K,
        });
    }

    // --- phase 6: filters --------------------------------------------------------------------
    //
    // Time stages are phase 6 too and are **not** lowered here: `said` and `observed` need
    // SMYSL-2.4's effective-time engine and its range index, which is TX-P6 step 6. The
    // observed axis alone is answerable from `Store::observed_at` today, and lowering half an
    // axis pair would make `said during …` and `observed during …` differ in whether they were
    // honoured rather than in what they asked.
    let mut filters = Vec::new();
    for f in &pipeline.filters {
        if f.lens_none {
            // `with lens none` turns off compile-forward for the stage (SMYSL-2.9 §4.7,
            // OQ-77). This build refuses every qualified reference anyway, so the modifier has
            // nothing to turn off — but it is not an error, and silently accepting it is right:
            // a query written for a later build must mean the same thing here or nothing.
        }
        if let Some(lowered) = crate::pred::lower(&f.pred, diags) {
            filters.push(lowered);
        }
    }
    for t in &pipeline.times {
        diags.push(
            Diagnostic::at(Code::E461, t.span).with_message(
                "a time stage reads effective time, which needs the library layer's time index \
             (SMYSL-2.4); this build has a plain store"
                    .to_string(),
            ),
        );
    }
    if !filters.is_empty() {
        steps.push(PlanStep::Filter {
            stages: filters.len(),
        });
    }

    // --- phase 7: traversals and closures ----------------------------------------------------
    let mut traversals = Vec::new();
    for t in &pipeline.traversals {
        if let Some(lowered) = lower_traversal(env.store, t, diags) {
            traversals.push(lowered);
        }
    }
    let mut closures = Vec::new();
    for c in &pipeline.closures {
        for item in &c.items {
            match item {
                ClosureItem::Rebuttals => closures.push(ClosurePlan::Rebuttals),
                ClosureItem::Grounds => closures.push(ClosurePlan::Grounds),
                ClosureItem::Holders | ClosureItem::Speakers => {
                    diags.push(
                        Diagnostic::at(Code::E461, c.span).with_message(
                            "`with holders` and `with speakers` read payload uid keys through \
                         lenses (SMYSL-2.5 §3.9), and this build has no lens engine: that is \
                         TX-P12's"
                                .to_string(),
                        ),
                    );
                }
                ClosureItem::Spans | ClosureItem::Datings => {
                    diags.push(
                        Diagnostic::at(Code::E461, c.span).with_message(
                            "`with spans` and `with datings` are the library layer's (SMYSL-2.4); \
                         a plain store records neither"
                                .to_string(),
                        ),
                    );
                }
            }
        }
    }
    if !traversals.is_empty() || !closures.is_empty() {
        steps.push(PlanStep::Expand {
            traversals: traversals.iter().map(|t| t.label.clone()).collect(),
            closures: closures
                .iter()
                .map(|c| match c {
                    ClosurePlan::Rebuttals => "rebuttals",
                    ClosurePlan::Grounds => "grounds",
                })
                .collect(),
        });
    }

    steps.push(PlanStep::Supersede {
        enabled: !opts.all_versions,
    });

    Some(PipelinePlan {
        scope: env.scope.name().to_string(),
        items: Vec::new(),
        stores: candidates.stores,
        candidates: candidates.uids,
        partitions,
        source,
        filters,
        traversals,
        closures,
    })
}

/// Phase 7 for one traversal: which edges, which way, how far.
fn lower_traversal(
    store: &Store,
    t: &crate::ast::Traversal,
    diags: &mut Vec<Diagnostic>,
) -> Option<TraversalPlan> {
    let (label, edges, direction) =
        match &t.edge {
            EdgeSpec::Out(k) => (
                format!("->{k}"),
                edge_set(store, k, t.span, diags)?,
                Direction::Out,
            ),
            EdgeSpec::In(k) => (
                format!("<-{k}"),
                edge_set(store, k, t.span, diags)?,
                Direction::In,
            ),
            EdgeSpec::Both(k) => (
                format!("-[{k}]-"),
                edge_set(store, k, t.span, diags)?,
                Direction::Both,
            ),
            EdgeSpec::Named(named) => match named {
                // The spelled traversals are graph operations, and §3.3 gives each one an existing
                // function with an edge preset. `rests on` and `dependents` are the two that §3.3
                // says gain a depth through the new BFS, which is what this step added.
                NamedEdge::Grounds => ("grounds".to_string(), EdgeSet::support(), Direction::Out),
                NamedEdge::Deps => ("deps".to_string(), EdgeSet::support(), Direction::Out),
                NamedEdge::RestsOn => ("rests on".to_string(), EdgeSet::premises(), Direction::Out),
                NamedEdge::Dependents => (
                    "dependents".to_string(),
                    EdgeSet::dependency(),
                    Direction::In,
                ),
                NamedEdge::Lineage => ("lineage".to_string(), EdgeSet::support(), Direction::Both),
                NamedEdge::Mentions => {
                    let kind = RelKind::parse("x.text/mentions")
                        .ok()
                        .or_else(|| RelKind::parse("mentions").ok());
                    match kind.and_then(|k| store.adjacency().edge_kind(&k)) {
                        Some(k) => ("mentions".to_string(), EdgeSet::one(k), Direction::Out),
                        None => {
                            // Not `SMY-E462`: the kind is not undeclared in general, it is absent
                            // from *this* store, which is a different fact and a different fix.
                            diags.push(Diagnostic::at(Code::E511, t.span).with_message(
                            "`expand mentions` needs an `x.text/mentions` relation, and no \
                             record in this store declares or uses one"
                                .to_string(),
                        ));
                            return None;
                        }
                    }
                }
            },
        };
    Some(TraversalPlan {
        label,
        edges,
        direction,
        depth: t.depth.clone(),
        span: t.span,
    })
}

/// The edge set for a relation kind named in a traversal.
///
/// Phase 2 already refused a kind nothing declares with `SMY-E462`, so reaching here with an
/// unknown name means the kind is declared but this store interned no edge for it — an empty
/// set, which walks nowhere and is the right answer rather than an error.
fn edge_set(store: &Store, kind: &str, span: Span, diags: &mut Vec<Diagnostic>) -> Option<EdgeSet> {
    match kind {
        "grounds" => return Some(EdgeSet::one(EdgeKind::Grounds)),
        "deps" => return Some(EdgeSet::one(EdgeKind::Deps)),
        _ => {}
    }
    let parsed = match RelKind::parse(kind) {
        Ok(k) => k,
        Err(_) => {
            diags.push(
                Diagnostic::at(Code::E462, span)
                    .with_message(format!("`{kind}` is not a well-formed relation kind")),
            );
            return None;
        }
    };
    Some(match store.adjacency().edge_kind(&parsed) {
        Some(k) => EdgeSet::one(k),
        None => EdgeSet::of([]),
    })
}

/// Phase 9 for one ordering stage.
fn lower_ordering(o: &Ordering, diags: &mut Vec<Diagnostic>) -> Option<OrderPlan> {
    match o {
        Ordering::OrderBy { keys, .. } => {
            // One key per stage in this build. A multi-key sort is a lexicographic comparator
            // over several typed fields, and the second key only ever breaks a tie in the
            // first — which `order by` already breaks by uid. Refusing the rest by name is
            // better than silently honouring one of them.
            let first = keys.first()?;
            let dir = first.dir.unwrap_or(Dir::Ascending);
            if keys.len() > 1 {
                diags.push(
                    Diagnostic::at(Code::E461, o.span()).with_message(
                        "`order by` with more than one key needs a lexicographic comparator over \
                     typed fields; this build sorts by one key and breaks ties by uid"
                            .to_string(),
                    ),
                );
                return None;
            }
            match &first.target {
                SortTarget::Salience => Some(OrderPlan::Salience { dir }),
                SortTarget::Field(f) => Some(OrderPlan::Field {
                    field: crate::pred::resolve(f, diags)?,
                    dir,
                }),
            }
        }
        Ordering::ReadingOrder { span } => {
            diags.push(
                Diagnostic::at(Code::E461, *span).with_message(
                    "`in reading order` is span order, and a unit in a plain store carries no \
                 span: spans are the library layer's (SMYSL-2.4)"
                        .to_string(),
                ),
            );
            None
        }
        Ordering::TimeOrder { span, .. } => {
            diags.push(
                Diagnostic::at(Code::E461, *span).with_message(
                    "`in time order` reads effective time, which needs the library layer's time \
                 index (SMYSL-2.4)"
                        .to_string(),
                ),
            );
            None
        }
        Ordering::GroupBy { field, .. } => Some(OrderPlan::GroupBy {
            field: crate::pred::resolve(field, diags)?,
        }),
        Ordering::CompareBy { field, .. } => Some(OrderPlan::CompareBy {
            field: crate::pred::resolve(field, diags)?,
        }),
        Ordering::AlignByClass { span, .. } => {
            diags.push(
                Diagnostic::at(Code::E461, *span).with_message(
                    "`align by class` needs the class matrix of SMYSL-2.4's classes, which TX-P7 \
                 proposes and this build cannot read"
                        .to_string(),
                ),
            );
            None
        }
    }
}

/// Phase 9 for one shaping stage.
fn lower_shaping(sh: &Shaping, diags: &mut Vec<Diagnostic>) -> Option<ShapePlan> {
    match sh {
        Shaping::Limit { count, .. } => Some(ShapePlan::Limit {
            count: count.clone(),
        }),
        Shaping::Pack {
            tokens,
            as_view,
            span,
        } => {
            if *as_view {
                // A query that writes. Not refused because packing is hard — it is not — but
                // because emitting a `View` record appends to a store, and this crate is pure:
                // nothing here may write. The CLI is what writes, and the CLI is step 7.
                diags.push(
                    Diagnostic::at(Code::E461, *span).with_message(
                        "`as view` appends a `View` record to the store, and this crate is pure: \
                     writing is the CLI's, which is TX-P6 step 7"
                            .to_string(),
                    ),
                );
                return None;
            }
            Some(ShapePlan::Pack {
                tokens: tokens.clone(),
            })
        }
    }
}

/// Phase 9 for the output stage.
fn lower_output(stage: &OutputStage, diags: &mut Vec<Diagnostic>) -> Option<OutputPlan> {
    match &stage.output {
        Output::Count => Some(OutputPlan::Count),
        Output::Units => Some(OutputPlan::Units),
        Output::Json => Some(OutputPlan::Json),
        Output::Show => Some(OutputPlan::Show),
        Output::Spans => {
            diags.push(
                Diagnostic::at(Code::E461, stage.span).with_message(
                    "`spans` reports where each unit sits in a text, which is the library layer's \
                 (SMYSL-2.4)"
                        .to_string(),
                ),
            );
            None
        }
        Output::Measure(ids) => {
            diags.push(Diagnostic::at(Code::E461, stage.span).with_message(format!(
                "`measure` dispatches to the measures of SMYSL-2.6, which this build does not \
                 carry: {}",
                ids.join(", ")
            )));
            None
        }
    }
}

fn set_name(op: SetOp) -> &'static str {
    match op {
        SetOp::Union => "union",
        SetOp::Intersect => "intersect",
        SetOp::Except => "except",
    }
}

fn order_name(o: &OrderPlan) -> String {
    let dir = |d: &Dir| match d {
        Dir::Ascending => "ascending",
        Dir::Descending => "descending",
    };
    match o {
        OrderPlan::Salience { dir: d } => format!("salience {}", dir(d)),
        OrderPlan::Field { field, dir: d } => format!("{} {}", field.name(), dir(d)),
        OrderPlan::GroupBy { field } => format!("group by {}", field.name()),
        OrderPlan::CompareBy { field } => format!("compare by {}", field.name()),
    }
}

fn shape_name(sh: &ShapePlan) -> String {
    let count = |c: &Count| match c {
        Count::Int(n) => n.to_string(),
        Count::Param(p) => format!("${p}"),
    };
    match sh {
        ShapePlan::Limit { count: c } => format!("limit {}", count(c)),
        ShapePlan::Pack { tokens } => format!("pack {} tokens", count(tokens)),
    }
}

fn output_name(o: OutputPlan) -> &'static str {
    match o {
        OutputPlan::Count => "count",
        OutputPlan::Units => "units",
        OutputPlan::Json => "json",
        OutputPlan::Show => "show",
    }
}

/// Phase 5. One source lowers; the rest are refused by name, with what each is waiting for.
///
/// `~sem` and `~hybrid` are not "later phases" — they are a different tier. A semantic source
/// consults a model, which is what SMYSL-2.5 §4.1 keeps behind the `SemanticSource` port so
/// this crate stays pure, and `~hybrid` routes between the two and so needs both. Saying so is
/// the point: `SMY-E461`'s meaning is "not available in this build", and a reader told *which
/// build would have it* can go and get one.
fn lower_source(stage: &crate::ast::SourceStage, diags: &mut Vec<Diagnostic>) -> Option<Source> {
    let span = stage.span;
    match &stage.source {
        AstSource::Find { text, engine, lang } => {
            // A language selector is the library layer's: `lang:<code>` asks which language a
            // unit is in, and nothing in a plain store records that — the catalog does
            // (SMYSL-2.4), which is step 6. `lang:any` asks for more still, a bridge per pair
            // of partitions (§3.5), which is TX-P10's. Refused rather than ignored, because a
            // query that asked for one language and silently got all of them is answered
            // wrongly and says nothing about it.
            if let Some(sel) = lang {
                let what = match sel {
                    LangSel::Any => "`lang:any` needs a language bridge per pair of partitions",
                    LangSel::One(_) => "`lang:<code>` needs the catalog's language facts",
                };
                diags.push(Diagnostic::at(Code::E461, span).with_message(format!(
                    "{what}, which this build has no source for: it ranks lexically over a \
                     plain store"
                )));
                return None;
            }
            match engine {
                Some(Engine::Sem) => {
                    diags.push(
                        Diagnostic::at(Code::E461, span).with_message(
                            "`~sem` consults an embedding model, which this pure build has no \
                         port bound for"
                                .to_string(),
                        ),
                    );
                    None
                }
                Some(Engine::Hybrid) => {
                    diags.push(
                        Diagnostic::at(Code::E461, span).with_message(
                            "`~hybrid` routes between the lexical and semantic engines, so it \
                         needs the semantic one this pure build has no port bound for"
                                .to_string(),
                        ),
                    );
                    None
                }
                // SMYSL-2.9 OQ-83: an absent engine is `~lex`.
                engine => Some(Source::Lexical(Lexical {
                    text: text.clone(),
                    explicit: engine.is_some(),
                    span,
                })),
            }
        }
        other => {
            let what = match other {
                AstSource::AboutEntity(_) => "`about entity(…)` needs entity resolution",
                AstSource::AboutClass(_) => "`about class(…)` needs class membership",
                AstSource::Text { .. } => "`text(…)` needs the library layer's locators",
                AstSource::Uid(_) => {
                    "`uid` is phase 5 for a store this build can reach, and \
                                      is not lowered yet"
                }
                AstSource::View(_) => "`view` needs a saved view",
                AstSource::Pointer(_) => "`pointer` needs the pointer registry",
                AstSource::Metric(_) => "`metric` dispatches to the measures of SMYSL-2.6",
                AstSource::Question(_) => "`question` is the interpretive source of OQ-47",
                AstSource::Match { .. } => "`match` is refused by SMYSL-2.5 §3.3 in this RFC",
                AstSource::Find { .. } => unreachable!("handled above"),
            };
            diags.push(
                Diagnostic::at(Code::E461, span)
                    .with_message(format!("{what}, which this build does not have")),
            );
            None
        }
    }
}

/// Phase 2 for relation kinds: the fourteen kernel kinds, plus whatever the declarations in
/// scope add. An undeclared kind is `SMY-E462`.
fn resolve_relations(store: &Store, p: &Pipeline, diags: &mut Vec<Diagnostic>) -> Vec<String> {
    let mut known: BTreeSet<String> = RelKind::KERNEL.iter().map(|k| k.to_string()).collect();
    for record in store.iter() {
        if let Record::SchemaDecl(d) = record {
            for kind in &d.relations {
                known.insert(kind.to_string());
            }
        }
    }

    let mut used = Vec::new();
    for t in &p.traversals {
        let kind = match &t.edge {
            // `grounds` and `deps` are spellable as arrow traversals (`expand ->grounds`, §3.3)
            // and are **not** relation kinds: they are the two kernel support fields, so no
            // schema declares them and `SMY-E462` would be wrong. TX-P6 step 4 found this by
            // trying to walk one — until traversals were lowered, nothing reached the walk, so
            // the refusal was invisible.
            EdgeSpec::Out(k) | EdgeSpec::In(k) | EdgeSpec::Both(k)
                if k == "grounds" || k == "deps" =>
            {
                continue
            }
            EdgeSpec::Out(k) | EdgeSpec::In(k) | EdgeSpec::Both(k) => k,
            // The spelled traversals are graph operations rather than relation kinds, so
            // there is nothing here to declare (SMYSL-2.9 §4.7).
            EdgeSpec::Named(_) => continue,
        };
        if !known.contains(kind) {
            diags.push(Diagnostic::at(Code::E462, t.span).with_message(format!(
                "`{kind}` is not a relation kind: the kernel does not define it and no schema \
                 in scope declares it"
            )));
        } else if !used.contains(kind) {
            used.push(kind.clone());
        }
    }
    used
}

/// Whether the store holds a declaration of this id at this revision.
///
/// `Store::schema_decls` is TX-P12 step 1's; until then the log is the index, which for a
/// handful of declarations is the same answer at a cost nobody will notice.
fn declares(store: &Store, id: &str, revision: u32) -> bool {
    store.iter().any(|r| match r {
        Record::SchemaDecl(d) => d.id.as_str() == id && d.version == revision,
        _ => false,
    })
}
