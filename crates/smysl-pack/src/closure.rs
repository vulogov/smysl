//! Closure expansion (§18.2).
//!
//! Selecting a unit is never a single purchase. Buying it at L1 drags in its deps, grounds
//! and warrants at L0; buying it at any level drags in its rebuttals and every position of
//! any contention it touches - and each of *those* drags in its own.
//!
//! The expansion is a fixpoint rather than one pass, because C3 and C4 are transitive:
//! pinning a rebuttal can pin that rebuttal's own contentions. It terminates because the
//! required set only grows and is bounded by the graph.

use std::collections::BTreeMap;

use smysl_core::{Lod, Uid};
use smysl_graph::Store;

use crate::constraints::{warrants_of, Selection};

/// The minimum levels a selection must reach for `(uid, level)` to be admissible.
///
/// Includes `(uid, level)` itself. Levels already met by `selected` are still reported, so
/// a caller can see the whole obligation; use [`delta`] for what it would actually cost.
pub fn required(store: &Store, uid: Uid, level: Lod) -> Selection {
    let mut need: Selection = Selection::new();
    let mut work: Vec<(Uid, Lod)> = vec![(uid, level)];

    while let Some((x, lx)) = work.pop() {
        // Raise the requirement if this visit demands more than a previous one did.
        match need.get(&x) {
            Some(existing) if *existing >= lx => continue,
            _ => {
                need.insert(x, lx);
            }
        }
        if !store.contains_uid(&x) {
            continue;
        }

        if lx >= Lod::L1 {
            if let Some(unit) = store.get(&x) {
                // C1, C2: interpretable from the L0 of its deps and grounds (rule L).
                for d in unit.core.references() {
                    if store.contains_uid(d) {
                        work.push((*d, Lod::L0));
                    }
                }
            }
            // C6: the inferential licence travels with the inference.
            for w in warrants_of(store, &x) {
                work.push((w, Lod::L0));
            }
        }

        // C3 binds at every level. Rule R: a claim never travels unopposed.
        for r in store.rebuttals_of(&x) {
            if store.contains_uid(&r) {
                work.push((r, Lod::L0));
            }
        }

        // C4: an open contention pins every position, including the one being bought.
        for k in store.contentions().iter().filter(|k| k.is_open()) {
            if !k.pins(&x) {
                continue;
            }
            for p in &k.positions {
                if store.contains_uid(p) {
                    work.push((*p, Lod::L0));
                }
            }
        }
    }

    need
}

/// What `(uid, level)` would add to an existing selection.
///
/// Only the shortfall: a requirement already met costs nothing, which is what makes the
/// second unit from a cluster so much cheaper than the first.
pub fn delta(store: &Store, selected: &Selection, uid: Uid, level: Lod) -> Selection {
    required(store, uid, level)
        .into_iter()
        .filter(|(u, l)| match selected.get(u) {
            Some(existing) => existing < l,
            None => true,
        })
        .collect()
}

/// Why a unit is in a pack (`pack --explain`).
///
/// The TUI's pack simulator visualises exactly this: budget behaviour is the least
/// intuitive part of the format, and "why did that drop out" is far more legible as a
/// named constraint than as a number.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Reason {
    /// Named in the focus set (C5).
    Focus,
    /// Named by the active thread (C5).
    ThreadPin,
    /// Rule R: it rebuts something selected (C3).
    Rebuts(Uid),
    /// A position in an open contention touching something selected (C4).
    Contests(Uid),
    /// An interpretive prerequisite of something at L1+ (C1).
    DepOf(Uid),
    /// Evidential support for something at L1+ (C2).
    GroundOf(Uid),
    /// The inferential licence for something at L1+ (C6).
    WarrantOf(Uid),
    /// It earned its place on value per token.
    Density,
}

impl Reason {
    pub const fn constraint(&self) -> &'static str {
        match self {
            Reason::Focus | Reason::ThreadPin => "C5",
            Reason::Rebuts(_) => "C3",
            Reason::Contests(_) => "C4",
            Reason::DepOf(_) => "C1",
            Reason::GroundOf(_) => "C2",
            Reason::WarrantOf(_) => "C6",
            Reason::Density => "-",
        }
    }

    /// Whether a constraint forced this unit in, as opposed to it earning its place.
    pub const fn is_forced(&self) -> bool {
        !matches!(self, Reason::Density)
    }
}

impl core::fmt::Display for Reason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Reason::Focus => f.write_str("in focus"),
            Reason::ThreadPin => f.write_str("pinned by the thread"),
            Reason::Rebuts(u) => write!(f, "rebuts {u}"),
            Reason::Contests(u) => write!(f, "contests {u}"),
            Reason::DepOf(u) => write!(f, "dep of {u}"),
            Reason::GroundOf(u) => write!(f, "ground of {u}"),
            Reason::WarrantOf(u) => write!(f, "warrant of {u}"),
            Reason::Density => f.write_str("earned on density"),
        }
    }
}

/// Attribute each unit a closure pulled in to the unit that pulled it.
pub fn reasons(store: &Store, uid: Uid, level: Lod) -> BTreeMap<Uid, Reason> {
    let mut out = BTreeMap::new();
    let need = required(store, uid, level);

    for (x, lx) in &need {
        if *x == uid {
            continue;
        }
        // Attribute to the first selected unit that demands it, in canonical order, so
        // the explanation is a function of the graph.
        let mut reason = None;
        for (holder, hl) in &need {
            if holder == x {
                continue;
            }
            let Some(unit) = store.get(holder) else {
                continue;
            };
            if *hl >= Lod::L1 {
                if unit.core.deps.contains(x) {
                    reason = Some(Reason::DepOf(*holder));
                    break;
                }
                if unit.core.grounds.contains(x) {
                    reason = Some(Reason::GroundOf(*holder));
                    break;
                }
                if warrants_of(store, holder).contains(x) {
                    reason = Some(Reason::WarrantOf(*holder));
                    break;
                }
            }
            if store.rebuttals_of(holder).contains(x) {
                reason = Some(Reason::Rebuts(*holder));
                break;
            }
            if store
                .contentions()
                .iter()
                .any(|k| k.is_open() && k.pins(holder) && k.positions.contains(x))
            {
                reason = Some(Reason::Contests(*holder));
                break;
            }
        }
        let _ = lx;
        out.insert(*x, reason.unwrap_or(Reason::Density));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Contention, ContentionId, Detected, DetectionKind, Hlc, KernelType,
        Record, RelKind, Relation, SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder,
    };

    fn evidence(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .body("a body")
            .build()
            .unwrap()
    }

    fn claim(gist: &str, grounds: Vec<Uid>) -> UnitCore {
        let status = if grounds.is_empty() {
            Status::Speculative
        } else {
            Status::Inferred
        };
        UnitCoreBuilder::new(KernelType::Claim, gist, status)
            .grounds(grounds)
            .body("a body")
            .build()
            .unwrap()
    }

    #[test]
    fn a_lone_unit_requires_only_itself() {
        let c = claim("a claim", vec![]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        assert_eq!(
            required(&store, uc, Lod::L0),
            Selection::from([(uc, Lod::L0)])
        );
    }

    #[test]
    fn l1_pulls_in_grounds_at_l0() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);

        assert_eq!(
            required(&store, uc, Lod::L1),
            Selection::from([(uc, Lod::L1), (ue, Lod::L0)])
        );
        assert_eq!(
            required(&store, uc, Lod::L0),
            Selection::from([(uc, Lod::L0)]),
            "L0 has no closure obligation"
        );
    }

    #[test]
    fn rebuttals_come_in_at_every_level() {
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);
        for level in [Lod::L0, Lod::L1] {
            assert!(required(&store, uc, level).contains_key(&ur), "at {level}");
        }
    }

    /// The fixpoint: pinning a rebuttal pins that rebuttal's own rebuttal.
    #[test]
    fn the_closure_is_transitive() {
        let a = claim("a", vec![]);
        let ua = canonical_uid(&a);
        let b = claim("b", vec![]);
        let ub = canonical_uid(&b);
        let c = claim("c", vec![]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Rebuts, ub, ua)),
            Record::Relation(Relation::new(RelKind::Rebuts, uc, ub)),
        ]);
        let need = required(&store, ua, Lod::L0);
        assert_eq!(need.len(), 3, "a pulls b, and b pulls c");
    }

    /// A rebuttal reached at L0 pulls in its own contention positions - C3 and C4
    /// interleave, which is why one pass is not enough.
    #[test]
    fn c3_and_c4_interleave() {
        let target = claim("the target", vec![]);
        let ut = canonical_uid(&target);
        let rebuttal = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&rebuttal);
        let other = claim("the other position", vec![]);
        let uo = canonical_uid(&other);

        let k = Contention::new(
            ContentionId::new("k/x").unwrap(),
            ur,
            vec![ur, uo],
            Detected {
                kind: DetectionKind::SupersessionFork,
                ts: Hlc::new(0, 0, AgentId::new("tool:t").unwrap()),
            },
        );
        let store = Store::from_records(vec![
            Record::Unit(target),
            Record::Unit(rebuttal),
            Record::Unit(other),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, ut)),
            Record::Contention(k),
        ]);

        let need = required(&store, ut, Lod::L0);
        assert!(need.contains_key(&ur), "C3 pulled the rebuttal");
        assert!(
            need.contains_key(&uo),
            "and C4 pulled its contention partner"
        );
    }

    #[test]
    fn a_deeper_visit_raises_an_earlier_requirement() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);
        let need = required(&store, uc, Lod::L2);
        assert_eq!(need[&uc], Lod::L2);
        assert_eq!(need[&ue], Lod::L0);
    }

    /// The second unit of a cluster is cheap because its closure is already paid for.
    #[test]
    fn a_delta_only_charges_for_the_shortfall() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let a = claim("claim a", vec![ue]);
        let ua = canonical_uid(&a);
        let b = claim("claim b", vec![ue]);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(a), Record::Unit(b)]);

        let first = delta(&store, &Selection::new(), ua, Lod::L1);
        assert_eq!(first.len(), 2, "the claim and its evidence");

        let selected = Selection::from([(ua, Lod::L1), (ue, Lod::L0)]);
        let second = delta(&store, &selected, ub, Lod::L1);
        assert_eq!(
            second,
            Selection::from([(ub, Lod::L1)]),
            "the evidence is paid for"
        );
    }

    #[test]
    fn a_delta_still_charges_for_an_upgrade() {
        let c = claim("the claim", vec![]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let selected = Selection::from([(uc, Lod::L0)]);
        assert_eq!(
            delta(&store, &selected, uc, Lod::L1),
            Selection::from([(uc, Lod::L1)])
        );
        assert!(delta(&store, &selected, uc, Lod::L0).is_empty());
    }

    #[test]
    fn reasons_attribute_each_forced_unit() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let r = claim("the rebuttal", vec![]);
        let ur = canonical_uid(&r);
        let store = Store::from_records(vec![
            Record::Unit(e),
            Record::Unit(c),
            Record::Unit(r),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
        ]);

        let why = reasons(&store, uc, Lod::L1);
        assert_eq!(why[&ue], Reason::GroundOf(uc));
        assert_eq!(why[&ur], Reason::Rebuts(uc));
        assert_eq!(why[&ue].constraint(), "C2");
        assert_eq!(why[&ur].constraint(), "C3");
        assert!(why[&ur].is_forced());
        assert!(!Reason::Density.is_forced());
    }

    #[test]
    fn closure_is_deterministic() {
        let e = evidence("the evidence");
        let ue = canonical_uid(&e);
        let c = claim("the claim", vec![ue]);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(e), Record::Unit(c)]);
        assert_eq!(required(&store, uc, Lod::L1), required(&store, uc, Lod::L1));
        assert_eq!(reasons(&store, uc, Lod::L1), reasons(&store, uc, Lod::L1));
    }
}
