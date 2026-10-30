//! The store conformance suite's operations half (SMYSL-2.8 §5.1).
//!
//! `crates/smysl-graph/tests/store_conformance.rs` compares every `StoreRead` answer across
//! implementations. This compares what the *operations* in other crates make of them: an
//! operation ported to `impl StoreRead` (TX-P13a steps 5–10) must give an [`Overlay`] the
//! answer it gives the store the overlay stands for — `Store::from_records` of the same
//! records, in the same order. `smysl_ingest::stage` checks every batch through an overlay since
//! step 10, so the `check` comparison here is the property ingest relies on.
//!
//! With feature `store-redb`, the unit store on disk is compared too, with its core cache off:
//! every operation here then reads every core it touches from the log.
//!
//! Compared in `Debug` form, which for every result type here is its whole content.

use smysl::{
    parse_surface, CheckOptions, DetectionContext, EdgeSet, Overlay, PackRequest, Query, Record,
    RetractionPolicy, Retriever, ReviewOptions, SalienceRequest, Store, ThreadSchema, TraceKind,
};
use smysl_core::{AgentId, ThreadId};

fn fixtures(dir: &str) -> Vec<(String, Vec<Record>)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
    let mut paths: Vec<_> = std::fs::read_dir(root.join(dir))
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "smy"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            let parsed = parse_surface(&std::fs::read_to_string(&p).unwrap()).ok()?;
            Some((
                p.file_name().unwrap().to_string_lossy().into_owned(),
                parsed.records,
            ))
        })
        .collect()
}

const SCHEMAS: [ThreadSchema; 5] = [
    ThreadSchema::Analysis,
    ThreadSchema::Narrative,
    ThreadSchema::Brief,
    ThreadSchema::Qa,
    ThreadSchema::Plan,
];

/// Every operation's answer on one store, as lines.
fn operations(s: &(impl smysl::StoreRead + ?Sized)) -> Vec<String> {
    let mut out = Vec::new();
    let mut say = |k: &str, v: String| out.push(format!("{k}: {v}"));

    let mut report = smysl::check(s, CheckOptions::default());
    report.sort();
    say("check", format!("{report:?}"));
    say(
        "fidelity",
        format!("{:?}", smysl_check::granularity_distribution(s)),
    );

    let sal = smysl::salience(s, &SalienceRequest::default());
    say("salience", format!("{sal:?}"));
    for budget in [200, 1000, 4000] {
        let mut req = PackRequest::default();
        req.budget = budget;
        say(
            &format!("pack {budget}"),
            format!("{:?}", smysl::pack(s, &sal, &req)),
        );
    }

    let opts = smysl::DeriveOptions::new(
        ThreadId::new("t/ops").unwrap(),
        AgentId::new("tool:ops").unwrap(),
    );
    let profile = smysl::Profile::builtin(smysl::Profile::builtin_names()[0]).unwrap();
    for schema in SCHEMAS {
        let (thread, report) = smysl::derive_thread(s, schema, &opts);
        say(
            &format!("thread {schema:?}"),
            format!("{thread:?} {report:?}"),
        );
        say(
            &format!("rule L {schema:?}"),
            format!("{:?}", smysl::satisfies_rule_l(s, &thread)),
        );
        say(
            &format!("ir {schema:?}"),
            format!(
                "{:?}",
                smysl::build_ir(s, &thread, &profile, &smysl::BuildOptions::default())
            ),
        );
    }

    let roots: Vec<smysl::Uid> = s.units().take(10).map(|(u, _)| *u).collect();
    for root in &roots {
        say(
            "trace",
            format!("{:?}", smysl::trace(s, *root, TraceKind::Both, None)),
        );
        say(
            "trace_via",
            format!("{:?}", smysl::trace_via(s, *root, &EdgeSet::all(), Some(4))),
        );
        say("dependents", format!("{:?}", smysl::dependents(s, *root)));
    }
    for policy in [RetractionPolicy::Strict, RetractionPolicy::Advisory] {
        say(
            &format!("effective {policy:?}"),
            format!("{:?}", smysl::effective_status(s, policy)),
        );
    }
    say(
        "detect",
        format!(
            "{:?}",
            smysl_graph::merge::contention::detect(s, &DetectionContext::default())
        ),
    );
    say(
        "review",
        format!("{:?}", smysl::review_with(s, &ReviewOptions::default())),
    );
    say("compact", format!("{:?}", smysl::compact(s)));
    say("relink", format!("{:?}", smysl::relink(s)));

    let bm = smysl::Bm25::index(s);
    for q in ["the", "incident deploy", "claim", "rollback latency"] {
        say(
            &format!("bm25 {q}"),
            format!("{:?}", bm.search(&Query::new(q, 10))),
        );
    }
    out
}

fn conform(name: &str, records: &[Record]) {
    let oracle = Store::from_records(records.to_vec());
    let want = operations(&oracle);
    assert!(want.len() >= 30, "{name}: {} answers", want.len());
    // Something to disagree about: an empty check report on every corpus would make this
    // comparison vacuous for the operation ingest depends on.
    let log: Vec<Record> = oracle.iter().cloned().collect();
    let n = log.len();
    for k in [n / 3, 2 * n / 3] {
        let base = Store::from_records(log[..k].to_vec());
        let got = operations(&Overlay::new(&base, log[k..].to_vec()));
        assert_eq!(got.len(), want.len(), "{name} at {k}/{n}: answer count");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "{name}: an Overlay at {k}/{n} disagrees");
        }
    }
    #[cfg(feature = "store-redb")]
    {
        let dir = std::env::temp_dir().join(format!(
            "smysl-ops-redb-{}-{}",
            std::process::id(),
            name.replace(['/', '.', ' '], "-")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("store.smy");
        std::fs::write(&path, smysl::to_cbor_seq(&log)).unwrap();
        let mut opts = smysl::redb::RedbOptions::default();
        opts.core_cache_bytes = 0;
        let (s, _) = smysl::redb::RedbStore::open(&path, opts).unwrap();
        let got = operations(&s);
        assert_eq!(got.len(), want.len(), "{name}: RedbStore: answer count");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g, w, "{name}: the unit store disagrees");
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn operations_answer_an_overlay_as_its_store_on_the_corpora() {
    let corpora = fixtures("corpus");
    assert!(corpora.len() >= 13, "{} corpora", corpora.len());
    for (name, records) in &corpora {
        conform(name, records);
    }
}

#[test]
fn operations_answer_an_overlay_as_its_store_on_the_check_fixtures() {
    let mut diagnosed = 0;
    for (name, records) in fixtures("conformance/check") {
        diagnosed += usize::from(
            !smysl::check(
                &Store::from_records(records.clone()),
                CheckOptions::default(),
            )
            .is_empty(),
        );
        conform(&name, &records);
    }
    assert!(diagnosed > 0, "no check fixture produced a diagnostic");
}
