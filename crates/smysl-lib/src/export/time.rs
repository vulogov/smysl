//! Time in an export (SMYSL-2.7 §3.4, §3.10): the `by-time` key and the `--as-of` subset.

use crate::import::classify::{names, Id};
use smysl_core::{canonical_uid, Record, SchemaId, Uid};
use smysl_graph::Store;
use smysl_text::time::calendar::civil_from_ms;
use smysl_text::time::engine::effective;
use smysl_text::time::{Axis, Bound, Subject};
use std::collections::{BTreeMap, BTreeSet};

const MS_PER_DAY: i64 = 86_400_000;

/// The key a unit takes under `by-time`: `t` and the start of the window its effective time
/// (rule E, the `said` axis) begins in, as `yyyymmddthhmmssz` in UTC. §3.4 writes the
/// separators upper-case, and a key is a view-id segment, which is lower-case: written as §3.4
/// has it, every window would carry a hash suffix (`SMY-W497`). A unit with no lower
/// bound goes to `unkeyed`: undated is neither early nor late (A-2.4 rule 5), so it is in no
/// window.
///
/// Windows are aligned to the Unix epoch, so the key is a function of the unit's time and the
/// window alone, and a re-export cuts the same files.
pub(crate) fn by_time(store: &Store, sel: &BTreeSet<Uid>, window_ms: u64) -> BTreeMap<Uid, String> {
    let eff = effective(store, Axis::Said);
    let w = (window_ms.max(1)).min(i64::MAX as u64) as i64;
    sel.iter()
        .map(|u| {
            let key = match eff.interval(&Subject::Unit(*u)).lo {
                Bound::At(at) => window_key(at.0.div_euclid(w) * w),
                Bound::Open => "unkeyed".to_string(),
            };
            (*u, key)
        })
        .collect()
}

fn window_key(ms: i64) -> String {
    let c = civil_from_ms(ms);
    let in_day = ms.rem_euclid(MS_PER_DAY) / 1000;
    format!(
        "t{:04}{:02}{:02}t{:02}{:02}{:02}z",
        c.year,
        c.month,
        c.day,
        in_day / 3600,
        in_day / 60 % 60,
        in_day % 60
    )
}

/// What `--as-of` keeps of a store, and what it counted.
pub(crate) struct Known {
    pub store: Store,
    /// Units with no attestation, whose known time is therefore unknown (`SMY-W415`).
    pub unknown: usize,
    /// Units known in time whose `deps` or `grounds` were not (`SMY-W498`).
    pub unclosed: usize,
}

/// The known-time subset of `store` at an upper bound, exclusive, in Unix milliseconds
/// (§3.10).
///
/// 1. A record with a clock of its own (an attestation, dating, commitment, withdrawal,
///    resolution, redaction or thread) is kept when its wall time is before the bound.
/// 2. A unit's known time is its earliest attestation. One with none is left out and counted
///    (`W415`).
/// 3. A relation's is its earliest attestation, or else the later of its two ends'.
/// 4. A unit whose `deps` or `grounds` name a unit the store holds and the subset does not is
///    left out too, to a fixpoint, and counted (`W498`): a file resting on what was not yet
///    known would not be closed.
/// 5. Everything else enters by the inclusion rules. Here that means a record is kept only when
///    every unit, relation and dating it names is kept or was never held at all: an attestation
///    of a unit that was not yet known is not yet known either.
/// 6. Of a schema's declarations, the versions above the highest one a kept unit or relation
///    needs are cut.
pub(crate) fn known(store: &Store, before_ms: i64) -> Known {
    let in_time = |wall: u64| (wall as i64) < before_ms;
    let clock = |r: &Record| -> Option<u64> {
        Some(match r {
            Record::Attestation(a) => a.ts.wall_ms,
            Record::Dating(d) => d.ts.wall_ms,
            Record::Commit(c) => c.ts.wall_ms,
            Record::Withdrawal(w) => w.ts.wall_ms,
            Record::Resolution(x) => x.ts.wall_ms,
            Record::Redaction(x) => x.ts.wall_ms,
            Record::Thread(t) => t.ts.wall_ms,
            _ => return None,
        })
    };
    let mut first: BTreeMap<Uid, u64> = BTreeMap::new();
    for r in store.iter() {
        if let Record::Attestation(a) = r {
            let e = first.entry(a.uid).or_insert(a.ts.wall_ms);
            *e = (*e).min(a.ts.wall_ms);
        }
    }
    // What the whole store holds by uid: units, relations by rid, datings by did.
    let mut held: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
    let mut unknown = 0;
    let mut kept: BTreeSet<Uid> = BTreeSet::new();
    for (u, _) in store.units() {
        match first.get(u) {
            None => unknown += 1,
            Some(t) if in_time(*t) => {
                kept.insert(*u);
            }
            Some(_) => {}
        }
    }
    let relations: Vec<&smysl_core::Relation> = store
        .iter()
        .filter_map(|r| match r {
            Record::Relation(x) => Some(x),
            _ => None,
        })
        .collect();
    for r in store.iter() {
        match r {
            Record::Relation(x) => {
                held.insert(x.uid());
            }
            Record::Dating(d) => {
                held.insert(d.did().as_uid());
            }
            _ => {}
        }
    }
    // Rule 4, to a fixpoint.
    let mut unclosed = 0;
    loop {
        let out: Vec<Uid> = kept
            .iter()
            .filter(|u| {
                store.get(u).is_some_and(|x| {
                    x.core
                        .references()
                        .any(|d| store.contains_uid(d) && !kept.contains(d))
                })
            })
            .copied()
            .collect();
        if out.is_empty() {
            break;
        }
        unclosed += out.len();
        for u in out {
            kept.remove(&u);
        }
    }
    let known_unit = |u: &Uid| -> Option<u64> { kept.contains(u).then(|| first[u]) };
    let mut rids = Vec::new();
    for x in &relations {
        let rid = x.uid();
        let t = match first.get(&rid) {
            Some(t) => Some(*t),
            None => match (known_unit(&x.from), known_unit(&x.to)) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            },
        };
        let ends = |u: &Uid| kept.contains(u) || !store.contains_uid(u);
        if t.is_some_and(in_time) && ends(&x.from) && ends(&x.to) {
            rids.push(rid);
        }
    }
    kept.extend(rids);

    // Rules 1 and 5, to a fixpoint: a dating kept makes its did held for what names it.
    let mut out: Vec<&Record> = Vec::new();
    let mut taken: BTreeSet<[u8; 32]> = BTreeSet::new();
    loop {
        let mut grew = false;
        for r in store.iter() {
            let h = Store::record_hash(r);
            if taken.contains(&h) {
                continue;
            }
            let keep = match r {
                Record::Unit(c) => kept.contains(&canonical_uid(c)),
                Record::Relation(x) => kept.contains(&x.uid()),
                _ => {
                    clock(r).is_none_or(in_time)
                        && names(r).iter().all(|i| match i {
                            Id::Uid(u) => kept.contains(u) || !held.contains(u),
                            _ => true,
                        })
                }
            };
            if keep {
                if let Record::Dating(d) = r {
                    kept.insert(d.did().as_uid());
                }
                taken.insert(h);
                out.push(r);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    // Rule 6.
    let types: BTreeSet<&SchemaId> = out
        .iter()
        .filter_map(|r| r.as_unit().map(|c| &c.schema))
        .collect();
    let kinds: BTreeSet<&smysl_core::RelKind> = out
        .iter()
        .filter_map(|r| match r {
            Record::Relation(x) => Some(&x.kind),
            _ => None,
        })
        .collect();
    let mut needed: BTreeMap<&SchemaId, u32> = BTreeMap::new();
    for r in &out {
        if let Record::SchemaDecl(d) = r {
            if d.types.iter().any(|t| types.contains(t))
                || d.relations.iter().any(|k| kinds.contains(k))
            {
                let v = needed.entry(&d.id).or_insert(d.version);
                *v = (*v).max(d.version);
            }
        }
    }
    let records: Vec<Record> = out
        .into_iter()
        .filter(|r| match r {
            Record::SchemaDecl(d) => needed.get(&d.id).is_none_or(|v| d.version <= *v),
            _ => true,
        })
        .cloned()
        .collect();
    Known {
        store: Store::from_records(records),
        unknown,
        unclosed,
    }
}

#[cfg(test)]
mod tests {
    use super::window_key;

    #[test]
    fn a_window_key_is_its_start_in_utc() {
        assert_eq!(window_key(0), "t19700101t000000z");
        assert_eq!(window_key(1_705_276_800_000), "t20240115t000000z");
        assert_eq!(
            window_key(1_705_276_800_000 + 3_723_000),
            "t20240115t010203z"
        );
    }
}
