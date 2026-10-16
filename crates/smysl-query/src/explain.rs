//! `--explain` (SMYSL-2.5 §3.2, §4.2).
//!
//! §3.2's requirement is the whole design: each phase "records what it did, so `--explain`
//! prints the plan rather than reconstructing it". So this file is a formatter over
//! [`PlanStep`] and holds no planning logic — if a line here had to work something out, the
//! phase that should have recorded it did not.
//!
//! The output is a plain text block, one line per phase, stable enough to be a golden. That
//! is the exit TX-P6 step 2 asks for: plan snapshots over `fixtures/corpus`.

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
        }
    }

    // The honest end of a four-phase planner. A plan that stopped at phase 4 and did not say
    // so would read like a plan that finished.
    out.push_str("  phases 5-9 (sources, filters, traversals, set ops, ordering) are not in\n");
    out.push_str("  this build; a query that needs one is refused with SMY-E461\n");
    out
}
