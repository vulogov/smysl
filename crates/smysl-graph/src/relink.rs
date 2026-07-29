//! Re-pointing references after a unit has been replaced.
//!
//! **Identity is content**, which is what makes merge coordination-free — and what makes
//! editing sharp-edged. Change a unit and its uid changes; anything that referenced the old
//! one now references something that is not there.
//!
//! Inside a single document this is invisible and safe: references are written as labels and
//! uids are recomputed on every parse, so an edit re-resolves. Across stores it is not.
//! A store holds `grounds: [uid]`, and a corrected unit is a *different* unit — the old one
//! stays, the new one arrives, and nothing connects what pointed at the first to the second.
//! "Open it and change a line" was only ever true of the first case.
//!
//! `supersedes` is the edge that says *this replaces that*, and it is the only honest basis
//! for re-pointing: it is a statement someone made, not a similarity this module guessed.
//! Where it is absent, the reference is reported and left alone.
//!
//! The store is append-only, so nothing is rewritten in place. Relinking **emits new
//! records**: a corrected unit, plus a `supersedes` edge from it to the one it replaces.
//! The old graph stays readable and rule U still holds.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{canonical_uid, Record, RelKind, Relation, Uid, UnitCore};

use crate::store::Store;

/// Batch units ordered so a unit's grounds inside the batch come before it.
///
/// Kahn over the grounds edges. A cycle cannot be ordered, and the units it holds are left
/// in input order: rule M is unverifiable through a cycle, `check` reports `SMY-E061`, and
/// that is fatal regardless — so this declines to guess rather than looping.
pub fn topological(units: &[UnitCore]) -> Vec<usize> {
    let index: BTreeMap<Uid, usize> = units
        .iter()
        .enumerate()
        .map(|(i, u)| (canonical_uid(u), i))
        .collect();

    let mut pending: Vec<BTreeSet<usize>> = units
        .iter()
        .map(|u| {
            u.grounds
                .iter()
                .filter_map(|g| index.get(g).copied())
                .collect()
        })
        .collect();

    let mut out = Vec::with_capacity(units.len());
    let mut done = vec![false; units.len()];
    for _ in 0..units.len() {
        let Some(next) = (0..units.len()).find(|i| !done[*i] && pending[*i].is_empty()) else {
            break;
        };
        done[next] = true;
        out.push(next);
        for p in pending.iter_mut() {
            p.remove(&next);
        }
    }
    out.extend((0..units.len()).filter(|i| !done[*i]));
    out
}

/// Re-point every reference after units have been rewritten in place.
///
/// **Any transform that changes a unit's content changes its uid**, and everything pointing
/// at the old one then dangles. Rule T's cap, rule M's weakening and relinking all have that
/// shape, which is why this lives here rather than beside any one of them.
///
/// `before[i]` is the uid `units[i]` had prior to the transform. The sweep is topological, so
/// a unit is re-pointed only once everything it references has settled, and one pass suffices
/// rather than iterating to a fixed point.
pub fn resettle(
    before: &[Uid],
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
) -> (Vec<UnitCore>, Vec<Relation>, BTreeMap<Uid, Uid>) {
    let mut remap: BTreeMap<Uid, Uid> = BTreeMap::new();
    for (i, u) in units.iter().enumerate() {
        let Some(was) = before.get(i) else { continue };
        let now = canonical_uid(u);
        if now != *was {
            remap.insert(*was, now);
        }
    }

    let mut placed: Vec<Option<UnitCore>> = vec![None; units.len()];
    for i in topological(&units) {
        let was = canonical_uid(&units[i]);
        let mut core = units[i].clone();
        core.grounds = follow(&core.grounds, &remap);
        core.deps = follow(&core.deps, &remap);

        let now = canonical_uid(&core);
        if now != was {
            remap.insert(was, now);
        }
        placed[i] = Some(core);
    }

    let relations = relations
        .into_iter()
        .map(|mut r| {
            r.from = remap.get(&r.from).copied().unwrap_or(r.from);
            r.to = remap.get(&r.to).copied().unwrap_or(r.to);
            r
        })
        .collect();

    (placed.into_iter().flatten().collect(), relations, remap)
}

fn follow(set: &BTreeSet<Uid>, remap: &BTreeMap<Uid, Uid>) -> BTreeSet<Uid> {
    set.iter()
        .map(|u| remap.get(u).copied().unwrap_or(*u))
        .collect()
}

/// What relinking would do, or did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Relinked {
    /// New records to append: corrected units and the `supersedes` edges that place them.
    pub records: Vec<Record>,
    /// Units that moved, old uid to new.
    pub moved: BTreeMap<Uid, Uid>,
    /// References this cannot fix, as (holder, target). Reported rather than guessed at:
    /// a reference to something the store has never seen has no successor to follow.
    pub dangling: Vec<(Uid, Uid)>,
    /// Targets with more than one successor and no ordering between them. Re-pointing would
    /// mean choosing a winner, which is exactly what merge refuses to do — so this does too,
    /// and the disagreement stays visible.
    pub forked: Vec<Uid>,
}

impl Relinked {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Whether anything needs a human before it can be fixed.
    pub fn needs_attention(&self) -> bool {
        !self.dangling.is_empty() || !self.forked.is_empty()
    }
}

/// Follow `supersedes` to the newest replacement of each unit.
///
/// A chain `C supersedes B supersedes A` resolves `A` to `C`. A *fork* — two units
/// superseding the same target with nothing ordering them — resolves to nothing: that is a
/// contention, and picking one would be adjudicating it.
fn successors(store: &Store) -> (BTreeMap<Uid, Uid>, Vec<Uid>) {
    let mut direct: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for r in store.relations_of_kind(&RelKind::Supersedes) {
        direct.entry(r.to).or_default().insert(r.from);
    }

    let mut forked = Vec::new();
    let mut one: BTreeMap<Uid, Uid> = BTreeMap::new();
    for (target, succs) in &direct {
        match succs.len() {
            0 => {}
            1 => {
                one.insert(*target, *succs.iter().next().expect("one"));
            }
            _ => {
                // A chain among the successors is not a fork: if they are totally ordered,
                // there is a latest and no disagreement.
                let latest: Vec<&Uid> = succs
                    .iter()
                    .filter(|a| {
                        !succs
                            .iter()
                            .any(|b| b != *a && store.has_relation(&RelKind::Supersedes, b, a))
                    })
                    .collect();
                match latest.as_slice() {
                    [only] => {
                        one.insert(*target, **only);
                    }
                    _ => forked.push(*target),
                }
            }
        }
    }

    // Collapse chains, so a reference to the oldest lands on the newest.
    let mut newest = BTreeMap::new();
    for start in one.keys() {
        let mut at = *start;
        let mut seen = BTreeSet::new();
        while let Some(next) = one.get(&at) {
            if !seen.insert(at) {
                break; // a supersession cycle; leave it alone
            }
            at = *next;
        }
        if at != *start {
            newest.insert(*start, at);
        }
    }
    (newest, forked)
}

/// Re-point references onto the newest replacement of what they name.
///
/// Emits new records rather than editing: the store is append-only, and the units this
/// rewrites are themselves replaced, so each carries a `supersedes` edge back to the version
/// it corrects. Nothing is lost and the old reading stays available.
pub fn relink(store: &Store) -> Relinked {
    let mut out = Relinked::default();
    let (newest, forked) = successors(store);
    out.forked = forked;

    // **Every** unit is a candidate, not only those directly holding a replaced reference.
    // Re-pointing a unit moves its own identity, so a unit resting on *that* one has to
    // follow, and a unit resting on that one after it. Collecting only the direct holders
    // catches the first rank of a cascade and silently leaves the rest pointing at versions
    // that no longer exist - which is the failure this whole module is about.
    let mut before = Vec::new();
    let mut candidates = Vec::new();
    for (uid, unit) in store.units() {
        // A unit that has itself been replaced is not the one to fix - its replacement is,
        // and that one is in this list too. Without this, relinking a store re-points the
        // superseded version again on every run and never reaches a fixed point.
        if newest.contains_key(uid) {
            continue;
        }
        before.push(*uid);
        candidates.push(unit.core.clone());

        // Anything naming a uid the store does not hold cannot be followed anywhere.
        for r in unit.core.grounds.iter().chain(unit.core.deps.iter()) {
            if store.get(r).is_none() {
                out.dangling.push((*uid, *r));
            }
        }
    }
    if newest.is_empty() {
        out.dangling.sort();
        out.dangling.dedup();
        return out;
    }

    // Point them at the replacements, then settle the cascade: rewriting a unit moves its
    // own identity too, so whatever referenced *it* has to follow.
    let repointed: Vec<UnitCore> = candidates
        .iter()
        .map(|u| {
            let mut c = u.clone();
            c.grounds = follow(&c.grounds, &newest);
            c.deps = follow(&c.deps, &newest);
            c
        })
        .collect();
    let (settled, _, remap) = resettle(&before, repointed, Vec::new());

    for (i, core) in settled.into_iter().enumerate() {
        let Some(was) = before.get(i) else { continue };
        let now = canonical_uid(&core);
        if now == *was {
            continue;
        }
        out.moved.insert(*was, now);
        out.records.push(Record::Unit(core));
        // The corrected unit replaces the one it corrects, said out loud rather than left
        // for a reader to infer from two similar units.
        out.records.push(Record::Relation(Relation::new(
            RelKind::Supersedes,
            now,
            *was,
        )));
    }
    let _ = remap;

    out.dangling.sort();
    out.dangling.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status, UnitCoreBuilder};

    fn unit(gist: &str, grounds: Vec<Uid>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
        if !grounds.is_empty() {
            b = b.grounds(grounds);
        }
        b.build().unwrap()
    }

    /// The case the README's "change a line" glosses over: a corrected unit is a *different*
    /// unit, and what rested on the original still rests on the original.
    #[test]
    fn a_reference_follows_its_target_to_the_replacement() {
        let old = unit("the original evidence", vec![]);
        let new = unit("the corrected evidence", vec![]);
        let (old_uid, new_uid) = (canonical_uid(&old), canonical_uid(&new));
        let rests = unit("rests on the evidence", vec![old_uid]);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, new_uid, old_uid)),
        ]);

        let out = relink(&store);
        assert_eq!(out.moved.len(), 1, "nothing was re-pointed");

        // The emitted unit points at the replacement, not the original.
        let rewritten: Vec<&UnitCore> = out
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Unit(u) => Some(u),
                _ => None,
            })
            .collect();
        assert_eq!(rewritten.len(), 1);
        assert!(rewritten[0].grounds.contains(&new_uid));
        assert!(!rewritten[0].grounds.contains(&old_uid));
    }

    /// Append-only: the correction is a new record carrying `supersedes`, never an edit.
    #[test]
    fn a_relink_supersedes_rather_than_rewrites() {
        let old = unit("original", vec![]);
        let new = unit("corrected", vec![]);
        let (old_uid, new_uid) = (canonical_uid(&old), canonical_uid(&new));
        let rests = unit("rests on it", vec![old_uid]);
        let rests_uid = canonical_uid(&rests);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, new_uid, old_uid)),
        ]);

        let out = relink(&store);
        let edges: Vec<&Relation> = out
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Relation(rel) => Some(rel),
                _ => None,
            })
            .collect();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, RelKind::Supersedes);
        assert_eq!(
            edges[0].to, rests_uid,
            "the new version replaces the old one"
        );
    }

    /// Two replacements with nothing ordering them is a contention. Choosing one would be
    /// adjudicating a disagreement, which merge refuses to do and so does this.
    #[test]
    fn a_fork_is_reported_rather_than_resolved() {
        let old = unit("original", vec![]);
        let a = unit("correction a", vec![]);
        let b = unit("correction b", vec![]);
        let old_uid = canonical_uid(&old);
        let rests = unit("rests on it", vec![old_uid]);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(a.clone()),
            Record::Unit(b.clone()),
            Record::Unit(rests),
            Record::Relation(Relation::new(
                RelKind::Supersedes,
                canonical_uid(&a),
                old_uid,
            )),
            Record::Relation(Relation::new(
                RelKind::Supersedes,
                canonical_uid(&b),
                old_uid,
            )),
        ]);

        let out = relink(&store);
        assert_eq!(out.forked, vec![old_uid]);
        assert!(out.is_empty(), "a fork must not be silently re-pointed");
        assert!(out.needs_attention());
    }

    /// A chain resolves to its newest link, so a reference to the oldest lands on the last.
    #[test]
    fn a_chain_resolves_to_the_newest_replacement() {
        let a = unit("first", vec![]);
        let b = unit("second", vec![]);
        let c = unit("third", vec![]);
        let (ua, ub, uc) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&c));
        let rests = unit("rests on the first", vec![ua]);

        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, ub, ua)),
            Record::Relation(Relation::new(RelKind::Supersedes, uc, ub)),
        ]);

        let out = relink(&store);
        let rewritten = out
            .records
            .iter()
            .find_map(|r| match r {
                Record::Unit(u) => Some(u),
                _ => None,
            })
            .expect("a rewritten unit");
        assert!(rewritten.grounds.contains(&uc), "did not reach the newest");
    }

    /// A reference to something the store has never held has no successor to follow. It is
    /// reported for a human, not guessed at.
    #[test]
    fn an_unknown_reference_is_reported_not_invented() {
        let missing = Uid::from_bytes([9; 32]);
        let rests = unit("rests on nothing", vec![missing]);
        let store = Store::from_records(vec![Record::Unit(rests.clone())]);

        let out = relink(&store);
        assert!(out.is_empty(), "nothing could honestly be re-pointed");
        assert_eq!(out.dangling, vec![(canonical_uid(&rests), missing)]);
        assert!(out.needs_attention());
    }

    /// A store with nothing superseded needs no work, and must not manufacture any.
    #[test]
    fn a_store_with_no_replacements_is_left_alone() {
        let a = unit("a", vec![]);
        let b = unit("b", vec![canonical_uid(&a)]);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

        let out = relink(&store);
        assert!(out.is_empty());
        assert!(!out.needs_attention());
    }

    /// Re-pointing moves the holder's identity too, so a unit resting on *it* has to follow.
    #[test]
    fn the_cascade_settles_through_a_chain_of_holders() {
        let old = unit("original", vec![]);
        let new = unit("corrected", vec![]);
        let (old_uid, new_uid) = (canonical_uid(&old), canonical_uid(&new));
        let mid = unit("rests on the evidence", vec![old_uid]);
        let mid_uid = canonical_uid(&mid);
        let top = unit("rests on the middle", vec![mid_uid]);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(mid),
            Record::Unit(top),
            Record::Relation(Relation::new(RelKind::Supersedes, new_uid, old_uid)),
        ]);

        let out = relink(&store);
        assert_eq!(
            out.moved.len(),
            2,
            "the holder moved but its dependent did not"
        );

        // Applying the result leaves nothing pointing at a replaced unit.
        let mut all: Vec<Record> = store.iter().cloned().collect();
        all.extend(out.records.clone());
        let after = Store::from_records(all);
        let newest: Vec<Uid> = out.moved.values().copied().collect();
        for uid in &newest {
            let unit = after.get(uid).expect("the rewritten unit is present");
            for g in &unit.core.grounds {
                assert!(
                    !out.moved.contains_key(g),
                    "still points at a replaced unit"
                );
            }
        }
    }

    /// Relinking a relinked store must find nothing left to do. Without skipping units that
    /// have themselves been replaced, the superseded version is re-pointed on every run and
    /// the operation never settles.
    #[test]
    fn relinking_twice_is_a_no_op_the_second_time() {
        let old = unit("original", vec![]);
        let new = unit("corrected", vec![]);
        let (old_uid, new_uid) = (canonical_uid(&old), canonical_uid(&new));
        let rests = unit("rests on it", vec![old_uid]);

        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, new_uid, old_uid)),
        ]);

        let first = relink(&store);
        assert!(!first.is_empty(), "nothing happened on the first pass");

        let mut all: Vec<Record> = store.iter().cloned().collect();
        all.extend(first.records);
        let after = Store::from_records(all);

        assert!(
            relink(&after).is_empty(),
            "relinking did not reach a fixed point"
        );
    }

    /// Deterministic: the same store relinks to the same records (rule D).
    #[test]
    fn relinking_is_reproducible() {
        let old = unit("original", vec![]);
        let new = unit("corrected", vec![]);
        let (old_uid, new_uid) = (canonical_uid(&old), canonical_uid(&new));
        let rests = unit("rests on it", vec![old_uid]);
        let store = Store::from_records(vec![
            Record::Unit(old),
            Record::Unit(new),
            Record::Unit(rests),
            Record::Relation(Relation::new(RelKind::Supersedes, new_uid, old_uid)),
        ]);
        assert_eq!(relink(&store).records, relink(&store).records);
    }
}
