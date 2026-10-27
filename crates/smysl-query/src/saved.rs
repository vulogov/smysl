//! Saved queries and reruns (SMYSL-2.5 §3.6, §4.1 `src/saved.rs`; TX-P8 step 1).
//!
//! A saved query is one `x.query/saved` unit whose payload carries the seven keys of
//! SMYSL-2.3 Appendix C, linked to each unit of its result by an `x.query/answers` edge. Two
//! rules from §3.6 shape everything here.
//!
//! **A query is saved only after it has planned and run to completion** (OQ-82). Three of the
//! seven keys are facts about a run — the result hash, the revisions in force, the lens chains
//! applied — and the result edges are the result. So [`to_records`] takes a [`Plan`] and a
//! [`Run`] and nothing else can produce one; there is no "save this text" that bypasses it.
//!
//! **The result edge is inert for salience** (OQ-49). Draft 3 linked results with kernel
//! `answers`, which carries support, so saving a query would have raised its results'
//! salience and changed every later `order by salience` — including the rerun of the query
//! that was saved. [`RESULT_EDGE`] is `x.query/answers`, an extension kind
//! `EdgeKind::carries_support` treats as non-supporting, and `tests/saved.rs` asserts that
//! saving changes no result's salience rather than taking the kind's word for it.
//!
//! # What a rerun is
//!
//! Let `P` be the result the edges record and `E_pin` the same text run again, pinned to the
//! revisions the unit recorded, over today's data. `data_effect = diff(P, E_pin)`: what the
//! store's growth changed. `E_cur` is the text run under current revisions, every pin
//! dropped, and the **schema effect** is `diff(E_pin, E_cur)`: what the declarations' growth
//! changed (TX-P12 step 4). Pinned means the revisions the unit recorded in `query:schemas` —
//! the query's own `using` pins and the current revision of every schema an unpinned field
//! resolved against when it was saved — so a query saved before a revision was declared is
//! compiled forward from the revision it was asked under (§3.9), and its rerun keeps that
//! question while the data changes under it.
//!
//! # The recipe
//!
//! §3.6 lists the recipe's inputs: `sq` version, analyzer ids, IDF scope, estimator id, RRF
//! `k`, class policy, template-set id, parameters. Two of them have nothing to record in this
//! build — no class policy enters a plan before TX-P7's classes do, and no template set
//! before step 3 — and they are hashed as empty rather than left out, so the framing does not
//! change when they arrive. Step 3's `ask` sets the template set for every slot it plans
//! ([`PlanOptions::template_set`]); a query a person wrote still hashes it as empty.
//! One input the list omits is the **per-partition limit** a source
//! keeps before fusion ([`PlanOptions::limit`]), which changes a result as surely as `k` does;
//! it is hashed here and raised as OQ-84, because the payload has no declared key to carry it
//! and a key the schema does not declare is a key a reader cannot trust.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::hash::Rolling;
use smysl_core::ids::Uid;
use smysl_core::schemas::query as schema;
use smysl_core::surface::{object_to_payload, payload_to_object, HObject, HValue, Spanned};
use smysl_core::types::{RelKind, Relation, SourceKind, SourceRef};
use smysl_core::{SchemaId, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_retrieve::{Analyze, Tokenizer};

use crate::ast::{Atom, Count, Expr, Pipeline, Pred, Query, Shaping, Source, Value};
use crate::exec::Run;
use crate::plan::{Env, Plan, PlanExpr, PlanOptions};
use crate::ports::IdfScope;

/// The relation kind a saved query's results hang from: `x.query/answers`, never kernel
/// `answers` (SMYSL-2.5 §3.6, OQ-49).
pub const RESULT_EDGE: &str = schema::ANSWERS;

/// The framing of a result hash (§3.6: *"tool-level, not format"*).
const RESULT_FRAME: &[u8] = b"smysl/sq-result/1\0";

/// The framing of a recipe hash. Tool-level as well: a recipe is this planner's description of
/// how it would answer, and another implementation of `sq/1` may describe itself differently.
const RECIPE_FRAME: &[u8] = b"smysl/sq-recipe/1\0";

/// What went into a plan besides the query (SMYSL-2.5 §3.6, the `query:recipe` key).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Recipe {
    pub sq: u32,
    /// Sorted, distinct, one per analyzer a partition was searched under.
    pub analyzers: Vec<String>,
    pub idf_scope: IdfScope,
    /// The packer's estimator id when the plan packs, else empty.
    pub estimator: String,
    pub rrf_k: f64,
    /// Hits kept per partition before fusion. See the module header and OQ-84.
    pub per_partition: usize,
    /// Empty until TX-P7's classes enter a plan.
    pub class_policy: String,
    /// Empty until TX-P8 step 3's templates do.
    pub template_set: String,
    /// Sorted by name.
    pub params: Vec<(String, String)>,
}

impl Recipe {
    /// The recipe of a plan under these options.
    pub fn of(plan: &Plan, opts: &PlanOptions) -> Recipe {
        let mut analyzers: BTreeSet<String> = BTreeSet::new();
        collect_analyzers(&plan.expr, &mut analyzers);
        let packs = plan
            .shaping
            .iter()
            .any(|s| matches!(s, crate::plan::ShapePlan::Pack { .. }));
        Recipe {
            sq: plan.sq,
            analyzers: analyzers.into_iter().collect(),
            idf_scope: opts.idf_scope,
            estimator: if packs {
                smysl_pack::DEFAULT_ESTIMATOR.to_string()
            } else {
                String::new()
            },
            rrf_k: crate::fuse::RRF_K,
            per_partition: opts.limit,
            class_policy: String::new(),
            template_set: opts.template_set.clone(),
            params: opts
                .params
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }

    /// BLAKE3 over the framed inputs, each terminated by `\0` so no two sequences of inputs
    /// concatenate to the same bytes.
    pub fn hash(&self) -> [u8; 32] {
        let mut h = Rolling::new();
        h.update(RECIPE_FRAME);
        let mut put = |s: &str| {
            h.update(s.as_bytes());
            h.update(b"\0");
        };
        put(&format!("sq/{}", self.sq));
        put(&self.analyzers.join(","));
        put(self.idf_scope.as_str());
        put(&self.estimator);
        put(&format!("{}", self.rrf_k));
        put(&format!("{}", self.per_partition));
        put(&self.class_policy);
        put(&self.template_set);
        for (k, v) in &self.params {
            put(&format!("{k}={v}"));
        }
        h.finish()
    }
}

fn collect_analyzers(expr: &PlanExpr, into: &mut BTreeSet<String>) {
    match expr {
        PlanExpr::Pipeline(p) => {
            for partition in &p.partitions {
                into.insert(
                    partition
                        .chain
                        .clone()
                        .unwrap_or_else(|| Tokenizer::plain().id().to_string()),
                );
            }
        }
        PlanExpr::Set { left, right, .. } => {
            collect_analyzers(left, into);
            collect_analyzers(right, into);
        }
    }
}

/// `query:result-hash`: BLAKE3 over the frame and the sorted result uids, each in canonical
/// text and newline-terminated.
pub fn result_hash(uids: &[Uid]) -> [u8; 32] {
    let sorted: BTreeSet<&Uid> = uids.iter().collect();
    let mut h = Rolling::new();
    h.update(RESULT_FRAME);
    for uid in sorted {
        h.update(uid.canonical().as_bytes());
        h.update(b"\n");
    }
    h.finish()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bind every `$parameter` in a query from `params`, so the text saved is *"the query as
/// written, after parameter binding"* (§3.6). An unbound one is `SMY-E460` at the span of the
/// stage that names it, which is the executor's rule as well.
pub fn bind(query: &Query, params: &BTreeMap<String, String>) -> Result<Query, Diagnostic> {
    let mut q = query.clone();
    q.expr = bind_expr(&q.expr, params)?;
    for stage in &mut q.shaping {
        match stage {
            Shaping::Limit { count, span }
            | Shaping::Pack {
                tokens: count,
                span,
                ..
            } => {
                *count = bind_count(count, params, *span)?;
            }
        }
    }
    Ok(q)
}

fn bind_expr(expr: &Expr, params: &BTreeMap<String, String>) -> Result<Expr, Diagnostic> {
    Ok(match expr {
        Expr::Pipeline(p) => Expr::Pipeline(bind_pipeline(p, params)?),
        Expr::Set {
            op,
            left,
            right,
            span,
        } => Expr::Set {
            op: *op,
            left: Box::new(bind_expr(left, params)?),
            right: Box::new(bind_expr(right, params)?),
            span: *span,
        },
    })
}

fn bind_pipeline(p: &Pipeline, params: &BTreeMap<String, String>) -> Result<Pipeline, Diagnostic> {
    let mut out = p.clone();
    if let Some(stage) = &mut out.source {
        let span = stage.span;
        match &mut stage.source {
            Source::Find { text, .. }
            | Source::AboutEntity(text)
            | Source::Uid(text)
            | Source::Question(text) => *text = bind_atom(text, params, span)?,
            Source::AboutClass(_)
            | Source::Text { .. }
            | Source::View(_)
            | Source::Pointer(_)
            | Source::Metric(_)
            | Source::Match { .. } => {}
        }
    }
    for f in &mut out.filters {
        f.pred = bind_pred(&f.pred, params, f.span)?;
    }
    for t in &mut out.traversals {
        if let Some(depth) = &t.depth {
            t.depth = Some(bind_count(depth, params, t.span)?);
        }
    }
    Ok(out)
}

fn bind_pred(
    pred: &Pred,
    params: &BTreeMap<String, String>,
    span: Span,
) -> Result<Pred, Diagnostic> {
    Ok(match pred {
        Pred::And(items) => Pred::And(
            items
                .iter()
                .map(|p| bind_pred(p, params, span))
                .collect::<Result<_, _>>()?,
        ),
        Pred::Or(items) => Pred::Or(
            items
                .iter()
                .map(|p| bind_pred(p, params, span))
                .collect::<Result<_, _>>()?,
        ),
        Pred::Not(inner) => Pred::Not(Box::new(bind_pred(inner, params, span)?)),
        Pred::Cmp { field, op, value } => Pred::Cmp {
            field: field.clone(),
            op: *op,
            value: bind_value(value, params, span)?,
        },
        Pred::Matches(atom) => Pred::Matches(bind_atom(atom, params, span)?),
        Pred::Near { text, threshold } => Pred::Near {
            text: bind_atom(text, params, span)?,
            threshold: *threshold,
        },
    })
}

fn bind_value(
    v: &Value,
    params: &BTreeMap<String, String>,
    span: Span,
) -> Result<Value, Diagnostic> {
    Ok(match v {
        Value::Param(name) => Value::Str(lookup(name, params, span)?),
        Value::List(items) => Value::List(
            items
                .iter()
                .map(|v| bind_value(v, params, span))
                .collect::<Result<_, _>>()?,
        ),
        other => other.clone(),
    })
}

fn bind_atom(a: &Atom, params: &BTreeMap<String, String>, span: Span) -> Result<Atom, Diagnostic> {
    Ok(match a {
        Atom::Param(name) => Atom::Str(lookup(name, params, span)?),
        Atom::Str(s) => Atom::Str(s.clone()),
    })
}

fn bind_count(
    c: &Count,
    params: &BTreeMap<String, String>,
    span: Span,
) -> Result<Count, Diagnostic> {
    Ok(match c {
        Count::Param(name) => {
            let bound = lookup(name, params, span)?;
            Count::Int(bound.parse::<i64>().map_err(|_| {
                Diagnostic::at(Code::E460, span).with_message(format!(
                    "`${name}` is bound to `{bound}`, which is not a count"
                ))
            })?)
        }
        Count::Int(n) => Count::Int(*n),
    })
}

fn lookup(name: &str, params: &BTreeMap<String, String>, span: Span) -> Result<String, Diagnostic> {
    params.get(name).cloned().ok_or_else(|| {
        Diagnostic::at(Code::E460, span).with_message(format!(
            "`${name}` is not bound, so the query cannot be saved as written"
        ))
    })
}

/// The records a save writes: the `x.query/saved` unit and one [`RESULT_EDGE`] per result.
///
/// `query` is the query the plan was lowered from — bound first, by [`bind`], because the text
/// saved is the text after binding. The unit's gist is that text in canonical form, its source
/// cites it (`sq:<recipe hash, short>`, status `cited`, as `@question` units are authored in
/// `fixtures/corpus/F4-qa.smy`), and its payload carries the seven keys. A query that did not
/// run to completion never reaches here: the [`Run`] is the proof.
pub fn to_records(
    query: &Query,
    plan: &Plan,
    run: &Run,
    opts: &PlanOptions,
) -> Result<(UnitCore, Vec<Relation>), Diagnostic> {
    let text = crate::print(query);
    let recipe = Recipe::of(plan, opts);
    let recipe_hex = hex(&recipe.hash());
    let results = run.answer.uids();
    let result_hex = hex(&result_hash(&results));

    let span = Span::new(0, 0);
    let s = |v: &str| Spanned::new(HValue::Str(v.to_string()), span);
    let list = |items: Vec<String>| {
        Spanned::new(
            HValue::Array(
                items
                    .into_iter()
                    .map(|v| Spanned::new(HValue::Str(v), span))
                    .collect(),
            ),
            span,
        )
    };
    let key = |k: &str| Spanned::new(k.to_string(), span);
    let mut payload = HObject::new();
    payload.insert(key("query:text"), s(&text));
    payload.insert(key("query:recipe"), s(&recipe_hex));
    // Empty for a pure plan (§3.6); the model's hash for one that consults a model, and the
    // translator's for a query a model wrote (§3.15).
    let mut models = plan.models.clone();
    if let Some(t) = &opts.translator {
        if !models.contains(t) {
            models.push(t.clone());
        }
    }
    payload.insert(key("query:models"), list(models));
    payload.insert(key("query:result-hash"), s(&result_hex));
    payload.insert(key("query:sq"), s(&format!("sq/{}", plan.sq)));
    payload.insert(
        key("query:schemas"),
        list(
            plan.revisions
                .iter()
                .map(|p| format!("{}@{}", p.id, p.revision))
                .collect(),
        ),
    );
    // The chains a pin compiled through, `id@from->to:hash`, space-separated (§3.6, §3.10).
    // No chain applied is the empty string rather than a hash of an empty list, so a reader
    // cannot mistake "no chains" for "chains whose hash happens to be this".
    payload.insert(key("query:lens-chain"), s(&plan.lens_chains.join(" ")));
    let bytes = object_to_payload(&payload).expect("seven keys are not an empty object");

    let schema = SchemaId::Extension(schema::SAVED.to_string());
    let source = SourceRef::new(SourceKind::Doc, format!("sq:{}", &recipe_hex[..12]));
    let mut builder = UnitCoreBuilder::new(schema, text, Status::Cited)
        .source(source)
        .payload(bytes);
    if let Some(q) = &opts.question {
        builder = builder.body(q.clone());
    }
    let unit = builder.build().map_err(|e| {
        Diagnostic::at(Code::E460, span)
            .with_message(format!("the saved query could not be built: {e}"))
    })?;
    let from = smysl_core::canonical_uid(&unit);
    let kind = RelKind::Extension(RESULT_EDGE.to_string());
    let edges = results
        .iter()
        .map(|to| Relation::new(kind.clone(), from, *to))
        .collect();
    Ok((unit, edges))
}

/// A saved query, read back from a store.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SavedQuery {
    pub uid: Uid,
    pub text: String,
    pub recipe: String,
    pub models: Vec<String>,
    pub result_hash: String,
    pub sq: String,
    pub schemas: Vec<String>,
    pub lens_chain: String,
    /// The result the edges record, in uid order.
    pub results: Vec<Uid>,
}

/// Why a uid did not load as a saved query. No `SMY-` code, because none of these is a fact
/// about a query: the first two are about a uid and the third about a record somebody else
/// wrote, and the CLI reports them in words.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadError {
    NotInStore,
    NotASavedQuery(String),
    Malformed(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::NotInStore => write!(f, "no unit with that uid is in the store"),
            LoadError::NotASavedQuery(schema) => {
                write!(f, "that unit is `{schema}`, not `{}`", schema::SAVED)
            }
            LoadError::Malformed(why) => write!(f, "the saved query's payload is malformed: {why}"),
        }
    }
}

impl std::error::Error for LoadError {}

/// Read a saved query and the result its edges record.
pub fn load(store: &Store, uid: &Uid) -> Result<SavedQuery, LoadError> {
    let unit = store.get(uid).ok_or(LoadError::NotInStore)?;
    if unit.core.schema.as_str() != schema::SAVED {
        return Err(LoadError::NotASavedQuery(
            unit.core.schema.as_str().to_string(),
        ));
    }
    let bytes = unit
        .core
        .payload
        .as_deref()
        .ok_or_else(|| LoadError::Malformed("no payload".to_string()))?;
    let object = payload_to_object(bytes).map_err(|e| LoadError::Malformed(e.to_string()))?;
    let text = |key: &str| -> Result<String, LoadError> {
        match object.get(key).map(|v| &v.value) {
            Some(HValue::Str(s)) => Ok(s.clone()),
            Some(other) => Err(LoadError::Malformed(format!(
                "`{key}` is {}, not text",
                other.type_name()
            ))),
            None => Err(LoadError::Malformed(format!("`{key}` is missing"))),
        }
    };
    let list = |key: &str| -> Result<Vec<String>, LoadError> {
        match object.get(key).map(|v| &v.value) {
            Some(HValue::Array(items)) => items
                .iter()
                .map(|i| match &i.value {
                    HValue::Str(s) => Ok(s.clone()),
                    other => Err(LoadError::Malformed(format!(
                        "`{key}` holds {}, not text",
                        other.type_name()
                    ))),
                })
                .collect(),
            Some(other) => Err(LoadError::Malformed(format!(
                "`{key}` is {}, not a list",
                other.type_name()
            ))),
            None => Err(LoadError::Malformed(format!("`{key}` is missing"))),
        }
    };
    let kind = RelKind::Extension(RESULT_EDGE.to_string());
    let results: BTreeSet<Uid> = store
        .relations_of_kind(&kind)
        .into_iter()
        .filter(|r| r.from == *uid)
        .map(|r| r.to)
        .collect();
    Ok(SavedQuery {
        uid: *uid,
        text: text("query:text")?,
        recipe: text("query:recipe")?,
        models: list("query:models")?,
        result_hash: text("query:result-hash")?,
        sq: text("query:sq")?,
        schemas: list("query:schemas")?,
        lens_chain: text("query:lens-chain")?,
        results: results.into_iter().collect(),
    })
}

/// What changed between two result sets, in uid order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SetDiff {
    pub added: Vec<Uid>,
    pub removed: Vec<Uid>,
}

impl SetDiff {
    pub fn between(before: &[Uid], after: &[Uid]) -> SetDiff {
        let a: BTreeSet<&Uid> = before.iter().collect();
        let b: BTreeSet<&Uid> = after.iter().collect();
        SetDiff {
            added: b.difference(&a).map(|u| **u).collect(),
            removed: a.difference(&b).map(|u| **u).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// A saved query, run again (SMYSL-2.5 §3.6).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Rerun {
    pub plan: Plan,
    pub run: Run,
    /// `diff(P, E_pin)`: what the data changed.
    pub data_effect: SetDiff,
    /// `diff(E_pin, E_cur)`, for a `--current` rerun: what the declarations' growth changed.
    /// `None` for a pinned rerun, which does not ask.
    pub schema_effect: Option<SetDiff>,
    /// Whether this run's recipe is the one recorded. A rerun under other options — another
    /// IDF scope, another per-partition limit — is a different question, and its data effect
    /// is reported but labelled.
    pub recipe_matches: bool,
}

/// Whether a recorded `id@from->to:hash` chain is built, to that hash, by the store's
/// declarations.
fn reproduces(env: &Env<'_>, chain: &str) -> bool {
    let parse = || -> Option<(smysl_core::ids::SchemaId, u32, u32, String)> {
        let (head, hash) = chain.rsplit_once(':')?;
        let (id, span) = head.rsplit_once('@')?;
        let (from, to) = span.split_once("->")?;
        Some((
            SchemaId::parse(id).ok()?,
            from.parse().ok()?,
            to.parse().ok()?,
            hash.to_string(),
        ))
    };
    let Some((id, from, to, hash)) = parse() else {
        return false;
    };
    env.reader
        .schemas()
        .set
        .chain(&id, from, to)
        .is_ok_and(|c| hex(&c.hash) == hash)
}

/// Run a saved query again, pinned — and, with `current`, under current revisions too.
///
/// Pinned means the saved text with every revision the unit recorded (`query:schemas`) as a
/// `using` pin, planned over today's store: a recorded revision the store no longer declares is
/// `SMY-E466`, which the planner raises for the pin, and so is a recorded lens-chain hash the
/// declarations no longer reproduce (§3.6), because the query would then answer from part of
/// its meaning. `current` plans the text again with every schema pin dropped, returns that
/// plan and run, and reports `diff(E_pin, E_cur)` as the schema effect.
/// `SMY-W467` (§3.6): a saved query that recorded a model, rerun under another. The rerun still
/// answers — a model-dependent query was always going to — and the diffs are printed, but
/// labelled: a unit that came or went may be the model's doing rather than the data's.
fn model_changed(saved: &SavedQuery, plan: &Plan) -> Option<Diagnostic> {
    // Only the models the plan consults: a saved query a model *wrote* records its translator
    // too (§3.15), and a pure rerun of it consults nothing that could have changed.
    if plan.models.iter().all(|m| saved.models.contains(m)) {
        return None;
    }
    let short = |v: &[String]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.iter()
                .map(|h| h[..h.len().min(12)].to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    Some(Diagnostic::new(Code::W467).with_message(format!(
        "saved under model {} and rerun under {}; the data effect is not comparable",
        short(&saved.models),
        short(&plan.models)
    )))
}

pub fn rerun(
    saved: &SavedQuery,
    env: &Env<'_>,
    opts: &PlanOptions,
    current: bool,
) -> Result<Rerun, Vec<Diagnostic>> {
    let parsed = crate::parse(&saved.text).map_err(|d| vec![d])?;
    // The record of the save is not data the query found. A saved query's gist *is* its
    // text, so a lexical source over the same store ranks the saved unit for its own terms
    // — `find "connection pool"` finds the unit that says `find "connection pool"`, at rank
    // 1 — and a data effect that counted it would report the save as growth. Taken out of
    // scope before anything scores, not out of the answer after, because a unit removed
    // after fusion leaves every other hit one rank lower than the save found it. Found by
    // §5.4's first test and recorded as OQ-85, since a fresh `find` has the same unit in
    // reach. The exclusion is the rerun's and not the recipe's: the recipe is compared
    // under the caller's options, which is what was saved.
    let mut pinned = opts.clone();
    pinned.exclude.insert(saved.uid);
    let mut at_pin = parsed.query.clone();
    for r in &saved.schemas {
        let Some((id, rev)) = r.rsplit_once('@') else {
            continue;
        };
        let Ok(revision) = rev.parse::<u32>() else {
            continue;
        };
        let already = at_pin
            .header
            .iter()
            .any(|u| matches!(u, crate::ast::Using::Schema { id: i, .. } if i == id));
        if !already {
            at_pin.header.push(crate::ast::Using::Schema {
                id: id.to_string(),
                revision,
                span: Span::new(0, 0),
            });
        }
    }
    let plan = crate::plan(&at_pin, env, &pinned)?;
    // Every chain the save compiled through must still be buildable from this store's
    // declarations, to the same hash: a declaration that was never merged here leaves the
    // query answering from part of its meaning (§3.6).
    for chain in saved.lens_chain.split_whitespace() {
        if !reproduces(env, chain) {
            return Err(vec![Diagnostic::at(Code::E466, Span::new(0, 0))
                .with_message(format!(
                    "the saved query was compiled through `{chain}`, which this store's \
                     declarations do not reproduce, so it cannot be pinned"
                ))]);
        }
    }
    let mut run = crate::run(&plan, env, &pinned)?;
    let incomparable = model_changed(saved, &plan);
    if let Some(w) = &incomparable {
        run.warnings.push(w.clone());
    }
    let now = run.answer.uids();
    let recipe_matches = hex(&Recipe::of(&plan, opts).hash()) == saved.recipe;
    let data_effect = SetDiff::between(&saved.results, &now);
    if !current {
        return Ok(Rerun {
            data_effect,
            schema_effect: None,
            recipe_matches,
            plan,
            run,
        });
    }
    let mut at_cur = parsed.query.clone();
    at_cur
        .header
        .retain(|u| !matches!(u, crate::ast::Using::Schema { .. }));
    let cur_plan = crate::plan(&at_cur, env, &pinned)?;
    let mut cur_run = crate::run(&cur_plan, env, &pinned)?;
    if let Some(w) = incomparable {
        cur_run.warnings.push(w);
    }
    let schema_effect = SetDiff::between(&now, &cur_run.answer.uids());
    Ok(Rerun {
        data_effect,
        schema_effect: Some(schema_effect),
        recipe_matches,
        plan: cur_plan,
        run: cur_run,
    })
}
