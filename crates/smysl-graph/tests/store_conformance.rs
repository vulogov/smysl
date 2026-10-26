//! The store conformance suite (SMYSL-2.8 §5.1; TX-P13a step 4).
//!
//! Every implementation of `StoreRead` built from the same record set must answer every method
//! the same way, byte for byte where the answer is bytes. At this step there are two:
//!
//! 1. `Store::from_records(R)` — the oracle;
//! 2. `Store::open` on a log of R;
//! 3. with feature `store-redb`, `RedbStore` on that log with its core cache off, so every
//!    `get` reads the log;
//! 4. the same with a cache large enough for everything, opened a second time so that it reads
//!    the store the first open built rather than building one, and once more after the log grew
//!    so that it folds in the tail;
//! 5. with `store-redb`, the log written by appends through `RedbShard` and through
//!    `AnyStore`'s append, against the in-memory store given the same appends.
//!
//! and, for each, the store behind every pointer the trait is implemented for (`&`, `Box`, `Rc`,
//! `Arc`), which is what keeps `check(&arc_store, …)` compiling once `check` is generic. Later
//! steps add the paged store, the union of shards and the overlay to the same list.
//!
//! Inputs: the F1–F13 corpora, the `check` conformance fixtures, all of them in one store, and a
//! synthetic store with withdrawals, a retraction, commitments and a contention so that the
//! lifecycle methods have something to say. Probes: every uid any record names, plus a hundred
//! seeded uids no record names.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, hash_bytes, to_cbor, to_cbor_seq, AgentId, Commit, Commitment, Contention,
    ContentionId, Detected, DetectionKind, Hlc, KernelType, Record, RelKind, Relation,
    ResolutionTarget, Status, Uid, UidPrefix, UnitCoreBuilder, Withdrawal,
};
use smysl_graph::{AnyStore, EdgeSet, HotKind, Overlay, Store, StoreRead};

fn fixtures(dir: &str) -> Vec<(String, Vec<Record>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
    let mut out = Vec::new();
    let mut paths: Vec<_> = std::fs::read_dir(root.join(dir))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "smy"))
        .collect();
    paths.sort();
    for p in paths {
        let src = std::fs::read_to_string(&p).unwrap();
        let Ok(parsed) = parse_surface(&src) else {
            continue;
        };
        out.push((
            p.file_name().unwrap().to_string_lossy().into_owned(),
            parsed.records,
        ));
    }
    out
}

fn agent(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

/// Withdrawals, a retraction, commitments and a contention, so the lifecycle methods answer.
fn lifecycle() -> Vec<Record> {
    let unit = |gist: &str, status: Status| {
        UnitCoreBuilder::new(KernelType::Claim, gist, status)
            .build()
            .unwrap()
    };
    let a = unit("the bridge holds", Status::Speculative);
    let b = UnitCoreBuilder::new(KernelType::Claim, "the bridge fails", Status::Inferred)
        .grounds(vec![canonical_uid(&a)])
        .build()
        .unwrap();
    let c = unit("the load is light", Status::Speculative);
    let d = unit("the river is high", Status::Speculative);
    let (ua, ub, uc, ud) = (
        canonical_uid(&a),
        canonical_uid(&b),
        canonical_uid(&c),
        canonical_uid(&d),
    );
    let rebuts = Relation::new(RelKind::Rebuts, uc, ub);
    let backs = Relation::new(RelKind::Backs, ud, ua);
    let withdrawn = Withdrawal::new(
        backs.uid(),
        agent("tool:conf"),
        Hlc::new(5, 0, agent("tool:conf")),
    );
    let mut records = vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Unit(c),
        Record::Unit(d),
        Record::Relation(rebuts),
        Record::Relation(backs),
        Record::Relation(Relation::new(RelKind::Retracts, ud, ua)),
        Record::Withdrawal(withdrawn),
        Record::Commit(Commit::new(
            uc,
            Commitment::Floated,
            agent("tool:conf"),
            Hlc::new(1, 0, agent("tool:conf")),
        )),
        Record::Commit(Commit::new(
            uc,
            Commitment::Committed,
            agent("tool:other"),
            Hlc::new(2, 0, agent("tool:other")),
        )),
    ];
    records.push(Record::Contention(Contention::new(
        ContentionId::new("x/c1").unwrap(),
        ub,
        vec![uc],
        Detected::new(
            DetectionKind::LiveRebuttal,
            Hlc::new(3, 0, agent("tool:conf")),
        ),
    )));
    records
}

/// Every uid a store mentions, plus a hundred no record names.
fn probes<S: StoreRead + ?Sized>(s: &S) -> Vec<Uid> {
    let mut out: BTreeSet<Uid> = BTreeSet::new();
    let g = s.adjacency();
    for n in 0..g.len() as u32 {
        if let Some(u) = g.uid(n) {
            out.insert(*u);
        }
    }
    for r in s.relations() {
        out.insert(r.uid());
    }
    for i in 0..100u32 {
        out.insert(Uid::from_bytes(hash_bytes(&i.to_be_bytes())));
    }
    out.into_iter().collect()
}

/// Every answer a store gives, as lines.
fn observe<S: StoreRead + ?Sized>(s: &S, probes: &[Uid]) -> Vec<String> {
    let mut out = Vec::new();
    let mut say = |k: &str, v: String| out.push(format!("{k}: {v}"));
    say("unit_count", s.unit_count().to_string());
    say(
        "units",
        format!(
            "{:?}",
            s.units()
                .map(|(u, x)| (*u, hash_bytes(&to_cbor(&Record::Unit(x.core.clone())))))
                .collect::<Vec<_>>()
        ),
    );
    say("adjacency", format!("{:?}", s.adjacency()));
    say(
        "relations",
        format!("{:?}", s.relations().collect::<Vec<_>>()),
    );
    say("threads", format!("{:?}", s.threads().collect::<Vec<_>>()));
    say("views", format!("{:?}", s.views().collect::<Vec<_>>()));
    say("contentions", format!("{:?}", s.contentions()));
    say(
        "withdrawals",
        format!("{:?}", s.withdrawals().collect::<Vec<_>>()),
    );
    say(
        "resolutions",
        format!("{:?}", s.resolutions().collect::<Vec<_>>()),
    );
    say("commits", format!("{:?}", s.commits().collect::<Vec<_>>()));
    for k in [
        HotKind::SchemaDecls,
        HotKind::LabelBindings,
        HotKind::PackInfos,
        HotKind::Unknown,
    ] {
        say(
            &format!("hot {k:?}"),
            format!(
                "{:?}",
                s.hot_records(k)
                    .map(|r| hash_bytes(&to_cbor(r)))
                    .collect::<Vec<_>>()
            ),
        );
    }
    // The library layer whole (its `Debug` is every map it holds), and the derived answers.
    let lib = s.library();
    say("library", format!("{lib:?}"));
    for alias in lib.aliases() {
        say(&format!("heads {alias}"), format!("{:?}", lib.heads(alias)));
    }
    for tid in lib.parts().keys() {
        say(
            &format!("units_with_tid {tid:?}"),
            format!("{:?}", lib.units_with_tid(tid)),
        );
    }
    let log: Vec<[u8; 32]> = s.records().map(|r| hash_bytes(&to_cbor(&r))).collect();
    say("records", format!("{log:?}"));
    say("duplicate_records", s.duplicate_records().to_string());
    say("record_set_digest", format!("{:?}", s.record_set_digest()));
    say("state_hash", format!("{:?}", s.state_hash()));
    say(
        "units_in_observed_order",
        format!("{:?}", s.units_in_observed_order()),
    );
    say("hops", format!("{:?} {:?}", s.hops(), s.latest_hop()));
    say("attested_units", format!("{:?}", s.attested_units()));
    say("open_contentions", format!("{:?}", s.open_contentions()));
    say(
        "contention_status",
        format!(
            "{:?}",
            s.contentions()
                .iter()
                .map(|c| s.contention_status(c))
                .collect::<Vec<_>>()
        ),
    );
    for level in [
        Commitment::Floated,
        Commitment::Committed,
        Commitment::Canonical,
    ] {
        say(
            &format!("units_at_commitment {level:?}"),
            format!("{:?}", s.units_at_commitment(level)),
        );
    }
    let mut dangling = smysl_core::diag::Report::new();
    s.report_dangling(&mut dangling);
    say("report_dangling", format!("{:?}", dangling.diagnostics));
    for kind in [
        RelKind::Rebuts,
        RelKind::Backs,
        RelKind::Retracts,
        RelKind::Supersedes,
        RelKind::Elaborates,
    ] {
        say(
            &format!("relations_of_kind {kind:?}"),
            format!("{:?}", s.relations_of_kind(&kind)),
        );
    }
    for r in s.relations() {
        say(
            &format!("relation {}", r.uid()),
            format!(
                "withdrawn {} live {} by_id {} has {}",
                s.is_withdrawn(r),
                s.is_live_rebuttal(r),
                s.relation_by_id(&r.uid()).is_some(),
                s.has_relation(&r.kind, &r.from, &r.to)
            ),
        );
    }
    for prefix in ["/", "f", "doc", "bench"] {
        say(
            &format!("units_with_source_prefix {prefix}"),
            format!("{:?}", s.units_with_source_prefix(prefix)),
        );
    }
    let all = EdgeSet::all();
    for u in probes {
        say(
            &format!("uid {u}"),
            format!(
                "get {:?} contains {} observed {:?} hop {:?} unfounded {} rebuttals {:?} \
                 supports {:?} commits {:?} commitment {:?} attestations {:?} by {:?} agree {} \
                 resolved {}",
                s.get(u).map(|x| x.attestations.len()),
                s.contains_uid(u),
                s.observed_at(u),
                s.hop_of(u),
                s.is_unfounded(u),
                s.rebuttals_of(u),
                s.supports_of(u, &all),
                s.commits_of(u),
                s.commitment_of(u),
                s.attestations_of(u),
                s.attested_by(u),
                s.agreement(u, 2),
                s.is_resolved(&ResolutionTarget::Relation(*u)),
            ),
        );
        let text = u.to_string();
        for n in [29, 40] {
            if let Ok(p) = UidPrefix::parse(&text[..n.min(text.len())]) {
                say(
                    &format!("prefix {n} {u}"),
                    format!("{:?} {:?}", s.matching_prefix(&p), s.resolve_prefix(&p)),
                );
            }
        }
        let next = {
            let mut b = *u.as_bytes();
            b[31] = b[31].wrapping_add(9);
            Uid::from_bytes(b)
        };
        say(
            &format!("range {u}"),
            format!("{:?}", s.units_in_range(u, &next)),
        );
    }
    out
}

fn implementations(name: &str, records: &[Record]) -> Vec<(String, Store)> {
    let oracle = Store::from_records(records.to_vec());
    let dir = std::env::temp_dir().join(format!(
        "smysl-conformance-{}-{}",
        std::process::id(),
        name.replace(['/', '.'], "-")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.smy");
    // The log of R: what `from_records` holds, in its order, so duplicates do not differ.
    let log: Vec<Record> = oracle.iter().cloned().collect();
    std::fs::write(&path, to_cbor_seq(&log)).unwrap();
    let opened = Store::open(&path).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    vec![("from_records".into(), oracle), ("open".into(), opened)]
}

fn conform(name: &str, records: &[Record]) {
    let impls = implementations(name, records);
    let (_, oracle) = &impls[0];
    let p = probes(oracle);
    let want = observe(oracle, &p);
    let check = |label: &str, got: Vec<String>| {
        assert_eq!(got.len(), want.len(), "{name}: {label}: answer count");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "{name}: {label} disagrees with the oracle");
        }
    };
    for (label, s) in &impls {
        check(label, observe(s, &p));
        check(&format!("&{label}"), observe(&s, &p));
        check(&format!("Box<{label}>"), observe(&Box::new(s.clone()), &p));
        check(&format!("Rc<{label}>"), observe(&Rc::new(s.clone()), &p));
        check(&format!("Arc<{label}>"), observe(&Arc::new(s.clone()), &p));
        check(
            &format!("AnyStore<{label}>"),
            observe(&AnyStore::from(s.clone()), &p),
        );
    }

    #[cfg(feature = "store-redb")]
    for (label, got) in redb_answers(name, oracle, &p) {
        check(&label, got);
    }

    // Implementation 7, `Overlay` (TX-P13a step 10): the log split at four points, each half
    // laid over a store of the other — which is `from_records` of the whole, in that order.
    let log: Vec<Record> = oracle.iter().cloned().collect();
    let n = log.len();
    for k in [0, n / 3, 2 * n / 3, n] {
        let base = Store::from_records(log[..k].to_vec());
        let over = Overlay::new(&base, log[k..].to_vec());
        check(&format!("Overlay at {k}/{n}"), observe(&over, &p));
        check(&format!("&Overlay at {k}/{n}"), observe(&&over, &p));
    }
    // Over an opened log, and over another overlay.
    let (_, opened) = &impls[1];
    check(
        "Overlay over open",
        observe(&Overlay::new(opened, Vec::new()), &p),
    );
    let first = Store::from_records(log[..n / 3].to_vec());
    let inner = Overlay::new(&first, log[n / 3..2 * n / 3].to_vec());
    let outer = Overlay::new(&inner, log[2 * n / 3..].to_vec());
    check("Overlay of an Overlay", observe(&outer, &p));

    // Interleaved rather than split: attestations land before their units, and a relation's
    // attestations arrive on the other side from the relation. The oracle is the store of the
    // same records in the overlay's order.
    let (even, odd): (Vec<_>, Vec<_>) = log
        .iter()
        .cloned()
        .enumerate()
        .partition(|(i, _)| i % 2 == 0);
    let even: Vec<Record> = even.into_iter().map(|(_, r)| r).collect();
    let odd: Vec<Record> = odd.into_iter().map(|(_, r)| r).collect();
    let mut order = even.clone();
    order.extend(odd.iter().cloned());
    let reordered = Store::from_records(order);
    let want_reordered = observe(&reordered, &p);
    let base = Store::from_records(even);
    let got = observe(&Overlay::new(&base, odd), &p);
    assert_eq!(
        got.len(),
        want_reordered.len(),
        "{name}: interleaved: answer count"
    );
    for (g, w) in got.iter().zip(&want_reordered) {
        assert_eq!(
            g, w,
            "{name}: an interleaved Overlay disagrees with its store"
        );
    }

    union_conforms(name, &log, oracle, &p);
}

/// A small seeded generator, so a partition is the same on every run.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// `log` dealt into `k` shards by a seed. About one unit in eight is dealt twice, so two shards
/// hold it — one unit by content, and the union must read it once with both shards'
/// attestations.
fn partition(log: &[Record], k: usize, seed: u64) -> Vec<Vec<Record>> {
    let mut rng = Seeded(seed | 1);
    let mut shards = vec![Vec::new(); k];
    for r in log {
        let a = rng.below(k);
        shards[a].push(r.clone());
        if k > 1 && matches!(r, Record::Unit(_)) && rng.below(8) == 0 {
            let b = (a + 1 + rng.below(k - 1)) % k;
            shards[b].push(r.clone());
        }
    }
    shards
}

fn permutations(n: usize) -> Vec<Vec<usize>> {
    if n == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for rest in permutations(n - 1) {
        for at in 0..=rest.len() {
            let mut p = rest.clone();
            p.insert(at, n - 1);
            out.push(p);
        }
    }
    out
}

/// Implementation 6 (TX-P13c step 1): `Union` of the log dealt into k = 1, 2 and 7 shards, in
/// every order for k ≤ 3 and five seeded orders for k = 7.
///
/// The oracle for a partition is `from_records` of the shards' records concatenated in shard-id
/// order: what a union is defined to answer. The answers that do not depend on order at all —
/// the record-set digest, every unit and its attestations, the relations — must also be the
/// whole log's.
fn union_conforms(name: &str, log: &[Record], whole: &Store, p: &[Uid]) {
    use smysl_graph::Union;

    for (k, seed) in [(1usize, 11u64), (2, 12), (2, 13), (7, 14)] {
        let parts = partition(log, k, seed);
        let ids: Vec<String> = (0..k).map(|i| format!("s{i}")).collect();
        let stores: Vec<Store> = parts.iter().cloned().map(Store::from_records).collect();
        let concat: Vec<Record> = parts.iter().flatten().cloned().collect();
        let want_store = Store::from_records(concat);
        let want = observe(&want_store, p);
        assert_eq!(
            want_store.record_set_digest(),
            whole.record_set_digest(),
            "{name}: k={k}: the partition lost or gained a record"
        );

        let orders: Vec<Vec<usize>> = if k <= 3 {
            permutations(k)
        } else {
            let mut rng = Seeded(seed);
            (0..5)
                .map(|_| {
                    let mut o: Vec<usize> = (0..k).collect();
                    for i in (1..k).rev() {
                        o.swap(i, rng.below(i + 1));
                    }
                    o
                })
                .collect()
        };
        for order in orders {
            let u = Union::new(order.iter().map(|&i| (ids[i].clone(), &stores[i])));
            let got = observe(&u, p);
            assert_eq!(
                got.len(),
                want.len(),
                "{name}: k={k} {order:?}: answer count"
            );
            for (g, w) in got.iter().zip(&want) {
                assert_eq!(g, w, "{name}: Union k={k} order {order:?} disagrees");
            }
            let units = |s: &dyn Fn() -> Vec<(Uid, usize)>| s();
            assert_eq!(
                units(&|| u
                    .units()
                    .map(|(id, x)| (*id, x.attestations.len()))
                    .collect()),
                units(&|| whole
                    .units()
                    .map(|(id, x)| (*id, x.attestations.len()))
                    .collect()),
                "{name}: k={k}: units and their attestations are the whole log's"
            );
            assert_eq!(
                u.relations().collect::<Vec<_>>(),
                whole.relations().collect::<Vec<_>>(),
                "{name}: k={k}: relations are the whole log's"
            );
        }

        // Over opened logs, and with one shard's units held twice within it.
        let dir = std::env::temp_dir().join(format!(
            "smysl-conformance-union-{}-{}-{k}",
            std::process::id(),
            name.replace(['/', '.', ' '], "-")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let opened: Vec<Store> = parts
            .iter()
            .enumerate()
            .map(|(i, recs)| {
                let path = dir.join(format!("s{i}.smy"));
                let mut bytes = to_cbor_seq(recs);
                if i == 0 {
                    bytes.extend(to_cbor_seq(&recs[..recs.len().min(3)]));
                }
                std::fs::write(&path, bytes).unwrap();
                Store::open(&path).unwrap()
            })
            .collect();
        let _ = std::fs::remove_dir_all(&dir);
        let u = Union::new(ids.iter().cloned().zip(opened.iter()));
        let got = observe(&u, p);
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "{name}: Union of opened logs, k={k}, disagrees");
        }
        // A union is a store like any other: an overlay lies over it, and `AnyStore` holds one.
        let over = Overlay::new(&u, Vec::new());
        for (g, w) in observe(&over, p).iter().zip(&want) {
            assert_eq!(g, w, "{name}: Overlay over a Union, k={k}, disagrees");
        }
        let any = AnyStore::shards(
            ids.iter()
                .cloned()
                .zip(stores.iter().cloned().map(AnyStore::from)),
            None,
        );
        for (g, w) in observe(&any, p).iter().zip(&want) {
            assert_eq!(g, w, "{name}: AnyStore::shards, k={k}, disagrees");
        }

        #[cfg(feature = "store-redb")]
        {
            use smysl_graph::redb::{RedbOptions, RedbStore};
            let dir = scratch(name, &format!("union-redb-{k}"));
            let mut off = RedbOptions::default();
            off.core_cache_bytes = 0;
            let redb: Vec<RedbStore> = parts
                .iter()
                .enumerate()
                .map(|(i, recs)| {
                    let path = dir.join(format!("s{i}.smy"));
                    std::fs::write(&path, to_cbor_seq(recs)).unwrap();
                    RedbStore::open(&path, off.clone()).unwrap().0
                })
                .collect();
            let u = Union::new(ids.iter().cloned().zip(redb.iter()));
            for (g, w) in observe(&u, p).iter().zip(&want) {
                assert_eq!(g, w, "{name}: Union of RedbStores, k={k}, disagrees");
            }
            drop(u);
            drop(redb);

            // Implementation 8 (TX-P13c step 3): the shards as one unit store, not a union of
            // stores — with the first shard holding some of its records twice, which a store of
            // several logs reads once, as `from_records` does.
            let logs: Vec<(String, std::path::PathBuf)> = ids
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    let path = dir.join(format!("m{i}.smy"));
                    let mut bytes = to_cbor_seq(&parts[i]);
                    if i == 0 {
                        bytes.extend(to_cbor_seq(&parts[i][..parts[i].len().min(3)]));
                    }
                    std::fs::write(&path, bytes).unwrap();
                    (id.clone(), path)
                })
                .collect();
            let at = dir.join("library.redb");
            let (one, _) = RedbStore::open_shards(&at, &logs, None, off.clone()).unwrap();
            for (g, w) in observe(&one, p).iter().zip(&want) {
                assert_eq!(g, w, "{name}: one RedbStore over k={k} logs disagrees");
            }
            assert_eq!(one.record_set_digest(), want_store.record_set_digest());
            assert_eq!(one.log_hash(), want_store.log_hash());
            drop(one);
            // Reopened, with the cache on: nothing to fold, the same answers.
            let (again, report) =
                RedbStore::open_shards(&at, &logs, None, RedbOptions::default()).unwrap();
            assert_eq!(report.replayed, 0, "{name}: k={k}: reopened and folded");
            for (g, w) in observe(&again, p).iter().zip(&want) {
                assert_eq!(g, w, "{name}: one RedbStore reopened, k={k}, disagrees");
            }
            drop(again);

            // Some of the shards: `from_records` of theirs alone, with a reference into one of
            // the others waiting on it — what a union of them given the shards' index answers.
            let some: Vec<String> = ids.iter().step_by(2).cloned().collect();
            let index = std::sync::Arc::new(smysl_graph::UidShards::of(
                ids.iter().map(String::as_str).zip(stores.iter()),
            ));
            let want_some = observe(
                &Union::new(ids.iter().cloned().zip(stores.iter()).step_by(2)).with_index(index),
                p,
            );
            let (sub, _) = RedbStore::open_shards(&at, &logs, Some(&some), off.clone()).unwrap();
            for (g, w) in observe(&sub, p).iter().zip(&want_some) {
                assert_eq!(
                    g, w,
                    "{name}: one RedbStore opened for {some:?} of k={k} disagrees"
                );
            }
            drop(sub);

            // A shard that grows in the middle of the order is folded on its own, and the store
            // answers for the new whole.
            if k > 1 {
                let more = &parts[0][..parts[0].len().min(2)];
                let mut bytes = std::fs::read(&logs[k - 1].1).unwrap();
                bytes.extend(to_cbor_seq(more));
                std::fs::write(&logs[k - 1].1, bytes).unwrap();
                let mut grown: Vec<Record> = Vec::new();
                for (i, part) in parts.iter().enumerate() {
                    grown.extend(part.iter().cloned());
                    if i == k - 1 {
                        grown.extend(more.iter().cloned());
                    }
                }
                let want_grown = observe(&Store::from_records(grown), p);
                let (g2, report) = RedbStore::open_shards(&at, &logs, None, off.clone()).unwrap();
                assert!(
                    !report.index_rebuilt,
                    "{name}: k={k}: a grown shard rebuilt"
                );
                for (g, w) in observe(&g2, p).iter().zip(&want_grown) {
                    assert_eq!(
                        g, w,
                        "{name}: one RedbStore after a shard grew, k={k}, disagrees"
                    );
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// A fresh directory for one fixture's logs.
#[cfg(feature = "store-redb")]
fn scratch(name: &str, what: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smysl-conformance-{what}-{}-{}",
        std::process::id(),
        name.replace(['/', '.', ' '], "-")
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Implementations 3 and 4 (TX-P13b step 1), and what lies over them, observed.
#[cfg(feature = "store-redb")]
fn redb_answers(name: &str, oracle: &Store, p: &[Uid]) -> Vec<(String, Vec<String>)> {
    use smysl_graph::redb::{RedbOptions, RedbStore};

    let dir = scratch(name, "redb");
    let path = dir.join("store.smy");
    let log: Vec<Record> = oracle.iter().cloned().collect();
    let n = log.len();
    let mut out = Vec::new();

    // The log in two writes, so the second open has a tail to fold in.
    let k = n / 2;
    std::fs::write(&path, to_cbor_seq(&log[..k])).unwrap();
    let mut off = RedbOptions::default();
    off.core_cache_bytes = 0;
    {
        let (s, report) = RedbStore::open(&path, off.clone()).unwrap();
        assert!(report.index_rebuilt, "{name}: the first open builds");
        assert_eq!(report.replayed as usize, k);
        assert_eq!(s.len(), k);
    }
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.extend(to_cbor_seq(&log[k..]));
    std::fs::write(&path, &bytes).unwrap();
    {
        let (s, report) = RedbStore::open(&path, off.clone()).unwrap();
        assert!(
            !report.index_rebuilt,
            "{name}: a longer log is caught up, not rebuilt"
        );
        assert_eq!(report.replayed as usize, n - k, "{name}: exactly the tail");
        out.push(("RedbStore, no cache".into(), observe(&s, p)));
        out.push(("&RedbStore".into(), observe(&&s, p)));
        out.push((
            "AnyStore<RedbStore>".into(),
            observe(&AnyStore::Redb(s.clone()), p),
        ));
        out.push((
            "Overlay over RedbStore".into(),
            observe(&Overlay::new(&s, Vec::new()), p),
        ));
        out.push((
            "RedbStore into memory".into(),
            observe(&AnyStore::Redb(s.clone()).into_memory(), p),
        ));
        assert_eq!(
            s.cache_stats().hits,
            0,
            "{name}: a zero budget caches nothing"
        );
    }
    {
        let (s, report) = RedbStore::open(&path, RedbOptions::default()).unwrap();
        assert!(
            !report.index_rebuilt && report.replayed == 0,
            "{name}: current"
        );
        out.push(("RedbStore, cached".into(), observe(&s, p)));
        out.push(("RedbStore, cached, again".into(), observe(&s, p)));
    }

    // An overlay whose base is on disk: the first half there, the second laid over it.
    let half = dir.join("half.smy");
    std::fs::write(&half, to_cbor_seq(&log[..k])).unwrap();
    {
        let (base, _) = RedbStore::open(&half, off).unwrap();
        out.push((
            format!("Overlay at {k}/{n} over RedbStore"),
            observe(&Overlay::new(&base, log[k..].to_vec()), p),
        ));
    }
    redb_writes(name, &log, p, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Implementation 5 (TX-P13b step 2): the log written through the unit store's writer, and
/// through `AnyStore`'s append, against the in-memory store given the same appends.
///
/// Not against the oracle: an append drops a record the store already holds, so a log written
/// by appends has no repeats where the oracle's may. What must agree is every answer and every
/// append report, call by call, and a reopen of what was written.
#[cfg(feature = "store-redb")]
fn redb_writes(name: &str, log: &[Record], p: &[Uid], dir: &std::path::Path) {
    use smysl_graph::redb::{RedbOptions, RedbShard, RedbStore};
    use smysl_graph::StoreWrite;

    let n = log.len();
    // Three appends, the second carrying repeats of the first, so the reports have duplicates.
    let mut second = log[n / 3..2 * n / 3].to_vec();
    second.extend(log[..(n / 3).min(10)].iter().cloned());
    let chunks = [log[..n / 3].to_vec(), second, log[2 * n / 3..].to_vec()];

    let mut memory = Store::default();
    let shard_log = dir.join("shard.smy");
    let any_log = dir.join("any.smy");
    let mut off = RedbOptions::default();
    off.core_cache_bytes = 0;
    let (mut shard, _) = RedbShard::open_rw(&shard_log, off.clone()).unwrap();
    let (mut any, _) = AnyStore::open_redb(&any_log, RedbOptions::default()).unwrap();
    assert!(
        any.as_memory().is_none(),
        "{name}: a unit store, not the fallback"
    );
    for (i, chunk) in chunks.iter().enumerate() {
        let want = memory.append(chunk).unwrap();
        assert_eq!(shard.append(chunk).unwrap(), want, "{name}: append {i}");
        assert_eq!(
            StoreWrite::append(&mut any, chunk).unwrap(),
            want,
            "{name}: append {i}"
        );
        let expect = observe(&memory, p);
        for (label, got) in [
            ("RedbShard", observe(&shard.reader().unwrap(), p)),
            ("AnyStore<RedbStore> appended", observe(&any, p)),
        ] {
            assert_eq!(got, expect, "{name}: {label} after append {i}");
        }
    }
    assert_eq!(shard.log_hash(), memory.log_hash(), "{name}: the same log");
    drop(shard);
    drop(any);
    assert_eq!(std::fs::read(&shard_log).unwrap(), memory.log_bytes());
    assert_eq!(std::fs::read(&any_log).unwrap(), memory.log_bytes());
    let (reopened, report) = RedbStore::open(&shard_log, off).unwrap();
    assert!(
        !report.index_rebuilt && report.replayed == 0,
        "{name}: what the writer wrote, the store describes"
    );
    assert_eq!(
        observe(&reopened, p),
        observe(&memory, p),
        "{name}: reopened"
    );
}

#[test]
fn every_implementation_answers_as_the_oracle_on_the_corpora() {
    let corpora = fixtures("corpus");
    assert!(corpora.len() >= 13, "{} corpora", corpora.len());
    for (name, records) in &corpora {
        conform(name, records);
    }
}

#[test]
fn every_implementation_answers_as_the_oracle_on_the_check_fixtures() {
    for (name, records) in fixtures("conformance/check") {
        conform(&name, &records);
    }
}

#[test]
fn every_implementation_answers_as_the_oracle_on_everything_at_once() {
    let mut all: Vec<Record> = fixtures("corpus")
        .into_iter()
        .chain(fixtures("conformance/check"))
        .flat_map(|(_, r)| r)
        .collect();
    all.extend(lifecycle());
    conform("everything", &all);
}

#[test]
fn the_lifecycle_store_has_something_to_say() {
    let s = Store::from_records(lifecycle());
    assert_eq!(s.withdrawals().count(), 1);
    assert_eq!(StoreRead::contentions(&s).len(), 1);
    assert!(s.units().any(|(u, _)| StoreRead::is_unfounded(&s, u)));
    assert!(StoreRead::units_at_commitment(&s, Commitment::Committed).len() == 1);
    conform("lifecycle", &lifecycle());
}

/// Two manifests under one alias, the second superseding the first; a unit sourced from a part;
/// a redaction and a dating. What `StoreRead::library` answers, with something in it.
fn library_records() -> Vec<Record> {
    use smysl_core::ids::{LangTag, Rdid, Tid};
    use smysl_core::types::library::{Dating, DatingTarget, Redaction};
    use smysl_core::types::{Carry, Manifest, PartEntry, SourceKind, SourceRef};

    let tid = |b: u8| Tid::from_normalised_bytes(&[b]);
    let manifest = |part: u8| {
        let mut m = Manifest::new(
            "bible",
            LangTag::new("en").unwrap(),
            "txt/1",
            "public-domain",
            "smysl/parts/1 level=line min=65536 max=4194304",
        );
        m.carry = Carry::Text;
        m.parts.push(PartEntry::new(
            tid(part),
            128,
            [part; 32],
            Rdid::from_bytes([part; 32]),
        ));
        m
    };
    let first = manifest(1);
    let mut second = manifest(2);
    second.supersedes = Some(first.mid());
    let sourced = UnitCoreBuilder::new(KernelType::Prose, "In the beginning.", Status::Cited)
        .source(SourceRef::new(
            SourceKind::Doc,
            format!("{}#Gen.1.1", tid(2).canonical()),
        ))
        .build()
        .unwrap();
    let u = canonical_uid(&sourced);
    vec![
        Record::Manifest(first),
        Record::Manifest(second),
        Record::Unit(sourced),
        Record::Redaction(Redaction::new(
            tid(1),
            agent("tool:conf"),
            Hlc::new(4, 0, agent("tool:conf")),
        )),
        Record::Dating(Dating::offset(
            DatingTarget::Unit(u),
            -3_600_000,
            agent("tool:conf"),
            Hlc::new(6, 0, agent("tool:conf")),
        )),
    ]
}

#[test]
fn the_library_layer_answers_alike_everywhere() {
    let s = Store::from_records(library_records());
    let lib = StoreRead::library(&s);
    assert_eq!(lib.manifest_count(), 2);
    assert_eq!(lib.heads("bible").len(), 1);
    assert_eq!(lib.redacted_count(), 1);
    assert_eq!(lib.dating_count(), 1);
    assert_eq!(lib.parts().len(), 2);
    assert_eq!(
        lib.units_with_tid(&smysl_core::ids::Tid::from_normalised_bytes(&[2]))
            .len(),
        1
    );
    conform("library", &library_records());
    let mut both = lifecycle();
    both.extend(library_records());
    conform("library and lifecycle", &both);
}

/// A log as `Store::open` keeps it, repeats and a torn tail included, reads the same through the
/// unit store — and the store says what it found as `open` does.
#[cfg(feature = "store-redb")]
#[test]
fn the_unit_store_reads_a_log_with_repeats_and_a_torn_tail_as_open_does() {
    use smysl_graph::redb::{RedbOptions, RedbStore};

    let mut log: Vec<Record> = lifecycle();
    log.extend(library_records());
    let repeats: Vec<Record> = log.iter().step_by(3).cloned().collect();
    log.extend(repeats);
    let dir = scratch("torn", "redb");
    let path = dir.join("store.smy");
    let mut bytes = to_cbor_seq(&log);
    let whole = to_cbor(&log[0]);
    bytes.extend_from_slice(&whole[..whole.len() / 2]);
    std::fs::write(&path, &bytes).unwrap();

    let (want_store, want) = Store::open_with(&path, smysl_graph::StoreOptions::default()).unwrap();
    assert!(want_store.duplicate_records() > 0);
    let p = probes(&want_store);
    let want_lines = observe(&want_store, &p);
    let (s, got) = RedbStore::open(&path, RedbOptions::default()).unwrap();
    assert_eq!(got.trailing_bytes, want.trailing_bytes);
    assert!(got.trailing_bytes > 0);
    assert!(got
        .report
        .iter()
        .any(|d| d.code == smysl_core::diag::Code::W110));
    assert_eq!(s.len(), want_store.len());
    assert_eq!(s.log_len(), want_store.log_len());
    assert_eq!(s.log_hash(), want_store.log_hash());
    assert_eq!(s.log_bytes(), want_store.log_bytes());
    let got_lines = observe(&s, &p);
    assert_eq!(got_lines.len(), want_lines.len());
    for (g, w) in got_lines.iter().zip(&want_lines) {
        assert_eq!(g, w, "the unit store disagrees with open");
    }
    drop(s);

    // A log rewritten under the store is not the one it describes: rebuilt, with W501.
    std::fs::write(&path, to_cbor_seq(&lifecycle())).unwrap();
    let (s, report) = RedbStore::open(&path, RedbOptions::default()).unwrap();
    assert!(report.index_rebuilt);
    assert!(report
        .report
        .iter()
        .any(|d| d.code == smysl_core::diag::Code::W501));
    assert_eq!(
        s.record_set_digest(),
        Store::open(&path).unwrap().record_set_digest()
    );
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// `Send + Sync` for the in-memory store and what lies over it (SMYSL-2.8 §5.1).
#[test]
fn the_views_over_a_store_are_send_and_sync() {
    fn both<T: Send + Sync>() {}
    both::<Store>();
    both::<Overlay<'_, Store>>();
    both::<smysl_graph::Union<&Store>>();
    both::<smysl_graph::Union<Store>>();
    both::<smysl_graph::UidShards>();
}

/// `Send + Sync` for what the RFC asks it of (SMYSL-2.8 §5.1).
#[cfg(feature = "store-redb")]
#[test]
fn the_unit_store_is_send_and_sync() {
    fn both<T: Send + Sync>() {}
    both::<smysl_graph::redb::RedbShard>();
    both::<smysl_graph::redb::RedbStore>();
    both::<AnyStore>();
    both::<Overlay<'_, smysl_graph::redb::RedbStore>>();
    both::<smysl_graph::Union<smysl_graph::redb::RedbStore>>();
}
