//! Revision inference (§3.9, A-12.1): units carry no revision stamp, so the revision a unit was
//! written at is narrowed from what it holds.

use super::lens::UnitView;
use super::{LensChain, SchemaSet};
use crate::ids::SchemaId;
use crate::types::UnitCore;

/// The revisions of `id` a unit is consistent with, `lo..=hi`. The engine lenses from `hi`, the
/// highest; by rule N any revision in the interval reads the unit the same (law L5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionGuess {
    pub id: SchemaId,
    pub lo: u32,
    pub hi: u32,
}

impl SchemaSet {
    /// The revisions a unit is consistent with, or `None` when no single id governs its type
    /// or that id has no current revision (a fork).
    ///
    /// The clues are A-12.1's, over the unit's core:
    /// - a key it holds that a revision retires (field `retired-in`), or that a step renames,
    ///   maps or retires away: it is older than that revision;
    /// - a key required from a revision (under its condition, if any) that it lacks: older;
    /// - its type, declared only from some revision: no older than that.
    ///
    /// The field clues are read from each revision's table, and the step clues by one rule:
    /// **a unit a step would change was written before that step**, because under rule N a
    /// step only ever changes what an earlier revision named. That covers renamed types, renamed and retired keys, mapped
    /// and split values and defaulted keys together, and it is the rule that makes L5 hold.
    /// The third clue, the revisions an extracted unit's attestation recipe records, is not a
    /// property of the core and is the caller's to intersect.
    pub fn infer_revision(&self, core: &UnitCore) -> Option<RevisionGuess> {
        let id = self.governing(&core.schema)?.clone();
        let current = self.current(&id)?.version;
        let revs = self.revisions(&id);
        let lo = revs
            .iter()
            .filter(|r| r.declares_type(&core.schema))
            .map(|r| r.version)
            .min()?;
        let view = UnitView::of(core);
        // The highest declared version strictly below `v`, or `lo` when there is none.
        let below = |v: u32| {
            revs.iter()
                .map(|r| r.version)
                .filter(|&x| x < v)
                .max()
                .unwrap_or(lo)
        };
        let mut hi = current;
        for r in revs {
            let Some(table) = r.fields.typed() else {
                continue;
            };
            for (k, f) in table {
                if let Some(at) = f.retired_in {
                    if view.payload.contains_key(k) {
                        hi = hi.min(below(at));
                    }
                }
                if let Some(at) = f.required_from {
                    let applies = f
                        .required_when
                        .as_ref()
                        .is_none_or(|c| c.holds(&view.payload));
                    if applies && !view.payload.contains_key(k) {
                        hi = hi.min(below(at));
                    }
                }
            }
        }
        for s in self.steps(&id) {
            if !s.is_usable() || s.from >= hi {
                continue;
            }
            let one = LensChain::from_steps(id.clone(), s.from, s.to, vec![s.clone()])
                .expect("one step is a contiguous chain");
            let mut v = view.clone();
            one.forward(&mut v);
            if v != view {
                hi = hi.min(s.from);
            }
        }
        Some(RevisionGuess {
            id,
            lo,
            hi: hi.max(lo),
        })
    }
}
