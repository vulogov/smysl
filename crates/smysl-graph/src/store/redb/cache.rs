//! Which cores stay in memory once read (SMYSL-2.8 §3.5.4, OQ-69).
//!
//! Two tiers, chosen by the header alone, so the choice is deterministic and changes latency,
//! never an answer:
//!
//! - **pinned**: the cores of judgements — `speculative`, `inferred`, `derived` — and of every
//!   relation endpoint. Every interpretive operation reads them again, and there are few of
//!   them. Kept from first touch, never evicted.
//! - **paged**: everything else, which in a text or telemetry store is nearly everything:
//!   extractions and readings, read once by a scan and rarely again. Least recently used first
//!   out, within a byte budget.
//!
//! A budget of zero turns both off, so every `get` reads the log: the setting the conformance
//! suite runs beside "everything fits", to show the answers do not depend on the cache.

use std::collections::BTreeMap;
use std::sync::Arc;

use smysl_core::Unit;

/// How a store's core cache has done since it was opened.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct CacheStats {
    /// Reads answered from memory.
    pub hits: u64,
    /// Reads that went to the log.
    pub misses: u64,
    /// Cores held because they are pinned.
    pub pinned: usize,
    /// Cores held in the paged tier, and the bytes they are charged.
    pub paged: usize,
    pub paged_bytes: usize,
}

/// What a decoded unit holds in memory, roughly: the struct, its heap text, its edge sets and
/// its attestations, each allocation with an allocator's word or two around it.
///
/// The budget is charged this rather than the core's encoded length. The first version charged
/// the encoded length, and at 155k units a 64 MiB budget held every core in the store while
/// believing it held 12 MB of them: a decoded unit is several times its CBOR, because a short
/// gist is a heap `String` behind a 24-byte header and a set of two uids is a b-tree node.
pub(super) fn resident(u: &Unit) -> usize {
    use std::mem::size_of;
    const ALLOC: usize = 16;
    let c = &u.core;
    let heap = |n: usize| if n == 0 { 0 } else { n + ALLOC };
    let text = heap(c.gist.capacity())
        + c.body.as_ref().map_or(0, |s| heap(s.capacity()))
        + c.detail.as_ref().map_or(0, |s| heap(s.capacity()))
        + c.payload.as_ref().map_or(0, |p| heap(p.capacity()))
        + c.source
            .as_ref()
            .map_or(0, |s| heap(s.reference.capacity()));
    // A b-tree node holds up to eleven keys and is rarely full; two uids' worth a key is near
    // what small sets cost.
    let refs = (c.deps.len() + c.grounds.len()) * 2 * size_of::<smysl_core::Uid>();
    let attest = u.attestations.len() * (size_of::<smysl_core::Attestation>() + 2 * ALLOC);
    size_of::<Unit>() + 2 * ALLOC + text + refs + attest
}

#[derive(Debug)]
pub(super) struct Cache {
    budget: usize,
    pinned: BTreeMap<u32, Arc<Unit>>,
    /// Unit index to its core, the tick it was last used at, and the bytes it is charged.
    paged: BTreeMap<u32, (Arc<Unit>, u64, usize)>,
    /// Tick to unit index: the eviction order.
    used: BTreeMap<u64, u32>,
    tick: u64,
    bytes: usize,
    hits: u64,
    misses: u64,
}

impl Cache {
    pub(super) fn new(budget: usize) -> Cache {
        Cache {
            budget,
            pinned: BTreeMap::new(),
            paged: BTreeMap::new(),
            used: BTreeMap::new(),
            tick: 0,
            bytes: 0,
            hits: 0,
            misses: 0,
        }
    }

    pub(super) fn get(&mut self, i: u32) -> Option<Arc<Unit>> {
        if let Some(u) = self.pinned.get(&i) {
            self.hits += 1;
            return Some(u.clone());
        }
        let tick = self.next_tick();
        match self.paged.get_mut(&i) {
            Some((u, last, _)) => {
                self.used.remove(last);
                *last = tick;
                self.used.insert(tick, i);
                self.hits += 1;
                Some(u.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Keep a core just read, in the tier its header chose, charged `size` bytes if paged.
    pub(super) fn put(&mut self, i: u32, unit: &Arc<Unit>, pinned: bool, size: usize) {
        if self.budget == 0 {
            return;
        }
        if pinned {
            self.pinned.insert(i, unit.clone());
            return;
        }
        if size > self.budget {
            return;
        }
        let tick = self.next_tick();
        if let Some((_, last, old)) = self.paged.insert(i, (unit.clone(), tick, size)) {
            self.used.remove(&last);
            self.bytes -= old;
        }
        self.used.insert(tick, i);
        self.bytes += size;
        while self.bytes > self.budget {
            let Some((_, victim)) = self.used.pop_first() else {
                break;
            };
            if let Some((_, _, freed)) = self.paged.remove(&victim) {
                self.bytes -= freed;
            }
        }
    }

    pub(super) fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            pinned: self.pinned.len(),
            paged: self.paged.len(),
            paged_bytes: self.bytes,
        }
    }

    fn next_tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status, UnitCoreBuilder};

    fn unit(n: u32) -> Arc<Unit> {
        let core =
            UnitCoreBuilder::new(KernelType::Claim, format!("gist {n}"), Status::Speculative)
                .build()
                .unwrap();
        Arc::new(Unit::new(core))
    }

    #[test]
    fn the_paged_tier_stays_within_its_budget_and_evicts_the_least_recent() {
        let mut c = Cache::new(300);
        for i in 0..3 {
            c.put(i, &unit(i), false, 100);
        }
        // Touch 0, so 1 is now the least recently used.
        assert!(c.get(0).is_some());
        c.put(3, &unit(3), false, 100);
        assert!(c.get(1).is_none(), "the least recently used went first");
        assert!(c.get(0).is_some() && c.get(2).is_some() && c.get(3).is_some());
        assert_eq!(c.stats().paged_bytes, 300);
    }

    #[test]
    fn a_unit_is_charged_more_than_its_encoding() {
        let u = unit(1);
        let encoded = smysl_core::to_cbor(&smysl_core::Record::Unit(u.core.clone())).len();
        assert!(resident(&u) > 3 * encoded, "{} vs {encoded}", resident(&u));
    }

    #[test]
    fn pinned_cores_are_never_evicted_and_a_zero_budget_keeps_nothing() {
        let mut c = Cache::new(100);
        c.put(7, &unit(7), true, 10_000);
        for i in 0..10 {
            c.put(i, &unit(i), false, 60);
        }
        assert!(c.get(7).is_some());
        assert_eq!(c.stats().pinned, 1);
        let mut none = Cache::new(0);
        none.put(1, &unit(1), true, 1);
        none.put(2, &unit(2), false, 1);
        assert!(none.get(1).is_none() && none.get(2).is_none());
        assert_eq!(none.stats().misses, 2);
    }
}
