//! Shards read as one store (SMYSL-2.8 §3.6–§3.7).
//!
//! [`Union`] borrows a set of shards and answers every [`StoreRead`] question as
//! `Store::from_records` of their records would — the shards' records concatenated **in shard-id
//! order**, whatever order they were handed over in. Most answers do not depend on that order
//! at all, because merge is a join-semilattice (rule U): units, relations, attestations,
//! commitments, the library layer, the record-set digest and the state hash are the same for
//! any concatenation. The few that do — the log itself, records of a [`HotKind`], recorded
//! contentions (first by id) and views (last by id) — follow shard-id order, so a union is a
//! function of *which* shards it holds and nothing else.
//!
//! **What is materialised**, at construction: the uid list (a merge of the shards' sorted uid
//! lists, with the first shard that holds each), the interpretive layer and the library layer
//! (the shards' own, joined), the records of each [`HotKind`], and a merged copy of every unit
//! two shards both hold or that an attestation in another shard lands on. All of it is
//! proportional to uids and judgements, not to text.
//!
//! **What is read through**: cores. `get` asks the first shard that holds the uid, and a core is
//! content-addressed, so any shard's copy is the same bytes. The adjacency is derived lazily from
//! the shards' own adjacencies — their `deps` and `grounds` edges — and the union's relations,
//! so building it reads no core either.
//!
//! **Waiting on a shard.** A reference to a uid no loaded shard holds is a dangling node, as in
//! any store. Given a [`UidShards`] index ([`Union::with_index`]), `report_dangling` says
//! `SMY-W500` for a uid the index places in a shard that is not loaded, instead of `SMY-E060`:
//! the reference waits, as an early attestation does, and loading that shard resolves it.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Deref;
use std::sync::{Arc, OnceLock};

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::{Attestation, Commit, Contention, Record, RelKind, Relation, Status, Uid, Unit};

use super::interp::Interpretive;
use super::library::StoreLibrary;
use super::read::{empty_commits, sealed};
use super::{
    digest_of_hashes, state_hash_of, Commits, HotKind, HotRecords, Relations, Resolutions, Store,
    StoreRead, Threads, Views, Withdrawals,
};
use crate::adjacency::{Adjacency, EdgeKind, UnitEdges};

/// A shard's name: an expression alias, or `catalog` (SMYSL-2.8 §3.6 treats it as opaque).
pub type ShardId = String;

const ALL_HOT: [HotKind; 4] = [
    HotKind::SchemaDecls,
    HotKind::LabelBindings,
    HotKind::PackInfos,
    HotKind::Unknown,
];

/// Shards read as the store their records would make (§3.7).
///
/// Read-only, like [`crate::Overlay`]: an append to a union would have to choose a shard, and
/// choosing is the library's job.
#[derive(Clone)]
pub struct Union<S: StoreRead> {
    /// In shard-id order.
    shards: Vec<(ShardId, S)>,
    /// Every unit any shard holds, ascending, and the first shard (by position) that holds it.
    uids: Vec<Uid>,
    home: Vec<u32>,
    /// Units whose answer is not one shard's: held by two shards, or given an attestation by a
    /// shard that does not hold them. Merged.
    own: BTreeMap<Uid, Unit>,
    interp: Interpretive,
    library: StoreLibrary,
    /// Attestations whose subject no shard holds.
    pending: Vec<Attestation>,
    /// The records of each hot kind, first occurrence kept, in shard-id then log order.
    hot: Vec<Record>,
    hot_at: BTreeMap<HotKind, Vec<usize>>,
    /// Whether a record is held by more than one shard, or twice by one: the log then has to be
    /// read with a dedup, as `from_records` reads its input.
    repeats: bool,
    index: Option<Arc<UidShards>>,
    derived: OnceLock<Derived>,
}

#[derive(Clone)]
struct Derived {
    adjacency: Adjacency,
    unfounded: BTreeSet<Uid>,
}

impl<S: StoreRead> std::fmt::Debug for Union<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Union")
            .field(
                "shards",
                &self.shards.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            )
            .field("units", &self.uids.len())
            .field("merged", &self.own.len())
            .finish_non_exhaustive()
    }
}

/// A unit's two edge lists, read off a shard's adjacency rather than its core.
struct EdgeLists {
    deps: Vec<Uid>,
    grounds: Vec<Uid>,
}

impl UnitEdges for EdgeLists {
    fn deps(&self) -> impl Iterator<Item = &Uid> {
        self.deps.iter()
    }
    fn grounds(&self) -> impl Iterator<Item = &Uid> {
        self.grounds.iter()
    }
}

fn all_uids<S: StoreRead + ?Sized>(s: &S) -> Vec<Uid> {
    s.units_in_range(&Uid::from_bytes([0; 32]), &Uid::from_bytes([0xff; 32]))
}

impl<S: StoreRead> Union<S> {
    /// The union of these shards. Their order does not matter: they are read in shard-id order.
    /// Two shards under one id are both read, in the order given.
    ///
    /// The union owns what it is given; a caller that keeps its stores passes references, since
    /// `&Store` is a store too.
    pub fn new(shards: impl IntoIterator<Item = (ShardId, S)>) -> Union<S> {
        let mut shards: Vec<(ShardId, S)> = shards.into_iter().collect();
        shards.sort_by(|a, b| a.0.cmp(&b.0));

        // The uid list: every shard's, merged, with the first shard holding each.
        let mut held: Vec<(Uid, u32)> = Vec::new();
        for (i, (_, s)) in shards.iter().enumerate() {
            held.extend(all_uids(s).into_iter().map(|u| (u, i as u32)));
        }
        held.sort_unstable();
        let mut uids = Vec::with_capacity(held.len());
        let mut home = Vec::with_capacity(held.len());
        let mut own: BTreeMap<Uid, Unit> = BTreeMap::new();
        let mut at = 0;
        while at < held.len() {
            let (uid, first) = held[at];
            let mut end = at + 1;
            while end < held.len() && held[end].0 == uid {
                end += 1;
            }
            if end - at > 1 {
                // Held twice: one core, the attestations of both.
                let mut merged = shards[first as usize]
                    .1
                    .get(&uid)
                    .map(|u| (*u).clone())
                    .expect("a shard holds the uids it lists");
                for &(_, other) in &held[at + 1..end] {
                    if let Some(u) = shards[other as usize].1.get(&uid) {
                        merged.attestations.extend(u.attestations.iter().cloned());
                    }
                }
                own.insert(uid, merged);
            }
            uids.push(uid);
            home.push(first);
            at = end;
        }
        drop(held);

        // The interpretive and library layers: the shards' own, joined in shard-id order.
        let mut interp = Interpretive::default();
        let mut library = StoreLibrary::default();
        for (_, s) in &shards {
            interp.join(sealed::Sealed::interp(s));
            library.join(s.library());
        }

        // Attestations a shard was still waiting on: another shard may hold the subject.
        let mut pending = Vec::new();
        for (_, s) in &shards {
            for a in sealed::Sealed::pending_attestations(s) {
                if let Some(u) = own.get_mut(&a.uid) {
                    u.attestations.insert(a.clone());
                } else if let Ok(i) = uids.binary_search(&a.uid) {
                    let mut m = shards[home[i] as usize]
                        .1
                        .get(&a.uid)
                        .map(|u| (*u).clone())
                        .expect("a shard holds the uids it lists");
                    m.attestations.insert(a.clone());
                    own.insert(a.uid, m);
                } else if !interp.attach_to_relation(a) {
                    pending.push(a.clone());
                }
            }
        }

        // Records of each hot kind, deduplicated by content as `from_records` would.
        let mut hot = Vec::new();
        let mut hot_at: BTreeMap<HotKind, Vec<usize>> = BTreeMap::new();
        let mut seen = BTreeSet::new();
        for kind in ALL_HOT {
            for (_, s) in &shards {
                for r in s.hot_records(kind) {
                    if seen.insert(Store::record_hash(r)) {
                        hot_at.entry(kind).or_default().push(hot.len());
                        hot.push(r.clone());
                    }
                }
            }
        }

        let total: usize = shards
            .iter()
            .map(|(_, s)| sealed::Sealed::record_hashes(s).count())
            .sum();
        let repeats = shards.iter().any(|(_, s)| s.duplicate_records() > 0)
            || merged_hashes(&shards).len() < total;

        Union {
            shards,
            uids,
            home,
            own,
            interp,
            library,
            pending,
            hot,
            hot_at,
            repeats,
            index: None,
            derived: OnceLock::new(),
        }
    }

    /// Consult `index` for references no loaded shard holds (`SMY-W500`).
    pub fn with_index(mut self, index: Arc<UidShards>) -> Union<S> {
        self.index = Some(index);
        self
    }

    /// The shards, in the order they are read.
    pub fn shards(&self) -> impl Iterator<Item = &ShardId> {
        self.shards.iter().map(|(id, _)| id)
    }

    /// The shard read first for this uid, if any holds it.
    pub fn shard_of(&self, uid: &Uid) -> Option<&ShardId> {
        let i = self.uids.binary_search(uid).ok()?;
        Some(&self.shards[self.home[i] as usize].0)
    }

    fn home_of(&self, uid: &Uid) -> Option<&S> {
        let i = self.uids.binary_search(uid).ok()?;
        Some(&self.shards[self.home[i] as usize].1)
    }

    fn derived(&self) -> &Derived {
        self.derived.get_or_init(|| {
            let relations: Vec<Relation> = self
                .interp
                .relations
                .values()
                .filter(|r| !self.interp.is_withdrawn(r))
                .cloned()
                .collect();
            // Each unit's edges from the adjacency of the shard read first for it: the core is
            // the same in every shard that holds it, so its edges are too.
            let units: Vec<(Uid, EdgeLists)> = self
                .uids
                .iter()
                .zip(&self.home)
                .map(|(uid, &h)| {
                    let g = self.shards[h as usize].1.adjacency();
                    let mut lists = EdgeLists {
                        deps: Vec::new(),
                        grounds: Vec::new(),
                    };
                    if let Some(id) = g.id(uid) {
                        for e in g.out_edges(id) {
                            let to = *g.uid(e.target).expect("an edge names a node");
                            match e.kind {
                                EdgeKind::Deps => lists.deps.push(to),
                                EdgeKind::Grounds => lists.grounds.push(to),
                                _ => {}
                            }
                        }
                    }
                    (*uid, lists)
                })
                .collect();
            let adjacency = Adjacency::from_parts(&units, &relations);
            drop(units);
            let mut unfounded = BTreeSet::new();
            if self
                .interp
                .relations
                .keys()
                .any(|(k, _, _)| k == RelKind::Retracts.as_str())
            {
                let eff =
                    crate::merge::effective_status(self, crate::merge::RetractionPolicy::Strict);
                unfounded = self
                    .uids
                    .iter()
                    .filter(|u| eff.get(u) == Some(Status::Unfounded))
                    .copied()
                    .collect();
            }
            Derived {
                adjacency,
                unfounded,
            }
        })
    }
}

/// Every shard's record hashes, ascending and without repeats.
fn merged_hashes<S: StoreRead>(shards: &[(ShardId, S)]) -> Vec<&[u8; 32]> {
    let mut all: Vec<&[u8; 32]> = shards
        .iter()
        .flat_map(|(_, s)| sealed::Sealed::record_hashes(s))
        .collect();
    all.sort_unstable();
    all.dedup();
    all
}

/// A unit read from a [`Union`]: one shard's handle, or the union's merged copy.
#[derive(Debug)]
pub enum UnionUnit<'a, R> {
    /// As the one shard that answers for it holds it.
    Shard(R),
    /// Merged from more than one shard.
    Own(&'a Unit),
}

impl<R: Clone> Clone for UnionUnit<'_, R> {
    fn clone(&self) -> Self {
        match self {
            UnionUnit::Shard(r) => UnionUnit::Shard(r.clone()),
            UnionUnit::Own(u) => UnionUnit::Own(u),
        }
    }
}

impl<R: Deref<Target = Unit>> Deref for UnionUnit<'_, R> {
    type Target = Unit;
    fn deref(&self) -> &Unit {
        match self {
            UnionUnit::Shard(r) => r,
            UnionUnit::Own(u) => u,
        }
    }
}

/// A [`Union`]'s units, in ascending uid order.
pub struct UnionUnits<'a, S: StoreRead> {
    union: &'a Union<S>,
    at: usize,
}

impl<'a, S: StoreRead> Iterator for UnionUnits<'a, S> {
    type Item = (&'a Uid, UnionUnit<'a, S::UnitRef<'a>>);
    fn next(&mut self) -> Option<Self::Item> {
        let uid = self.union.uids.get(self.at)?;
        let h = self.union.home[self.at];
        self.at += 1;
        let unit = match self.union.own.get(uid) {
            Some(u) => UnionUnit::Own(u),
            None => UnionUnit::Shard(
                self.union.shards[h as usize]
                    .1
                    .get(uid)
                    .expect("a shard holds the uids it lists"),
            ),
        };
        Some((uid, unit))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.union.uids.len() - self.at;
        (n, Some(n))
    }
}

/// A [`Union`]'s log: each shard's records in shard-id order, a record held twice read once.
pub struct UnionRecords<'a, S: StoreRead> {
    union: &'a Union<S>,
    next: usize,
    current: Option<S::Records<'a>>,
    /// Hashes read so far, when the union has repeats to skip.
    seen: Option<BTreeSet<[u8; 32]>>,
}

impl<'a, S: StoreRead> Iterator for UnionRecords<'a, S> {
    type Item = Cow<'a, Record>;
    fn next(&mut self) -> Option<Cow<'a, Record>> {
        loop {
            if let Some(it) = &mut self.current {
                match it.next() {
                    Some(r) => {
                        if let Some(seen) = &mut self.seen {
                            if !seen.insert(Store::record_hash(&r)) {
                                continue;
                            }
                        }
                        return Some(r);
                    }
                    None => self.current = None,
                }
            }
            let (_, s) = self.union.shards.get(self.next)?;
            self.next += 1;
            self.current = Some(s.records());
        }
    }
}

impl<S: StoreRead> sealed::Sealed for Union<S> {
    fn owned(&self) -> Cow<'_, Store> {
        Cow::Owned(Store::from_records(
            self.records().map(Cow::into_owned).collect(),
        ))
    }
    fn interp(&self) -> &Interpretive {
        &self.interp
    }
    fn pending_attestations(&self) -> &[Attestation] {
        &self.pending
    }
    fn has_record(&self, h: &[u8; 32]) -> bool {
        self.shards
            .iter()
            .any(|(_, s)| sealed::Sealed::has_record(s, h))
    }
    fn record_hashes(&self) -> Box<dyn Iterator<Item = &[u8; 32]> + '_> {
        Box::new(merged_hashes(&self.shards).into_iter())
    }
}

impl<S: StoreRead> StoreRead for Union<S> {
    type UnitRef<'a>
        = UnionUnit<'a, S::UnitRef<'a>>
    where
        Self: 'a;
    type Units<'a>
        = UnionUnits<'a, S>
    where
        Self: 'a;
    type Records<'a>
        = UnionRecords<'a, S>
    where
        Self: 'a;

    fn get(&self, uid: &Uid) -> Option<Self::UnitRef<'_>> {
        match self.own.get(uid) {
            Some(u) => Some(UnionUnit::Own(u)),
            None => self.home_of(uid)?.get(uid).map(UnionUnit::Shard),
        }
    }
    fn contains_uid(&self, uid: &Uid) -> bool {
        self.uids.binary_search(uid).is_ok()
    }
    fn units(&self) -> Self::Units<'_> {
        UnionUnits { union: self, at: 0 }
    }
    fn unit_count(&self) -> usize {
        self.uids.len()
    }
    fn units_in_range(&self, lo: &Uid, hi: &Uid) -> Vec<Uid> {
        if lo > hi {
            return Vec::new();
        }
        let a = self.uids.partition_point(|u| u < lo);
        let b = self.uids.partition_point(|u| u <= hi);
        self.uids[a..b].to_vec()
    }
    fn observed_at(&self, uid: &Uid) -> Option<u64> {
        self.home_of(uid)?.observed_at(uid)
    }
    fn hop_of(&self, uid: &Uid) -> Option<u32> {
        match self.own.get(uid) {
            Some(u) => u.attestations.iter().map(|a| a.hop).max(),
            None => self.home_of(uid)?.hop_of(uid),
        }
    }
    fn units_with_source_prefix(&self, prefix: &str) -> Vec<Uid> {
        let mut out: Vec<Uid> = self
            .shards
            .iter()
            .flat_map(|(_, s)| s.units_with_source_prefix(prefix))
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
    fn units_in_observed_order(&self) -> Vec<Uid> {
        // An observed instant is in the core, so every shard holding a uid gives it the same one.
        let mut timed: Vec<(u64, Uid)> = self
            .shards
            .iter()
            .flat_map(|(_, s)| {
                s.units_in_observed_order()
                    .into_iter()
                    .filter_map(|u| s.observed_at(&u).map(|ms| (ms, u)))
            })
            .collect();
        timed.sort_unstable();
        timed.dedup();
        timed.into_iter().map(|(_, u)| u).collect()
    }

    fn adjacency(&self) -> &Adjacency {
        &self.derived().adjacency
    }
    fn relations(&self) -> Relations<'_> {
        Relations(self.interp.relations.values())
    }
    fn relation_by_id(&self, rid: &Uid) -> Option<&Relation> {
        self.interp.relation_by_id(rid)
    }
    fn relation_by_key(&self, kind: &RelKind, from: &Uid, to: &Uid) -> Option<&Relation> {
        self.interp
            .relations
            .get(&(kind.as_str().to_string(), *from, *to))
    }
    fn threads(&self) -> Threads<'_> {
        Threads(self.interp.threads.values())
    }
    fn views(&self) -> Views<'_> {
        Views(self.interp.views.values())
    }
    fn contentions(&self) -> &[Contention] {
        &self.interp.contentions
    }
    fn withdrawals(&self) -> Withdrawals<'_> {
        Withdrawals(self.interp.withdrawals.values().flatten())
    }
    fn resolutions(&self) -> Resolutions<'_> {
        Resolutions(self.interp.resolutions.iter())
    }
    fn commits(&self) -> Commits<'_> {
        Commits(self.interp.commits.iter())
    }
    fn commits_of(&self, uid: &Uid) -> &BTreeSet<Commit> {
        self.interp
            .commits
            .get(uid)
            .unwrap_or_else(|| empty_commits())
    }
    fn is_withdrawn(&self, rel: &Relation) -> bool {
        self.interp.is_withdrawn(rel)
    }
    fn is_unfounded(&self, uid: &Uid) -> bool {
        self.derived().unfounded.contains(uid)
    }
    fn hot_records(&self, kind: HotKind) -> HotRecords<'_> {
        static NONE: Vec<usize> = Vec::new();
        HotRecords {
            records: &self.hot,
            at: self.hot_at.get(&kind).unwrap_or(&NONE).iter(),
            then: None,
        }
    }
    fn library(&self) -> &StoreLibrary {
        &self.library
    }

    fn records(&self) -> Self::Records<'_> {
        UnionRecords {
            union: self,
            next: 0,
            current: None,
            seen: self.repeats.then(BTreeSet::new),
        }
    }
    fn duplicate_records(&self) -> usize {
        // As `from_records`: a record held twice is one record.
        0
    }
    fn record_set_digest(&self) -> [u8; 32] {
        digest_of_hashes(merged_hashes(&self.shards))
    }
    fn state_hash(&self) -> [u8; 32] {
        state_hash_of(self.units(), &self.interp)
    }

    fn report_dangling(&self, report: &mut Report) {
        let g = self.adjacency();
        for n in g.dangling() {
            let Some(uid) = g.uid(n) else { continue };
            let waits: Vec<&str> = self
                .index
                .as_deref()
                .map(|ix| {
                    ix.shards_of(uid)
                        .filter(|s| !self.shards.iter().any(|(id, _)| id == s))
                        .collect()
                })
                .unwrap_or_default();
            if waits.is_empty() {
                report.push(
                    Diagnostic::new(Code::E060)
                        .with_subject(Subject::Unit(*uid))
                        .with_message("referenced by this store but not present in it"),
                );
            } else {
                report.push(
                    Diagnostic::new(Code::W500)
                        .with_subject(Subject::Unit(*uid))
                        .with_message(format!("waits on shard {}", waits.join(", "))),
                );
            }
        }
    }
}

/// Which shards hold each uid (§3.6's `uid_shard`): a multimap, since one unit extracted into two
/// expressions is one unit by content and lives in both.
///
/// Derived from the shards' uid lists and rebuildable from them; a library keeps one so that a
/// union of some of its shards can tell a reference that waits on another shard (`SMY-W500`)
/// from one that points at nothing (`SMY-E060`) without opening the others.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UidShards {
    /// Shard names, ascending; a pair's second field is a position here.
    names: Vec<ShardId>,
    /// `(uid, shard)`, ascending.
    pairs: Vec<(Uid, u32)>,
}

impl UidShards {
    pub fn new() -> UidShards {
        UidShards::default()
    }

    /// The index of these shards.
    pub fn of<'s, S: StoreRead + ?Sized + 's>(
        shards: impl IntoIterator<Item = (&'s str, &'s S)>,
    ) -> UidShards {
        let mut ix = UidShards::new();
        for (id, s) in shards {
            ix.set(id, all_uids(s));
        }
        ix
    }

    /// Say that `shard` holds exactly `uids`, replacing what was said of it before.
    pub fn set(&mut self, shard: &str, uids: impl IntoIterator<Item = Uid>) {
        let at = match self.names.binary_search_by(|n| n.as_str().cmp(shard)) {
            Ok(i) => {
                let i = i as u32;
                self.pairs.retain(|(_, s)| *s != i);
                i
            }
            Err(i) => {
                // Positions after the new name move up by one.
                for (_, s) in &mut self.pairs {
                    if *s >= i as u32 {
                        *s += 1;
                    }
                }
                self.names.insert(i, shard.to_string());
                i as u32
            }
        };
        self.pairs.extend(uids.into_iter().map(|u| (u, at)));
        self.pairs.sort_unstable();
        self.pairs.dedup();
    }

    /// Forget a shard.
    pub fn remove(&mut self, shard: &str) {
        if let Ok(i) = self.names.binary_search_by(|n| n.as_str().cmp(shard)) {
            let i = i as u32;
            self.pairs.retain(|(_, s)| *s != i);
            for (_, s) in &mut self.pairs {
                if *s > i {
                    *s -= 1;
                }
            }
            self.names.remove(i as usize);
        }
    }

    /// The shards that hold this uid, in name order.
    pub fn shards_of<'a>(&'a self, uid: &Uid) -> impl Iterator<Item = &'a str> + 'a {
        let a = self.pairs.partition_point(|(u, _)| u < uid);
        let b = self.pairs.partition_point(|(u, _)| u <= uid);
        self.pairs[a..b]
            .iter()
            .map(|(_, s)| self.names[*s as usize].as_str())
    }

    /// The uids this shard holds, ascending.
    pub fn uids_of(&self, shard: &str) -> Vec<Uid> {
        match self.names.binary_search_by(|n| n.as_str().cmp(shard)) {
            Ok(i) => self
                .pairs
                .iter()
                .filter(|(_, s)| *s == i as u32)
                .map(|(u, _)| *u)
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Add these uids to what `shard` holds.
    pub fn extend(&mut self, shard: &str, uids: impl IntoIterator<Item = Uid>) {
        let mut all = self.uids_of(shard);
        all.extend(uids);
        self.set(shard, all);
    }

    /// Every shard named, in name order.
    pub fn shard_names(&self) -> impl Iterator<Item = &str> {
        self.names.iter().map(String::as_str)
    }

    /// How many `(uid, shard)` pairs it holds.
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The pairs, for a caller that persists the index: `(uid, shard name)`, ascending.
    pub fn pairs(&self) -> impl Iterator<Item = (&Uid, &str)> {
        self.pairs
            .iter()
            .map(|(u, s)| (u, self.names[*s as usize].as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{canonical_uid, KernelType, RelKind, UnitCoreBuilder};

    fn unit(gist: &str) -> smysl_core::UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .unwrap()
    }

    /// The step's exit (SMYSL-2.8 §6, P13c.1): a cross-shard `quotes` edge waits on the shard
    /// that holds its target, and stops waiting when that shard is loaded.
    #[test]
    fn a_reference_into_an_unloaded_shard_waits_and_resolves_when_it_is_loaded() {
        let (a, b) = (unit("the reply"), unit("the message it quotes"));
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let quotes = Relation::new(RelKind::Extension("x.text/quotes".into()), ua, ub);
        let replies = Store::from_records(vec![Record::Unit(a), Record::Relation(quotes)]);
        let messages = Store::from_records(vec![Record::Unit(b)]);
        let index = UidShards::of([("chat/replies", &replies), ("chat/messages", &messages)]);

        let codes = |u: &Union<&Store>| {
            let mut r = Report::new();
            u.report_dangling(&mut r);
            r.diagnostics
                .iter()
                .map(|d| (d.code, d.message.clone()))
                .collect::<Vec<_>>()
        };

        let index = Arc::new(index);
        let alone = Union::new([("chat/replies".to_string(), &replies)]).with_index(index.clone());
        let got = codes(&alone);
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].0, Code::W500);
        assert!(got[0].1.contains("chat/messages"), "{got:?}");

        // Without the index nothing says where the uid lives: an ordinary dangling reference.
        let blind = Union::new([("chat/replies".to_string(), &replies)]);
        assert_eq!(codes(&blind)[0].0, Code::E060);

        // Loaded: nothing dangles, and nothing was written to either shard.
        let both = Union::new([
            ("chat/replies".to_string(), &replies),
            ("chat/messages".to_string(), &messages),
        ])
        .with_index(index.clone());
        assert!(codes(&both).is_empty());
        assert!(both.adjacency().dangling().is_empty());
        assert_eq!(replies.len() + messages.len(), 3);

        // A uid the index places nowhere still points at nothing.
        let mut r = Report::new();
        let lost = Union::new([("chat/messages".to_string(), &messages)]).with_index(index);
        lost.report_dangling(&mut r);
        assert!(
            r.diagnostics.is_empty(),
            "the messages shard references nothing"
        );
    }

    #[test]
    fn the_index_is_a_multimap_and_forgets_a_shard_whole() {
        let a = unit("held twice");
        let ua = canonical_uid(&a);
        let s = Store::from_records(vec![Record::Unit(a)]);
        let mut ix = UidShards::of([("b", &s), ("a", &s)]);
        assert_eq!(ix.shards_of(&ua).collect::<Vec<_>>(), ["a", "b"]);
        ix.set("c", [ua]);
        ix.remove("a");
        assert_eq!(ix.shards_of(&ua).collect::<Vec<_>>(), ["b", "c"]);
        assert_eq!(ix.shard_names().collect::<Vec<_>>(), ["b", "c"]);
        ix.set("b", []);
        assert_eq!(ix.shards_of(&ua).collect::<Vec<_>>(), ["c"]);
        assert_eq!(ix.len(), 1);
    }
}
