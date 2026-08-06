//! Rule M at the ingest boundary: weaken to what the grounds support (§9.1, §22.4).
//!
//! A model that claims `derived` from a `speculative` ground has overstated its confidence.
//! Rule T already handles the same mistake against the *rung* ceiling by capping the unit
//! and saying so; this handles it against the *grounds* ceiling the same way, so the
//! boundary has one rule rather than two: **ingest never lets a model overclaim; it caps
//! and tells you.**
//!
//! Weakening rather than rejecting, because both satisfy rule M equally - a capped unit's
//! status is at or below its weakest ground by construction - and only one of them keeps
//! the content. Rejection cascades: the units resting on a rejected one lose their ground
//! and have to go too, and nothing preserves what the model said. Worse, it is irreversible
//! against the normal flow of this system, where the evidence that would have justified the
//! claim arrives in a later merge. A weakened claim is still there to be strengthened; a
//! rejected one is gone.
//!
//! # Identity moves
//!
//! Status is hashed into the uid, so **weakening a unit changes its identity**, and every
//! unit grounded on it then points at something that does not exist. The pass therefore
//! walks the batch in topological order and rewrites references as it goes, which changes
//! those units' uids in turn. That cascade is the whole difficulty here, and doing it in one
//! ordered sweep is what keeps it from needing a fixed point.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{canonical_uid, Status, Uid, UnitCore};
use smysl_graph::Store;

use crate::ceiling;

/// One unit weakened to what its grounds support.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Weakening {
    /// The uid the model's unit had.
    pub before: Uid,
    /// The uid it has now. Different, because status is hashed.
    pub after: Uid,
    pub from: Status,
    pub to: Status,
    /// The ground that set the cap, for a diagnostic that names it.
    pub weakest: Option<Uid>,
}

/// What the pass did to a batch.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Applied {
    /// The batch, rewritten. Same units, same order, satisfying rule M.
    pub units: Vec<UnitCore>,
    /// Old uid to new, for every unit whose identity moved - including those that moved
    /// only because a reference inside them was rewritten.
    pub remap: BTreeMap<Uid, Uid>,
    /// Units whose status was lowered.
    pub weakened: Vec<Weakening>,
}

/// The status a uid carries, looking first at what this pass has already rewritten, then at
/// the batch, then at the store.
fn status_of(
    uid: &Uid,
    rewritten: &BTreeMap<Uid, UnitCore>,
    original: &BTreeMap<Uid, UnitCore>,
    store: &Store,
) -> Option<Status> {
    if let Some(u) = rewritten.get(uid) {
        return Some(u.status);
    }
    if let Some(u) = original.get(uid) {
        return Some(u.status);
    }
    store.get(uid).map(|u| u.core.status)
}

/// Re-point references after a transform moved identities.
///
/// Re-exported from `smysl_graph::relink`, which is where it belongs: rule T's cap, rule
/// M's weakening and `relink` all need it, and the third copy is the one that proves it is
/// graph surgery rather than an ingest detail.
pub use smysl_graph::relink::resettle;

/// Bring a batch into rule M, weakening what overclaims and rewriting what then points at
/// a moved identity.
pub fn apply(store: &Store, units: Vec<UnitCore>) -> Applied {
    let original: BTreeMap<Uid, UnitCore> = units
        .iter()
        .map(|u| (canonical_uid(u), u.clone()))
        .collect();

    // Placed back by index, so the batch keeps its input order however the topological
    // sweep visited it.
    let mut placed: Vec<Option<UnitCore>> = vec![None; units.len()];
    let mut out = Applied::default();
    let mut rewritten: BTreeMap<Uid, UnitCore> = BTreeMap::new();

    for i in smysl_graph::relink::topological(&units) {
        let before = canonical_uid(&units[i]);
        let mut core = units[i].clone();

        // Follow any ground or dep whose identity this pass has already moved.
        let follow = |set: &BTreeSet<Uid>, remap: &BTreeMap<Uid, Uid>| -> BTreeSet<Uid> {
            set.iter()
                .map(|u| remap.get(u).copied().unwrap_or(*u))
                .collect()
        };
        core.grounds = follow(&core.grounds, &out.remap);
        core.deps = follow(&core.deps, &out.remap);

        // The cap is the weakest ground. A unit with no grounds has nothing to exceed;
        // rule T has already capped it against the rung.
        let weakest = core
            .grounds
            .iter()
            .filter_map(|g| status_of(g, &rewritten, &original, store).map(|s| (s, *g)))
            .min_by_key(|(s, _)| *s);

        let from = core.status;
        if let Some((cap, ground)) = weakest {
            if core.status > cap {
                // `attainable` walks down to the strongest status the *shape* still
                // supports and floors at `speculative`, which needs nothing - so there is
                // always somewhere to land and this pass never has to reject.
                core.status = ceiling::attainable(&core, cap);
                let after = canonical_uid(&core);
                out.weakened.push(Weakening {
                    before,
                    after,
                    from,
                    to: core.status,
                    weakest: Some(ground),
                });
            }
        }

        let after = canonical_uid(&core);
        if after != before {
            out.remap.insert(before, after);
        }
        rewritten.insert(after, core.clone());
        placed[i] = Some(core);
    }

    out.units = placed.into_iter().flatten().collect();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Record, UnitCoreBuilder};

    fn unit(kind: KernelType, gist: &str, status: Status, grounds: Vec<Uid>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(kind, gist, status);
        if !grounds.is_empty() {
            b = b.grounds(grounds);
        }
        b.build().unwrap()
    }

    #[test]
    fn a_unit_within_its_grounds_is_untouched() {
        let weak = unit(KernelType::Evidence, "weak", Status::Speculative, vec![]);
        let ok = unit(
            KernelType::Claim,
            "ok",
            Status::Speculative,
            vec![canonical_uid(&weak)],
        );
        let out = apply(&Store::new(), vec![weak, ok]);
        assert!(out.weakened.is_empty());
        assert!(out.remap.is_empty(), "no identity moved");
    }

    #[test]
    fn an_overclaim_is_lowered_to_its_weakest_ground() {
        let weak = unit(KernelType::Evidence, "weak", Status::Speculative, vec![]);
        let over = unit(
            KernelType::Claim,
            "over",
            Status::Derived,
            vec![canonical_uid(&weak)],
        );
        let out = apply(&Store::new(), vec![weak, over]);

        assert_eq!(out.weakened.len(), 1);
        assert_eq!(out.weakened[0].from, Status::Derived);
        assert_eq!(out.weakened[0].to, Status::Speculative);
        assert_eq!(out.units.len(), 2, "nothing was dropped");
        assert!(out.units.iter().any(|u| u.gist == "over"));
    }

    /// The cascade this pass exists for. Weakening moves an identity, so anything grounded
    /// on it must be rewritten or it points at a unit that does not exist.
    #[test]
    fn a_dependent_follows_its_ground_to_its_new_identity() {
        let weak = unit(KernelType::Evidence, "weak", Status::Speculative, vec![]);
        let over = unit(
            KernelType::Claim,
            "over",
            Status::Derived,
            vec![canonical_uid(&weak)],
        );
        let over_uid = canonical_uid(&over);
        let dependent = unit(
            KernelType::Finding,
            "dependent",
            Status::Speculative,
            vec![over_uid],
        );

        let out = apply(&Store::new(), vec![weak, over, dependent]);
        let staged: BTreeSet<Uid> = out.units.iter().map(canonical_uid).collect();

        let d = out.units.iter().find(|u| u.gist == "dependent").unwrap();
        let ground = d.grounds.iter().next().copied().unwrap();
        assert_ne!(ground, over_uid, "the dependent kept a stale reference");
        assert!(
            staged.contains(&ground),
            "the dependent points outside the batch"
        );
    }

    /// Weakening propagates: a claim lowered to `speculative` caps everything above it.
    #[test]
    fn the_cap_travels_down_a_chain() {
        let weak = unit(KernelType::Evidence, "weak", Status::Speculative, vec![]);
        let mid = unit(
            KernelType::Claim,
            "mid",
            Status::Derived,
            vec![canonical_uid(&weak)],
        );
        let top = unit(
            KernelType::Finding,
            "top",
            Status::Derived,
            vec![canonical_uid(&mid)],
        );

        let out = apply(&Store::new(), vec![weak, mid, top]);
        for u in &out.units {
            assert!(
                u.status <= Status::Speculative,
                "{} stayed at {}",
                u.gist,
                u.status
            );
        }
        assert_eq!(out.weakened.len(), 2, "both overclaims were lowered");
    }

    /// A ground already in the store is the normal case, and caps just the same.
    #[test]
    fn a_ground_in_the_store_sets_the_cap() {
        let ground = unit(KernelType::Evidence, "stored", Status::Speculative, vec![]);
        let uid = canonical_uid(&ground);
        let store = Store::from_records(vec![Record::Unit(ground)]);

        let over = unit(KernelType::Claim, "over", Status::Derived, vec![uid]);
        let out = apply(&store, vec![over]);
        assert_eq!(out.weakened.len(), 1);
        assert_eq!(out.units[0].status, Status::Speculative);
    }

    /// The whole point of choosing this over rejection: nothing is lost, whatever the
    /// model claimed.
    #[test]
    fn every_unit_survives_however_badly_it_overclaimed() {
        let weak = unit(KernelType::Evidence, "weak", Status::Speculative, vec![]);
        let mut units = vec![weak.clone()];
        for i in 0..5 {
            units.push(unit(
                KernelType::Claim,
                &format!("overclaim {i}"),
                Status::Derived,
                vec![canonical_uid(&weak)],
            ));
        }
        let out = apply(&Store::new(), units);
        assert_eq!(out.units.len(), 6);
        assert_eq!(out.weakened.len(), 5);
    }

    /// A cycle cannot be ordered and cannot be checked for rule M. The pass carries the
    /// units unchanged rather than looping; `check` reports `SMY-E061`, which is fatal.
    #[test]
    fn a_cycle_is_carried_rather_than_looped_on() {
        // Built by hand, because a builder cannot make two units ground on each other.
        let mut a = unit(KernelType::Claim, "a", Status::Speculative, vec![]);
        let mut b = unit(KernelType::Claim, "b", Status::Speculative, vec![]);
        a.grounds = BTreeSet::from([canonical_uid(&b)]);
        b.grounds = BTreeSet::from([canonical_uid(&a)]);

        let out = apply(&Store::new(), vec![a, b]);
        assert_eq!(out.units.len(), 2, "both carried");
    }
}
