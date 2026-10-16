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

use std::collections::BTreeSet;

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::Uid;
use smysl_core::types::{Record, RelKind};
use smysl_graph::Store;

use crate::ast::{EdgeSpec, Expr, Pipeline, Query, Using};
use crate::ports::{item_text, Candidates, IdfScope, Partition, Scope};

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
///
/// `params` is absent until the executor can bind one: SMYSL-2.9 §4.10 refuses an unbound
/// `$parameter` at execution, where the bindings are, and this build does not execute.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct PlanOptions {
    pub idf_scope: IdfScope,
    pub all_versions: bool,
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

/// A schema pinned by a `using` clause, with the revision it resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub id: String,
    pub revision: u32,
}

/// What one phase did. `--explain` prints these in order (SMYSL-2.5 §3.2: each phase "records
/// what it did, so `--explain` prints the plan rather than reconstructing it").
#[derive(Debug, Clone, PartialEq, Eq)]
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
}

/// A lowered query (SMYSL-2.5 §4.2).
///
/// `recipe_hash` is **not** here. SMYSL-2.5 §3.6 defines the recipe over eight inputs — sq
/// version, analyzer ids, IDF scope, estimator id, RRF `k`, class policy, template-set id,
/// parameters — of which this build has two. A hash over a subset would be a saved query's
/// identity changing silently when the other six arrive, which is the one thing a recipe hash
/// exists to prevent. It lands with TX-P8, where saving does.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plan {
    pub sq: u32,
    pub purity: Purity,
    pub steps: Vec<PlanStep>,
    pub pins: Vec<Pin>,
    pub partitions: Vec<Partition>,
    pub candidates: BTreeSet<Uid>,
}

/// Lower a query through phases 1 to 4.
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

    // A set expression is phase 8. Refusing here rather than at the first pipeline keeps the
    // message about the operator the reader wrote.
    let pipeline = match &q.expr {
        Expr::Pipeline(p) => p,
        Expr::Set { span, .. } => {
            diags.push(not_yet(*span, "a set operation", "phase 8"));
            return Err(diags);
        }
    };

    unlowered(q, pipeline, &mut diags);

    // --- phase 2: resolution -----------------------------------------------------------------
    let relations = resolve_relations(env.store, pipeline, &mut diags);

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

    if !diags.is_empty() {
        return Err(diags);
    }

    // --- phase 4: partitions -----------------------------------------------------------------
    let partitions = env.scope.partition(&candidates.uids);

    let steps = vec![
        PlanStep::Header {
            sq,
            pins: pins.clone(),
        },
        PlanStep::Resolve {
            relations,
            fields_checked: false,
        },
        PlanStep::Scope {
            scope: env.scope.name().to_string(),
            items,
            stores: candidates.stores.clone(),
            candidates: candidates.uids.len(),
        },
        PlanStep::Partition {
            partitions: partitions
                .iter()
                .map(|p| (crate::explain::partition_name(&p.key), p.uids.len()))
                .collect(),
            idf_scope: opts.idf_scope,
        },
    ];

    Ok(Plan {
        sq,
        purity: Purity::Pure,
        steps,
        pins,
        partitions,
        candidates: candidates.uids,
    })
}

/// `SMY-E461`, with the phase that will lower the stage named.
///
/// Naming the phase is the difference between "this build will not do that" and "this build
/// does not do that yet", and only the second is true.
fn not_yet(span: Span, what: &str, phase: &str) -> Diagnostic {
    Diagnostic::at(Code::E461, span).with_message(format!(
        "{what} is {phase} of the planner, which this build does not reach: it plans the \
         header, resolution, scope and partitions"
    ))
}

/// Every stage phases 5 to 9 own, refused by name.
fn unlowered(q: &Query, p: &Pipeline, diags: &mut Vec<Diagnostic>) {
    if let Some(source) = &p.source {
        diags.push(not_yet(source.span, "a source", "phase 5"));
    }
    for f in &p.filters {
        diags.push(not_yet(f.span, "a `where` filter", "phase 6"));
    }
    for t in &p.times {
        diags.push(not_yet(t.span, "a time stage", "phase 6"));
    }
    for t in &p.traversals {
        diags.push(not_yet(t.span, "an `expand` traversal", "phase 7"));
    }
    for c in &p.closures {
        diags.push(not_yet(c.span, "a `with` closure", "phase 7"));
    }
    for o in &q.ordering {
        diags.push(not_yet(o.span(), "an ordering stage", "phase 9"));
    }
    for s in &q.shaping {
        diags.push(not_yet(s.span(), "a shaping stage", "phase 9"));
    }
    if let Some(out) = &q.output {
        diags.push(not_yet(out.span, "an output stage", "phase 9"));
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
