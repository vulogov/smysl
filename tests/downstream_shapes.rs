//! The downstream-shape compile test (SMYSL-2.8 §5.2): a semver guard for TX-P13a.
//!
//! Every public function that took `&Store` and now takes `&S where S: StoreRead + ?Sized` is
//! called here the ways a downstream crate already calls it: through `&Store`, `&&Store`, a
//! reborrowed `&mut Store`, `&Box<Store>`, `&Arc<Store>` and `&Rc<Store>`, as a function
//! pointer typed over `&Store`, and passed to a higher-order function that expects `Fn(&Store,
//! …)`. Before the port the pointer shapes compiled by deref coercion; after it they compile
//! because the delegating impls exist. If one of these stops compiling, a caller's code would
//! have too.
//!
//! It also runs: each shape's answer is compared, in `Debug` form, with the plain `&Store` one.

use std::collections::BTreeSet;
use std::rc::Rc;
use std::sync::Arc;

use smysl::{
    canonical_uid, EdgeSet, KernelType, Label, LabelBinding, Lineage, Record, SalienceReport,
    SalienceRequest, SourceKind, SourceRef, Status, Store, TraceKind, Uid, UnitCoreBuilder,
};

fn fixture() -> (Store, Uid) {
    let ground = UnitCoreBuilder::new(KernelType::Prose, "The ground.", Status::Cited)
        .source(SourceRef::new(SourceKind::Doc, "shapes#1"))
        .build()
        .unwrap();
    let g = canonical_uid(&ground);
    let claim = UnitCoreBuilder::new(KernelType::Claim, "The claim.", Status::Derived)
        .grounds(vec![g])
        .build()
        .unwrap();
    let c = canonical_uid(&claim);
    let store = Store::from_records(vec![
        Record::Unit(ground),
        Record::Unit(claim),
        Record::LabelBinding(LabelBinding::new(Label::new("c/one").unwrap(), c)),
    ]);
    (store, c)
}

/// Evaluate `$body` with `$s` bound to each shape of the fixture store, and require every
/// shape's answer to print as the `&Store` one does.
macro_rules! every_shape {
    (|$s:ident, $root:ident| $body:expr) => {{
        let (base, $root) = fixture();
        let want = {
            let $s = &base;
            format!("{:?}", $body)
        };
        let mut got = Vec::new();
        {
            let r = &base;
            let $s = &r;
            got.push(format!("{:?}", $body));
        }
        {
            let mut m = base.clone();
            let $s = &mut m;
            got.push(format!("{:?}", $body));
        }
        {
            let b = Box::new(base.clone());
            let $s = &b;
            got.push(format!("{:?}", $body));
        }
        {
            let a = Arc::new(base.clone());
            let $s = &a;
            got.push(format!("{:?}", $body));
        }
        {
            let r = Rc::new(base.clone());
            let $s = &r;
            got.push(format!("{:?}", $body));
        }
        // Not a downstream shape — a new implementation (TX-P13a step 10) — but every function
        // here must take it, and answer for it as for the store it stands for.
        {
            let o = smysl::Overlay::new(&base, Vec::new());
            let $s = &o;
            got.push(format!("{:?}", $body));
        }
        for g in got {
            assert_eq!(g, want, "{}", stringify!($body));
        }
    }};
}

#[test]
fn graph_functions_take_every_shape() {
    every_shape!(|s, root| smysl::trace(s, root, TraceKind::Both, None));
    every_shape!(|s, root| smysl::trace_via(s, root, &EdgeSet::all(), Some(3)));
    every_shape!(|s, root| smysl::dependents(s, root));
    every_shape!(|s, root| smysl::dependents_via(s, root, &EdgeSet::all()));
    every_shape!(|s, root| smysl::rests_on(s, root, &EdgeSet::all()));
    every_shape!(|s, root| smysl::membership(s, &BTreeSet::from([root])));
    every_shape!(|s, _root| smysl::diff(s, s));
    every_shape!(|s, _root| smysl::hop_diff(s, 0, 1, true));
    every_shape!(|s, _root| smysl::salience(s, &SalienceRequest::default()));
    every_shape!(|s, _root| smysl::view_roots(s));
    every_shape!(|s, _root| smysl::label_bindings(s, &Label::new("c/one").unwrap()));
    every_shape!(|s, root| smysl::labels_of(s, &root));
    every_shape!(|s, _root| smysl::label_index(s));
    every_shape!(|s, _root| smysl::resolve_label(s, &Label::new("c/one").unwrap()));
}

#[test]
fn merge_and_maintenance_functions_take_every_shape() {
    use smysl::{
        DetectionContext, MergeOptions, Report, RetractionAuthority, RetractionPolicy,
        ReviewOptions,
    };
    every_shape!(|s, _root| smysl::effective_status(s, RetractionPolicy::Strict));
    every_shape!(|s, _root| {
        let mut report = Report::new();
        smysl_graph::merge::retraction::report_retractions(
            s,
            RetractionPolicy::Strict,
            &mut report,
        );
        report
    });
    every_shape!(|s, root| smysl::plan_retraction(
        s,
        root,
        &[],
        RetractionPolicy::Strict,
        RetractionAuthority::Any
    ));
    every_shape!(|s, _root| smysl_graph::merge::contention::detect(
        s,
        &DetectionContext::default()
    ));
    every_shape!(|s, _root| smysl::review(s, &DetectionContext::default()));
    every_shape!(|s, _root| smysl::review_with(s, &ReviewOptions::default()));
    every_shape!(|s, _root| smysl::relink(s));
    every_shape!(|s, _root| smysl::supersession_heads(s));
    every_shape!(|s, _root| smysl::compact(s));
    every_shape!(|s, _root| smysl_graph::compact::depth_by_unit(s));
    // `merge` keeps `&mut Store` first (SMYSL-2.8 §3.3); only the store merged in is generic.
    every_shape!(|s, _root| {
        let mut into = Store::new();
        smysl::merge(&mut into, s, MergeOptions::default()).map(|r| r.added)
    });
}

#[test]
fn check_functions_take_every_shape() {
    use smysl::{CheckOptions, ConsumerProfile};
    every_shape!(|s, _root| smysl::check(s, CheckOptions::default()));
    every_shape!(|s, _root| smysl_check::check_and_fail_on(
        s,
        CheckOptions::default(),
        smysl::Severity::Error
    )
    .is_ok());
    every_shape!(|s, _root| smysl_check::granularity_distribution(s));
    every_shape!(|s, _root| smysl_check::fidelity(s, &ConsumerProfile::default()));
    every_shape!(|s, _root| {
        let mut report = smysl::Report::new();
        smysl_check::passes::integrity::run(s, &mut report);
        smysl_check::passes::epistemics::run(s, &mut report);
        smysl_check::passes::commitment::run(s, &mut report);
        smysl_check::passes::trust::run(s, &mut report);
        smysl_check::passes::time::run(s, &mut report);
        report
    });
}

#[test]
fn pack_thread_render_retrieve_and_ingest_functions_take_every_shape() {
    use smysl::{PackRequest, ThreadSchema};
    use smysl_core::{AgentId, Hlc, Lod, Rung, ThreadId};

    let sal = |s: &Store| smysl::salience(s, &SalienceRequest::default());
    every_shape!(|s, _root| smysl::pack(s, &sal(&fixture().0), &PackRequest::default()));
    every_shape!(|s, _root| {
        let req = PackRequest::default();
        let p = smysl::pack(&fixture().0, &sal(&fixture().0), &req).unwrap();
        smysl_pack::verify(s, &p, &req)
    });
    every_shape!(|s, root| smysl_pack::constraints::warrants_of(s, &root));
    every_shape!(|s, root| smysl_pack::closure::required(s, root, Lod::L1));
    every_shape!(|s, root| smysl_pack::closure::reasons(s, root, Lod::L1));

    let opts = smysl::DeriveOptions::new(
        ThreadId::new("t/shapes").unwrap(),
        AgentId::new("tool:shapes").unwrap(),
    );
    every_shape!(|s, _root| smysl::derive_thread(s, ThreadSchema::Analysis, &opts));
    let (thread, _) = smysl::derive_thread(&fixture().0, ThreadSchema::Analysis, &opts);
    every_shape!(|s, _root| smysl::satisfies_rule_l(s, &thread));
    let profile = smysl::Profile::builtin(smysl::Profile::builtin_names()[0]).unwrap();
    every_shape!(|s, _root| smysl::build_ir(s, &thread, &profile, &smysl::BuildOptions::default()));

    every_shape!(|s, _root| {
        use smysl::Retriever;
        smysl::Bm25::index(s).search(&smysl::Query::new("claim", 5))
    });
    every_shape!(|s, _root| {
        use smysl::Retriever;
        smysl::Bm25::index_with(s, smysl::Tokenizer::plain()).search(&smysl::Query::new("claim", 5))
    });

    let attest = smysl_ingest::Attest::new(
        AgentId::new("tool:shapes").unwrap(),
        Rung::Document,
        Hlc::new(1, 0, AgentId::new("tool:shapes").unwrap()),
    );
    let batch = || {
        vec![
            UnitCoreBuilder::new(KernelType::Claim, "A staged claim.", Status::Speculative)
                .build()
                .unwrap(),
        ]
    };
    every_shape!(|s, _root| smysl_ingest::stage::prepare(
        s,
        batch(),
        Vec::new(),
        Default::default(),
        &attest
    )
    .records());
}

fn with_store<R>(f: impl Fn(&Store) -> R) -> R {
    f(&fixture().0)
}

#[test]
fn graph_functions_stay_function_values_over_store() {
    let trace: fn(&Store, Uid, TraceKind, Option<u32>) -> Lineage = smysl::trace;
    let (store, root) = fixture();
    assert_eq!(trace(&store, root, TraceKind::Both, None).len(), 2);
    let salience: fn(&Store, &SalienceRequest) -> SalienceReport = smysl::salience;
    let _ = salience(&store, &SalienceRequest::default());
    let _: fn(&Store, &Store) -> smysl::StoreDiff = smysl::diff;
    let _: fn(&Store) -> BTreeSet<Uid> = smysl::view_roots;
    let _: fn(&Store, &Label) -> Vec<Uid> = smysl::label_bindings;

    let _: fn(&Store) -> smysl::Compacted = smysl::compact;
    let _: fn(&Store) -> smysl::Relinked = smysl::relink;
    let _: fn(
        &mut Store,
        &Store,
        smysl::MergeOptions,
    ) -> Result<smysl::MergeReport, smysl::AnyError> = smysl::merge;

    let check: fn(&Store, smysl::CheckOptions) -> smysl::Report = smysl::check;
    assert!(check(&store, smysl::CheckOptions::default()).is_empty());
    assert!(with_store(|s| smysl::check(s, smysl::CheckOptions::default())).is_empty());

    let _: fn(
        &Store,
        &smysl::SalienceReport,
        &smysl::PackRequest,
    ) -> Result<smysl::Pack, smysl::PackError> = smysl::pack;
    let _: fn(
        &Store,
        smysl::ThreadSchema,
        &smysl::DeriveOptions,
    ) -> (smysl::Thread, smysl::DeriveReport) = smysl::derive_thread;
    let _: fn(&Store) -> smysl::Bm25 = smysl::Bm25::index;

    assert!(with_store(smysl::view_roots).is_empty());
    assert_eq!(with_store(smysl::compact).duplicates, 0);
    assert_eq!(with_store(smysl::label_index).len(), 1);
}
