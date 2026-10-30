//! Pointer lenses and `pointer("…")` (SMYSL-2.5 §3.12; TX-P12 step 8).
//!
//! A `json/1` snapshot addresses its values by JSON Pointer, and a configuration renames them
//! between versions: `/database/pool/max_size` in one snapshot is `/db/pool/maxSize` in the
//! next. §3.12 lenses pointers with the op set it lenses payload keys with, and this module is
//! that reading, in four decisions the RFC leaves open:
//!
//! - **A pointer lens is a schema declaration whose `rename-key` or `retire` ops name
//!   pointers** — keys that start with `/` — and it names the family of snapshots it governs:
//!   `x.config/v1`, say, at revisions 1 and 2 with a step `rename-key /database → /db`. Its id
//!   is what a snapshot's marker refers to; it needs no field table.
//! - **A rename moves everything under it.** A pointer is a path, so renaming `/database`
//!   renames `/database/pool/max_size` to `/db/pool/max_size`; a `retire` of a pointer takes
//!   its subtree with it.
//! - **The version marker is a manifest field** (§3.12 names it as one option): the snapshot's
//!   manifest records `identifiers[<schema id>] = <revision>`, and a unit is in a snapshot by
//!   its source's manifest. The other option §3.12 names, a shape fingerprint, is not built.
//! - **A unit's locator is its source reference** after the last `#` (`config.json#/db/pool`),
//!   or the whole reference when it starts with `/`. Locators are never rewritten.
//!
//! `pointer(p)` compiles to one pointer per revision of each pointer lens that renames `p` or
//! anything above it: `p` is read at the last revision before a step renames it away (rule N
//! forbids a pointer coming back), carried forward through the later steps and back through
//! the earlier ones. A unit matches when its locator is its snapshot's pointer or under it. A
//! unit whose snapshot has no marker is matched against `p` as written, without a lens, and
//! counted (`SMY-W475`).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{SchemaId, Uid};
use smysl_core::schema::{LensOp, LensStep, SchemaSet};
use smysl_core::types::Record;
use smysl_graph::Store;

/// `pointer(p)`, compiled: the pointer each revision of each pointer lens spells it as.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PointerPlan {
    /// As written.
    pub pointer: String,
    /// `(schema id, revision, pointer)`, by id then revision; `None` where a revision retired
    /// it.
    pub across: Vec<(String, u32, Option<String>)>,
}

fn is_pointer(k: &str) -> bool {
    k.starts_with('/')
}

/// Whether a step names pointers.
fn names_pointers(s: &LensStep) -> bool {
    s.ops.iter().any(|o| match o {
        LensOp::RenameKey { from, to } => is_pointer(from) || is_pointer(to),
        LensOp::Retire { key } => is_pointer(key),
        _ => false,
    })
}

/// `p` with a renamed prefix replaced, if `from` is `p` or above it.
fn moved(p: &str, from: &str, to: &str) -> Option<String> {
    if p == from {
        return Some(to.to_string());
    }
    p.strip_prefix(from)
        .filter(|rest| rest.starts_with('/'))
        .map(|rest| format!("{to}{rest}"))
}

fn under(p: &str, top: &str) -> bool {
    p == top || p.strip_prefix(top).is_some_and(|r| r.starts_with('/'))
}

/// One step forward: `None` once retired.
fn forward(p: &str, s: &LensStep) -> Option<String> {
    let mut p = p.to_string();
    for op in &s.ops {
        match op {
            LensOp::RenameKey { from, to } => {
                if let Some(n) = moved(&p, from, to) {
                    p = n;
                }
            }
            LensOp::Retire { key } if under(&p, key) => return None,
            _ => {}
        }
    }
    Some(p)
}

/// One step backward: renames undone, ops in reverse.
fn backward(p: &str, s: &LensStep) -> String {
    let mut p = p.to_string();
    for op in s.ops.iter().rev() {
        if let LensOp::RenameKey { from, to } = op {
            if let Some(n) = moved(&p, to, from) {
                p = n;
            }
        }
    }
    p
}

/// The pointer lenses of a store: every schema id whose steps name pointers.
pub fn lenses(set: &SchemaSet) -> Vec<SchemaId> {
    set.ids()
        .filter_map(|i| SchemaId::parse(i).ok())
        .filter(|id| set.steps(id).iter().any(|s| names_pointers(s)))
        .collect()
}

/// Compile `pointer(p)` against the store's pointer lenses.
pub fn compile(set: &SchemaSet, p: &str) -> PointerPlan {
    let mut across = Vec::new();
    for id in lenses(set) {
        let mut versions: Vec<u32> = set.revisions(&id).iter().map(|r| r.version).collect();
        versions.sort();
        versions.dedup();
        let Some(&current) = versions.last() else {
            continue;
        };
        let steps = set.steps(&id);
        // The revision `p` is spelled at: the last before a step moves it or its parent away.
        let at = steps
            .iter()
            .filter(|s| {
                s.ops.iter().any(|o| match o {
                    LensOp::RenameKey { from, .. } => moved(p, from, "").is_some(),
                    LensOp::Retire { key } => under(p, key),
                    _ => false,
                })
            })
            .map(|s| s.from)
            .min()
            .unwrap_or(current);
        // A lens that never touches `p` lenses nothing for it.
        let touches = steps.iter().any(|s| {
            s.ops.iter().any(|o| match o {
                LensOp::RenameKey { from, to } => {
                    moved(p, from, "").is_some() || moved(p, to, "").is_some()
                }
                LensOp::Retire { key } => under(p, key),
                _ => false,
            })
        });
        if !touches {
            continue;
        }
        for &v in &versions {
            let name = if v == at {
                Some(p.to_string())
            } else if v > at {
                set.chain(&id, at, v).ok().and_then(|c| {
                    c.steps()
                        .iter()
                        .try_fold(p.to_string(), |q, s| forward(&q, s))
                })
            } else {
                set.chain(&id, v, at).ok().map(|c| {
                    c.steps()
                        .iter()
                        .rev()
                        .fold(p.to_string(), |q, s| backward(&q, s))
                })
            };
            across.push((id.as_str().to_string(), v, name));
        }
    }
    PointerPlan {
        pointer: p.to_string(),
        across,
    }
}

/// A unit's locator: its source reference after the last `#`, or the whole reference when it is
/// a pointer.
pub fn locator(reference: &str) -> Option<&str> {
    match reference.rsplit_once('#') {
        Some((_, frag)) if is_pointer(frag) => Some(frag),
        Some(_) => None,
        None => is_pointer(reference).then_some(reference),
    }
}

/// The units among `candidates` the plan matches, and how many were read without a lens
/// because their snapshot carries no marker for any pointer lens (`SMY-W475`).
pub fn select(store: &Store, plan: &PointerPlan, candidates: &BTreeSet<Uid>) -> (Vec<Uid>, usize) {
    // A snapshot's marker: `identifiers[<schema id>]` on its manifest.
    let ids: BTreeSet<&str> = plan.across.iter().map(|(i, _, _)| i.as_str()).collect();
    let mut markers: BTreeMap<smysl_core::ids::Mid, (String, u32)> = BTreeMap::new();
    for r in store.iter() {
        if let Record::Manifest(m) = r {
            for (k, v) in &m.identifiers {
                if ids.contains(k.as_str()) {
                    if let Ok(rev) = v.parse::<u32>() {
                        markers.insert(m.mid(), (k.clone(), rev));
                    }
                }
            }
        }
    }
    let names: BTreeMap<(&str, u32), &Option<String>> = plan
        .across
        .iter()
        .map(|(i, v, n)| ((i.as_str(), *v), n))
        .collect();
    let mut out = Vec::new();
    let mut unmarked = 0;
    for uid in candidates {
        let Some(unit) = store.get(uid) else {
            continue;
        };
        let Some(src) = unit.core.source.as_ref() else {
            continue;
        };
        let Some(loc) = locator(&src.reference) else {
            continue;
        };
        let marker = src.manifest().and_then(|m| markers.get(m));
        let want: Option<&str> = match marker {
            Some((id, rev)) => match names.get(&(id.as_str(), *rev)) {
                Some(Some(n)) => Some(n.as_str()),
                // Retired at that revision: nothing there is this pointer.
                Some(None) => None,
                // A revision the lens does not declare reads the pointer as written.
                None => Some(plan.pointer.as_str()),
            },
            None => {
                if !plan.across.is_empty() {
                    unmarked += 1;
                }
                Some(plan.pointer.as_str())
            }
        };
        if want.is_some_and(|w| under(loc, w)) {
            out.push(*uid);
        }
    }
    (out, unmarked)
}
