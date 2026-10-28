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

/// A chain hash as §4.4 abbreviates it: four hex digits, an ellipsis, two.
pub fn short_hash(h: &[u8; 32]) -> String {
    let hex: String = h.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}…{}", &hex[..4], &hex[hex.len() - 2..])
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
                fields,
            } => {
                out.push_str("  2 resolution: ");
                if relations.is_empty() {
                    out.push_str("no relation kinds named");
                } else {
                    out.push_str(&format!("relations {}", relations.join(", ")));
                }
                // Said out loud rather than left to be assumed: a payload name nothing
                // declares is read raw, and a reader who knows the phase exists would
                // otherwise read silence as "every field checked out".
                if !fields.is_empty() {
                    out.push_str(&format!("; fields {}", fields.join(", ")));
                    if !fields_checked {
                        out.push_str(" (no declaration in this store types a field)");
                    }
                }
                out.push('\n');
            }
            PlanStep::Pointer { plan } => {
                out.push_str(&format!("  5 source: pointer {:?}", plan.pointer));
                if plan.across.is_empty() {
                    out.push_str(", no pointer lens names it\n");
                } else {
                    let names: Vec<String> = plan
                        .across
                        .iter()
                        .map(|(id, v, n)| match n {
                            Some(n) => format!("{id}@{v} {n}"),
                            None => format!("{id}@{v} (retired)"),
                        })
                        .collect();
                    out.push_str(&format!(" across {}\n", names.join(", ")));
                }
            }
            PlanStep::Lens {
                pins,
                chains,
                rewrites,
            } => {
                // §4.4's three lines, under the phase that produced them.
                for (i, p) in pins.iter().enumerate() {
                    let label = if i == 0 { "pins:" } else { "" };
                    let to = if p.from == p.to {
                        "current".to_string()
                    } else {
                        format!("compiled forward to @{}", p.to)
                    };
                    out.push_str(&format!("      {label:<11}{}@{} ({to})\n", p.id, p.from));
                }
                for (i, c) in chains.iter().enumerate() {
                    let label = if i == 0 { "lenses:" } else { "" };
                    out.push_str(&format!(
                        "      {label:<11}{} {}->{}  chain {}  {} units read, {} unmappable\n",
                        c.id,
                        c.from,
                        c.to,
                        short_hash(&c.hash),
                        c.read,
                        c.unmappable
                    ));
                }
                for (i, r) in rewrites.iter().enumerate() {
                    let label = if i == 0 { "rewrites:" } else { "" };
                    out.push_str(&format!(
                        "      {label:<11}{}  =>  {}   ({})\n",
                        r.before,
                        r.after,
                        r.why.join("; ")
                    ));
                }
            }
            PlanStep::Scope {
                scope,
                items,
                stores,
                candidates,
                excluded,
            } => {
                let asked = if items.is_empty() {
                    "everything (no `from`)".to_string()
                } else {
                    items.join(", ")
                };
                let left_out = match excluded {
                    0 => String::new(),
                    n => format!(" ({n} excluded)"),
                };
                out.push_str(&format!(
                    "  3 scope: {asked} via {scope} -> {} store(s) {}, {candidates} unit(s){left_out}\n",
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
                for (name, n, chain) in partitions {
                    match chain {
                        Some(id) => out.push_str(&format!("      {name}: {n} unit(s), {id}\n")),
                        None => out.push_str(&format!("      {name}: {n} unit(s)\n")),
                    }
                }
            }
            PlanStep::Source {
                engine,
                lang,
                text,
                limit,
                rrf_k,
            } => {
                let lang = match lang {
                    Some(code) => format!(" lang:{code}"),
                    None => String::new(),
                };
                if *engine == "entity" {
                    // A selection, not a ranking: nothing per partition and no fusion.
                    out.push_str(&format!("  5 source: {engine} {text}, mentions walk\n"));
                } else {
                    out.push_str(&format!(
                        "  5 source: {engine}{lang} {text}, {limit} per partition, rrf k={rrf_k}\n"
                    ));
                }
            }
            PlanStep::Semantic {
                engine,
                route,
                lang,
                bridge,
                text,
                limit,
                rrf_k,
                model,
            } => {
                let lang = match lang {
                    Some(code) => format!(" lang:{code}"),
                    None => String::new(),
                };
                let route = match route {
                    Some(r) => format!(" (routes {r})"),
                    None => String::new(),
                };
                let model = match model {
                    Some(h) => format!(", model {}", &h[..12]),
                    None => String::new(),
                };
                if *bridge {
                    out.push_str(&format!(
                        "  5 source: {engine}{route} lang:any {text}, {limit} across every \
                         partition, embedding bridge{model}\n"
                    ));
                } else {
                    out.push_str(&format!(
                        "  5 source: {engine}{route}{lang} {text}, {limit} per partition, rrf \
                         k={rrf_k}{model}\n"
                    ));
                }
            }
            PlanStep::Filter { stages } => {
                out.push_str(&format!("  6 filters: {stages} stage(s), all must hold\n"));
            }
            PlanStep::Time { stages } => {
                out.push_str(&format!(
                    "  6 time: {}; undated units excluded and counted (W415)\n",
                    stages.join(", ")
                ));
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
            PlanStep::KeepsHistory => {
                out.push_str("  - supersession: what `<-supersedes` reached is kept as written\n");
            }
            PlanStep::SupersedeSeeds => {
                out.push_str("  - supersession: seeds give way to their newest head first\n");
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
