//! `--explain` (SMYSL-2.5 §3.2, §4.2).
//!
//! §3.2's requirement is the whole design: each phase "records what it did, so `--explain`
//! prints the plan rather than reconstructing it". So this file is a formatter over
//! [`PlanStep`] and holds no planning logic — if a line here had to work something out, the
//! phase that should have recorded it did not.
//!
//! The output is a plain text block, one line per phase, stable enough to be a golden. That
//! is the exit TX-P6 step 2 asks for: plan snapshots over `fixtures/corpus`.

use crate::fuse::Fused;
use crate::plan::{Plan, PlanStep, Purity};
use crate::ports::PartitionKey;

/// How a partition is named in a plan and in an explain line.
pub fn partition_name(key: &PartitionKey) -> String {
    match key {
        PartitionKey::Uncatalogued => "(no manifest)".to_string(),
        PartitionKey::Catalog { manifest, lang } => format!("{manifest} [{lang}]"),
    }
}

/// The plan as `--explain` prints it.
pub fn explain(plan: &Plan) -> String {
    let mut out = String::new();
    out.push_str(&format!("plan sq/{}\n", plan.sq));
    out.push_str(&format!(
        "  purity: {}\n",
        match &plan.purity {
            Purity::Pure => "pure".to_string(),
            Purity::ModelDependent(why) => format!("model-dependent ({})", why.join(", ")),
        }
    ));

    for step in &plan.steps {
        match step {
            PlanStep::Header { sq, pins } => {
                out.push_str(&format!("  1 header: sq/{sq}"));
                if pins.is_empty() {
                    out.push_str(", no schema pins\n");
                } else {
                    let names: Vec<String> = pins
                        .iter()
                        .map(|p| format!("{}@{}", p.id, p.revision))
                        .collect();
                    out.push_str(&format!(", pinned {}\n", names.join(", ")));
                }
            }
            PlanStep::Resolve {
                relations,
                fields_checked,
            } => {
                out.push_str("  2 resolution: ");
                if relations.is_empty() {
                    out.push_str("no relation kinds named");
                } else {
                    out.push_str(&format!("relations {}", relations.join(", ")));
                }
                // Said out loud rather than left to be assumed. A reader who knows the phase
                // exists would otherwise read silence as "every field checked out".
                if !fields_checked {
                    out.push_str("; fields not checked (payload shapes are opaque until FC-9)");
                }
                out.push('\n');
            }
            PlanStep::Scope {
                scope,
                items,
                stores,
                candidates,
            } => {
                let asked = if items.is_empty() {
                    "everything (no `from`)".to_string()
                } else {
                    items.join(", ")
                };
                out.push_str(&format!(
                    "  3 scope: {asked} via {scope} -> {} store(s) {}, {candidates} unit(s)\n",
                    stores.len(),
                    stores.join(", ")
                ));
            }
            PlanStep::Partition {
                partitions,
                idf_scope,
            } => {
                out.push_str(&format!(
                    "  4 partitions: {}, idf over {}\n",
                    partitions.len(),
                    idf_scope.as_str()
                ));
                for (name, n) in partitions {
                    out.push_str(&format!("      {name}: {n} unit(s)\n"));
                }
            }
            PlanStep::Source {
                engine,
                text,
                limit,
                rrf_k,
            } => {
                out.push_str(&format!(
                    "  5 source: {engine} {text}, {limit} per partition, rrf k={rrf_k}\n"
                ));
            }
            PlanStep::Filter { stages } => {
                out.push_str(&format!("  6 filters: {stages} stage(s), all must hold\n"));
            }
            PlanStep::Expand {
                traversals,
                closures,
            } => {
                out.push_str("  7 expand:");
                if !traversals.is_empty() {
                    out.push_str(&format!(" {}", traversals.join(", ")));
                }
                if !closures.is_empty() {
                    out.push_str(&format!(" with {}", closures.join(", ")));
                }
                out.push('\n');
            }
            PlanStep::Supersede { enabled } => {
                out.push_str(match enabled {
                    true => "  - supersession: superseded units give way to their newest head\n",
                    false => "  - supersession: off (--all-versions), every version kept\n",
                });
            }
            PlanStep::SetOp { op, .. } => {
                out.push_str(&format!("  8 set: {op}\n"));
            }
            PlanStep::Shape {
                ordering,
                shaping,
                output,
            } => {
                out.push_str("  9 shape:");
                if !ordering.is_empty() {
                    out.push_str(&format!(" {}", ordering.join(", ")));
                }
                if !shaping.is_empty() {
                    out.push_str(&format!(" | {}", shaping.join(", ")));
                }
                match output {
                    Some(o) => out.push_str(&format!(" -> {o}\n")),
                    None => out.push('\n'),
                }
            }
        }
    }

    out
}

/// `--why` for a fused result (SMYSL-2.5 §3.4).
///
/// §3.4 says exactly what it prints: *"per hit, the partition ranks and the per-term
/// contributions inside each partition"*. Both halves matter and for different reasons. The
/// ranks are the fusion's own arithmetic — a unit's fused score is a sum over partitions and
/// nothing else, so printing the ranks makes the score checkable by hand. The contributions
/// are the retrieval's, from `Hit::terms`, and they answer the question a rank cannot: *which
/// words put it here*. A result that can name them is auditable; one that cannot is a number.
///
/// Contributions are advisory in `smysl-retrieve` — a `Retriever` is a public trait and not
/// every implementation computes them — so an empty list prints as nothing rather than as
/// "no terms matched", which would be a different and false claim.
pub fn why(fused: &[Fused]) -> String {
    let mut out = String::new();
    if fused.is_empty() {
        out.push_str("no hits\n");
        return out;
    }
    for (at, hit) in fused.iter().enumerate() {
        out.push_str(&format!(
            "{:>3}. {} rrf {:.6}\n",
            at + 1,
            hit.uid,
            hit.score
        ));
        for p in &hit.placements {
            out.push_str(&format!(
                "       {} rank {} score {:.6}\n",
                partition_name(&p.key),
                p.rank,
                p.score
            ));
            for (term, contribution) in &p.terms {
                out.push_str(&format!("         {term} {contribution:.6}\n"));
            }
        }
    }
    out
}
