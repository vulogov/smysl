//! The exporter of TX-P11 step 2 (SMYSL-2.7 §3.6, §3.7, §5.1).
//!
//! One test per row of §3.6 that step 2 implements, including the reproductions of §2.3, then
//! the step's exit: F1–F13 exported under `single` (and `by-view`) with `full`, and the
//! in-memory P1, `rsd` of what an empty store retains of the files equal to `rsd(X)`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::surface::parse_surface;
use smysl_core::types::record::code;
use smysl_core::{
    canonical_uid, from_cbor_seq, AgentId, Attestation, Commit, Commitment, Contention,
    ContentionId, Detected, DetectionKind, Hlc, KernelType, Op, Record, RelKind, Relation, Role,
    Rung, Status, Step, Thread, ThreadId, ThreadSchema, Uid, UnitCore, UnitCoreBuilder, ViewId,
    Withdrawal,
};
use smysl_graph::Store;
use smysl_lib::{export_store, Collection, ExportOptions, Layout, Selection, UnknownRecords, With};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> Vec<Record> {
    let path = repo().join(rel);
    if rel.ends_with(".cbor") {
        let bytes = std::fs::read(&path).unwrap();
        from_cbor_seq(&bytes)
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .0
    } else {
        let src = std::fs::read_to_string(&path).unwrap();
        parse_surface(&src)
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .records
    }
}

/// F1–F13: the corpus documents, and F10, which exists only on the wire.
fn corpus() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo().join("fixtures/corpus"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".smy"))
        .map(|n| format!("fixtures/corpus/{n}"))
        .collect();
    out.push("fixtures/wire/F10-lifecycle.cbor".to_string());
    out.push("fixtures/wire/F14-unknown-codes.cbor".to_string());
    out.sort();
    out
}

fn agent() -> AgentId {
    AgentId::new("tool:test").unwrap()
}

fn ts(n: u64) -> Hlc {
    Hlc::new(n, 0, agent())
}

fn unit(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

fn grounded(gist: &str, on: Uid) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Inferred)
        .grounds([on])
        .build()
        .unwrap()
}

fn all(store: &Store, opts: &ExportOptions) -> Collection {
    export_store(store, &Selection::all(store), opts).unwrap()
}

fn only(store: &Store, u: Uid) -> Collection {
    export_store(store, &Selection::of([u]), &ExportOptions::new()).unwrap()
}

fn hashes(c: &Collection) -> BTreeSet<[u8; 32]> {
    c.records().iter().map(Store::record_hash).collect()
}

fn holds(c: &Collection, r: &Record) -> bool {
    hashes(c).contains(&Store::record_hash(r))
}

/// What an empty store retains of the files: the in-memory stand-in for `lib import`, which
/// arrives in step 4. P1 is that its digest is the collection's.
fn p1(c: &Collection) -> bool {
    Store::from_records(c.records()).record_set_digest() == c.rsd()
}

fn count(c: &Collection, code: smysl_core::Code) -> usize {
    c.report.iter().filter(|d| d.code == code).count()
}

// --- §3.6, row by row ----------------------------------------------------------------------

#[test]
fn f13s_four_commitments_travel() {
    let store = Store::from_records(fixture("fixtures/corpus/F13-canon.smy"));
    let c = all(&store, &ExportOptions::new());
    let commits = c
        .records()
        .iter()
        .filter(|r| matches!(r, Record::Commit(_)))
        .count();
    assert_eq!(commits, 4, "§2.3: `bundle` drops every one of them");
}

#[test]
fn an_extension_declaration_travels_with_what_it_declares() {
    for f in ["F11-extension-kinds", "F12-extension-types"] {
        let store = Store::from_records(fixture(&format!("fixtures/corpus/{f}.smy")));
        let c = all(&store, &ExportOptions::new());
        let decls = c
            .records()
            .iter()
            .filter(|r| matches!(r, Record::SchemaDecl(_)))
            .count();
        assert_eq!(decls, 1, "{f}");
        let view = c.files[0].view.clone().unwrap();
        let requires = match &c.records()[0] {
            Record::View(v) if v.id == view => v.requires.len(),
            other => panic!("{f}: the file does not open with its view: {other:?}"),
        };
        assert_eq!(
            requires, 1,
            "{f}: the generated view requires what it includes"
        );
    }
}

#[test]
fn an_edge_attestation_travels_with_its_edge() {
    let (a, b) = (unit("a"), unit("b"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let rel = Relation::new(RelKind::Contrasts, ua, ub);
    let on_edge = Attestation::new(rel.uid(), agent(), Op::Authored, Rung::Document, ts(1));
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Relation(rel),
        Record::Attestation(on_edge.clone()),
    ]);
    let c = only(&store, ua);
    // Gap A: `emit` tests an attestation's uid against the kept units, and this one names a rid.
    assert!(holds(&c, &Record::Attestation(on_edge)));
    assert!(p1(&c));
}

#[test]
fn annotation_units_are_pulled_in() {
    let (a, b) = (unit("a"), unit("b"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let (rn, cn, pos) = (
        unit("on the edge"),
        unit("why committed"),
        unit("a position"),
    );
    let (urn, ucn, upos) = (canonical_uid(&rn), canonical_uid(&cn), canonical_uid(&pos));
    let rel = Relation::new(RelKind::Contrasts, ua, ub).with_note(urn);
    let commit = Commit::new(ua, Commitment::Committed, agent(), ts(2)).with_note(ucn);
    let k = Contention::new(
        ContentionId::new("k/x").unwrap(),
        ua,
        vec![upos],
        Detected::new(DetectionKind::LiveRebuttal, ts(3)),
    );
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Unit(rn.clone()),
        Record::Unit(cn.clone()),
        Record::Unit(pos.clone()),
        Record::Relation(rel),
        Record::Commit(commit),
        Record::Contention(k),
    ]);
    let c = only(&store, ua);
    // Gap B: no edge reaches any of the three, so `closure` alone exports them dangling.
    for u in [rn, cn, pos] {
        assert!(holds(&c, &Record::Unit(u.clone())), "{}", u.gist);
    }
    assert_eq!(count(&c, smysl_core::Code::W495), 0);
    assert!(p1(&c));
}

#[test]
fn a_withdrawn_relation_and_its_withdrawal_both_travel() {
    let (a, b) = (unit("a"), unit("b"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let rel = Relation::new(RelKind::Contrasts, ua, ub);
    let w = Withdrawal::new(rel.uid(), agent(), ts(4));
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Relation(rel.clone()),
        Record::Withdrawal(w.clone()),
    ]);
    let c = all(&store, &ExportOptions::new());
    assert!(holds(&c, &Record::Relation(rel)));
    assert!(holds(&c, &Record::Withdrawal(w)));
}

#[test]
fn two_weights_of_one_edge_both_travel() {
    let (a, b) = (unit("a"), unit("b"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let light = Relation::new(RelKind::Contrasts, ua, ub).with_weight(0.25);
    let heavy = Relation::new(RelKind::Contrasts, ua, ub).with_weight(0.75);
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Relation(light.clone()),
        Record::Relation(heavy.clone()),
    ]);
    let c = all(&store, &ExportOptions::new());
    // The store's `relations` map keeps one of the two; the log keeps both, and so does export.
    assert!(holds(&c, &Record::Relation(light)));
    assert!(holds(&c, &Record::Relation(heavy)));
}

#[test]
fn unknown_records_go_to_unattributed_and_checkpoints_go_nowhere() {
    let a = unit("a");
    let future = Record::Unknown {
        code: 42,
        payload: vec![0xa0],
    };
    let checkpoint = Record::Unknown {
        code: code::CHECKPOINT,
        payload: vec![0xa0],
    };
    let store = Store::from_records(vec![Record::Unit(a), future.clone(), checkpoint.clone()]);
    let c = all(&store, &ExportOptions::new());
    let un = c.file("unattributed.cbor").expect("written");
    assert_eq!(un.view, None);
    assert_eq!(from_cbor_seq(&un.bytes).unwrap().0, vec![future.clone()]);
    assert!(!holds(&c, &checkpoint));
    assert_eq!(count(&c, smysl_core::Code::W425), 1);
    assert!(p1(&c));

    let dropped = all(&store, &ExportOptions::new().unknown(UnknownRecords::Drop));
    assert!(dropped.file("unattributed.cbor").is_none());
    assert!(!holds(&dropped, &future));
    assert_eq!(count(&dropped, smysl_core::Code::W425), 1, "still counted");
}

#[test]
fn a_thread_reaching_outside_the_file_is_left_out_and_counted() {
    let (a, b) = (unit("a"), unit("b"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let mut t = Thread::new(
        ThreadId::new("t/two").unwrap(),
        ThreadSchema::Analysis,
        agent(),
        "two unconnected units",
        ts(5),
    );
    t.steps = vec![Step::new(Role::Context, ua), Step::new(Role::Finding, ub)];
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Thread(t.clone()),
    ]);

    let c = only(&store, ua);
    assert!(!holds(&c, &Record::Thread(t.clone())));
    assert_eq!(count(&c, smysl_core::Code::W490), 1);

    let both = all(&store, &ExportOptions::new());
    assert!(holds(&both, &Record::Thread(t)));
    assert_eq!(count(&both, smysl_core::Code::W490), 0);
    match &both.records()[0] {
        Record::View(v) => assert_eq!(v.threads.len(), 1, "the view names the thread it holds"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn closure_follows_grounds_and_brings_live_rebuttals() {
    let base = unit("base");
    let ub = canonical_uid(&base);
    let top = grounded("top", ub);
    let ut = canonical_uid(&top);
    let rebut = unit("no");
    let ur = canonical_uid(&rebut);
    let store = Store::from_records(vec![
        Record::Unit(base.clone()),
        Record::Unit(top),
        Record::Unit(rebut.clone()),
        Record::Relation(Relation::new(RelKind::Rebuts, ur, ub)),
    ]);
    let c = only(&store, ut);
    assert!(holds(&c, &Record::Unit(base)), "rule L");
    assert!(holds(&c, &Record::Unit(rebut)), "rule R");
    match &c.records()[0] {
        Record::View(v) => assert_eq!(v.roots, BTreeSet::from([ut]), "roots are the selection"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_selection_naming_a_unit_the_store_lacks_is_refused() {
    let store = Store::from_records(vec![Record::Unit(unit("a"))]);
    let stranger = canonical_uid(&unit("b"));
    assert!(export_store(&store, &Selection::of([stranger]), &ExportOptions::new()).is_err());
}

// --- §3.7: one order, whatever the log's ----------------------------------------------------

/// A seeded xorshift, as the graph crate's property tests use: no `rand` in the pure tier.
fn shuffle<T>(v: &mut [T], seed: &mut u64) {
    for i in (1..v.len()).rev() {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        v.swap(i, (*seed % (i as u64 + 1)) as usize);
    }
}

#[test]
fn the_bytes_do_not_depend_on_log_order() {
    let opts = ExportOptions::new()
        .layout(Layout::ByView)
        .with(With::new().views(true).packs(true));
    let mut seed = 0x5eed_2700_u64;
    for rel in corpus() {
        let records = fixture(&rel);
        let first = all(&Store::from_records(records.clone()), &opts);
        for _ in 0..50 {
            let mut shuffled = records.clone();
            shuffle(&mut shuffled, &mut seed);
            let again = all(&Store::from_records(shuffled), &opts);
            assert_eq!(again, first, "{rel}");
        }
    }
}

// --- The exit: F1–F13, `single` and `by-view`, `full`, in-memory P1 -------------------------

/// Every record of the store is exported when every unit is selected, bar the ones §3.6 rules
/// out by type, and the ones W490 counts.
#[test]
fn every_fixture_exports_whole_and_passes_p1() {
    let with = With::new().views(true).packs(true);
    for rel in corpus() {
        let store = Store::from_records(fixture(&rel));
        for layout in [Layout::Single, Layout::ByView] {
            let c = all(&store, &ExportOptions::new().layout(layout).with(with));
            assert!(p1(&c), "{rel} {layout:?}: P1");
            for f in &c.files {
                assert_eq!(
                    f.rsd,
                    smysl_graph::record_set_digest(&from_cbor_seq(&f.bytes).unwrap().0)
                );
                if let Some(v) = &f.view {
                    assert!(ViewId::new(v.as_str()).is_ok());
                    assert_eq!(f.path, format!("{}.cbor", &v.as_str()["export/".len()..]));
                }
            }
            let got = hashes(&c);
            let lost: Vec<&Record> = store
                .iter()
                .filter(|r| !got.contains(&Store::record_hash(r)))
                .filter(|r| !matches!(r, Record::Unknown { code, .. } if *code == code::CHECKPOINT))
                .collect();
            let w490: usize = c
                .report
                .iter()
                .filter(|d| d.code == smysl_core::Code::W490)
                .map(|d| {
                    d.message
                        .split(' ')
                        .next()
                        .unwrap()
                        .parse::<usize>()
                        .unwrap()
                })
                .sum();
            // Under `single` every left-out record is lost. Under `by-view` a thread left out
            // of one view's file may be in another's, so W490 counts per file and bounds it.
            match layout {
                Layout::Single => assert_eq!(lost.len(), w490, "{rel}: lost {lost:?}"),
                _ => assert!(lost.len() <= w490, "{rel} {layout:?}: lost {lost:?}"),
            }
            assert!(
                lost.iter()
                    .all(|r| matches!(r, Record::Thread(_) | Record::PackInfo(_))),
                "{rel} {layout:?}: only W490's types are ever left out: {lost:?}"
            );
        }
        let single = all(&store, &ExportOptions::new().with(with));
        let keyed = single.files.iter().filter(|f| f.view.is_some()).count();
        assert_eq!(
            keyed,
            usize::from(store.units().next().is_some()),
            "{rel}: one file"
        );
    }
}

// --- Completions of §3.6's table, found by the exit -----------------------------------------

/// F10 resolves a contention its store neither records nor detects. Row 12 alone drops it;
/// §3.6's "dangling at source" exports it as it is, counted.
#[test]
fn a_record_naming_nothing_the_store_holds_travels_and_is_counted() {
    let store = Store::from_records(fixture("fixtures/wire/F10-lifecycle.cbor"));
    let resolutions = store
        .iter()
        .filter(|r| matches!(r, Record::Resolution(_)))
        .count();
    let c = all(&store, &ExportOptions::new());
    let got = c
        .records()
        .iter()
        .filter(|r| matches!(r, Record::Resolution(_)))
        .count();
    assert_eq!(got, resolutions);
    assert_eq!(count(&c, smysl_core::Code::W495), 1);
    assert!(c
        .report
        .iter()
        .any(|d| d.message.starts_with("1 exported record")));
}

#[test]
fn by_view_cuts_one_file_per_view_and_one_for_the_rest() {
    let (a, b, z) = (unit("a"), unit("b"), unit("in no view"));
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let va = smysl_core::View::new(ViewId::new("v/alpha").unwrap(), "x").with_roots([ua]);
    let vb = smysl_core::View::new(ViewId::new("v/beta").unwrap(), "x").with_roots([ua, ub]);
    let store = Store::from_records(vec![
        Record::Unit(a.clone()),
        Record::Unit(b.clone()),
        Record::Unit(z.clone()),
        Record::View(va),
        Record::View(vb),
    ]);
    let c = all(&store, &ExportOptions::new().layout(Layout::ByView));
    let paths: Vec<&str> = c.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["unkeyed.cbor", "v-alpha.cbor", "v-beta.cbor"]);
    let roots = |p: &str| match &from_cbor_seq(&c.file(p).unwrap().bytes).unwrap().0[0] {
        Record::View(v) => v.roots.clone(),
        other => panic!("{other:?}"),
    };
    // `a` is in both views and goes to the least id; each unit is a root exactly once.
    assert_eq!(roots("v-alpha.cbor"), BTreeSet::from([ua]));
    assert_eq!(roots("v-beta.cbor"), BTreeSet::from([ub]));
    assert_eq!(roots("unkeyed.cbor"), BTreeSet::from([canonical_uid(&z)]));
    assert_eq!(count(&c, smysl_core::Code::W497), 0);
    // The three units, and each file's keying view, which travels with it (step 3).
    assert_eq!(c.duplication(), (5, 5));
    assert!(p1(&c));
}

#[test]
fn a_declaration_used_only_by_payload_keys_travels() {
    // F11's shape: the declaration lists no type and no kind, and its units name it only
    // through `"code:kind"`.
    let store = Store::from_records(fixture("fixtures/corpus/F11-extension-kinds.smy"));
    let decl = store
        .iter()
        .find(|r| matches!(r, Record::SchemaDecl(_)))
        .cloned()
        .unwrap();
    let pin = store
        .units()
        .find(|(_, u)| u.core.payload.is_some())
        .map(|(u, _)| *u)
        .unwrap();
    assert!(holds(&only(&store, pin), &decl));
}
