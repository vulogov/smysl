//! Where the newer features meet each other.
//!
//! Relations, the rule T ceiling, `import`, episodes and recency, `relink`, quotes and
//! `compact` all landed close together and each was verified on its own. Every one of them
//! either **moves a unit's identity** or **reads something another one writes**, and those
//! are exactly the seams a per-feature test does not cover.
//!
//! Each test below is a question that had a plausible wrong answer.
//!
//! Three of them reach into `smysl-ingest`, which is an optional dependency of the facade,
//! so they are gated on the `ingest` feature. Without that gate the file fails to compile
//! under `--no-default-features` — which `--all-features` alone will never tell you, and
//! `make test-matrix` will.

use smysl::{
    canonical_uid, compact, relink, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind,
    Relation, Rung, SalienceRequest, SalienceWeights, Status, Store, Uid, UnitCore,
    UnitCoreBuilder,
};

fn agent() -> AgentId {
    AgentId::new("tool:test").unwrap()
}

fn unit(gist: &str, grounds: Vec<Uid>) -> UnitCore {
    let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
    if !grounds.is_empty() {
        b = b.grounds(grounds);
    }
    b.build().unwrap()
}

/// A unit carrying an attributed quote in its payload, as ingest produces.
#[cfg(feature = "ingest")]
fn quoted(gist: &str, quote: &str, grounds: Vec<Uid>) -> UnitCore {
    let span = smysl::Span::new(0, 0);
    let mut o = smysl::surface::hjson::HObject::default();
    o.insert(
        smysl::surface::hjson::Spanned::new(smysl_ingest::quote::QUOTE_KEY.to_string(), span),
        smysl::surface::hjson::Spanned::new(
            smysl::surface::hjson::HValue::Str(quote.to_string()),
            span,
        ),
    );
    let payload = smysl::surface::payload::object_to_payload(&o).expect("a payload");

    let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative).payload(payload);
    if !grounds.is_empty() {
        b = b.grounds(grounds);
    }
    b.build().unwrap()
}

fn attested(uid: Uid, hop: u32) -> Attestation {
    Attestation::new(
        uid,
        agent(),
        Op::Authored,
        Rung::Computed,
        Hlc::zero(agent()),
    )
    .at_hop(hop)
}

// ---------------------------------------------------------------------------
// relink × quotes
// ---------------------------------------------------------------------------

/// **Does an attributed quote survive relinking?**
///
/// `relink` rewrites a unit to point at a replacement, which changes its uid. If the rewrite
/// dropped anything it did not explicitly copy, the attribution would vanish silently — and
/// a quote lost is exactly the kind of loss nothing downstream would notice, because a unit
/// without one is perfectly legal.
// Needs the ingest layer, which `--no-default-features` leaves out.
#[cfg(feature = "ingest")]
#[test]
fn a_quote_survives_being_relinked() {
    let old = unit("the original evidence", vec![]);
    let new = unit("the corrected evidence", vec![]);
    let (o, n) = (canonical_uid(&old), canonical_uid(&new));
    let rests = quoted("rests on the evidence", "the original evidence", vec![o]);

    let store = Store::from_records(vec![
        Record::Unit(old),
        Record::Unit(new),
        Record::Unit(rests),
        Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
    ]);

    let out = relink(&store);
    let rewritten: Vec<&UnitCore> = out
        .records
        .iter()
        .filter_map(|r| match r {
            Record::Unit(u) => Some(u),
            _ => None,
        })
        .collect();
    assert_eq!(rewritten.len(), 1, "nothing was re-pointed");
    assert_eq!(
        smysl_ingest::quote::quote_of(rewritten[0]).as_deref(),
        Some("the original evidence"),
        "relinking dropped the attribution"
    );
    assert!(
        rewritten[0].grounds.contains(&n),
        "the reference did not move"
    );
}

// ---------------------------------------------------------------------------
// compact × episodes × recency
// ---------------------------------------------------------------------------

/// **Does compaction preserve which handoff produced what?**
///
/// Episodes live on attestations, and compaction filters attestations by whether their unit
/// survived. A filter that was even slightly wrong would strip the hop from units it kept,
/// and `Store::at_hop` would quietly start answering "nothing" — which reads identically to
/// a pipeline that added nothing.
#[test]
fn compaction_keeps_the_episode_of_every_unit_it_keeps() {
    let old = unit("the original", vec![]);
    let new = unit("the correction", vec![]);
    let later = unit("something from a later step", vec![]);
    let (o, n, l) = (
        canonical_uid(&old),
        canonical_uid(&new),
        canonical_uid(&later),
    );

    let store = Store::from_records(vec![
        Record::Unit(old),
        Record::Unit(new),
        Record::Unit(later),
        Record::Attestation(attested(o, 0)),
        Record::Attestation(attested(n, 3)),
        Record::Attestation(attested(l, 7)),
        Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
    ]);
    assert_eq!(store.hops(), [0, 3, 7].into_iter().collect());

    let after = Store::from_records(compact(&store).records);
    assert_eq!(after.hop_of(&n), Some(3), "a survivor lost its episode");
    assert_eq!(after.hop_of(&l), Some(7));
    assert_eq!(after.hop_of(&o), None, "a dropped unit kept an episode");
    assert_eq!(after.hops(), [3, 7].into_iter().collect());
    assert_eq!(after.latest_hop(), Some(7));
}

/// **Does recency still rank correctly after compaction?**
///
/// Recency reads `hop_of`, which reads attestations, which compaction filters. The ordering
/// it produces has to survive the store shrinking, or a pipeline that compacts starts
/// carrying forward the wrong things.
#[test]
fn recency_ranks_the_same_before_and_after_compaction() {
    let old = unit("the original", vec![]);
    let new = unit("the correction", vec![]);
    let fresh = unit("the newest claim", vec![]);
    let (o, n, f) = (
        canonical_uid(&old),
        canonical_uid(&new),
        canonical_uid(&fresh),
    );

    let store = Store::from_records(vec![
        Record::Unit(old),
        Record::Unit(new),
        Record::Unit(fresh),
        Record::Attestation(attested(o, 0)),
        Record::Attestation(attested(n, 1)),
        Record::Attestation(attested(f, 6)),
        Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
    ]);

    let req = || {
        SalienceRequest::default()
            .with_weights(SalienceWeights::recent())
            .at_hop(6)
    };
    let before = smysl::salience(&store, &req());
    let after_store = Store::from_records(compact(&store).records);
    let after = smysl::salience(&after_store, &req());

    assert!(
        before.get(&f) > before.get(&n),
        "the fresh unit did not outrank the older survivor before compaction"
    );
    assert!(
        after.get(&f) > after.get(&n),
        "compaction inverted the recency ordering"
    );
}

// ---------------------------------------------------------------------------
// rule M weakening × relations
// ---------------------------------------------------------------------------

/// **Does the rule M weakening carry relations to the units' new identities?**
///
/// Weakening lowers a status, which moves a uid. Grounds and deps are re-pointed — but
/// relations are a separate record type, and an edge left pointing at the pre-weakening uid
/// would dangle. `rebuts` edges are the ones rule R needs, so losing one silently loses the
/// constraint rather than merely a link.
// Needs the ingest layer, which `--no-default-features` leaves out.
#[cfg(feature = "ingest")]
#[test]
fn weakening_at_staging_carries_relations_to_the_new_identities() {
    use smysl_ingest::stage::{prepare, Attest};

    let weak = unit("a guess", vec![]);
    let weak_uid = canonical_uid(&weak);
    // `derived` on a `speculative` ground: rule M lowers it, and its uid moves.
    let over = UnitCoreBuilder::new(KernelType::Claim, "an overclaim", Status::Derived)
        .grounds([weak_uid])
        .build()
        .unwrap();
    let over_uid = canonical_uid(&over);
    let rebutter = unit("the objection", vec![]);
    let rebutter_uid = canonical_uid(&rebutter);

    let edge = Relation::new(RelKind::Rebuts, rebutter_uid, over_uid);
    let attest = Attest::new(agent(), Rung::Document, Hlc::zero(agent()));
    let staged = prepare(
        &Store::new(),
        vec![weak, over, rebutter],
        vec![edge],
        std::collections::BTreeMap::new(),
        &attest,
    );

    assert_eq!(staged.weakened.len(), 1, "rule M did not bind");
    assert_eq!(staged.relations.len(), 1, "the edge was lost");

    // Both ends must be units that are actually staged, or the batch ships a dangling edge.
    let staged_uids: std::collections::BTreeSet<Uid> =
        staged.units.iter().map(canonical_uid).collect();
    let rel = &staged.relations[0];
    assert!(staged_uids.contains(&rel.from), "`from` dangles");
    assert!(
        staged_uids.contains(&rel.to),
        "`to` still points at the pre-weakening identity"
    );
    assert_ne!(rel.to, over_uid, "the edge did not follow the weakening");

    // And the store built from it is sound, which is the property that matters downstream.
    let store = Store::from_records(staged.records());
    let mut report = smysl::Report::new();
    store.report_dangling(&mut report);
    assert!(report.is_empty(), "{report}");
    assert!(
        !store.rebuttals_of(&rel.to).is_empty(),
        "rule R cannot see the rebuttal it needs"
    );
}

// ---------------------------------------------------------------------------
// import × rule M × pack
// ---------------------------------------------------------------------------

/// **Does a `measured` import actually raise what can be built on it?**
///
/// The two halves have to meet: `import` writes `measured` with the attestation that permits
/// it, and rule M caps conclusions at their weakest ground. A claim resting on an imported
/// measurement should be allowed to be `derived` — which it could not have been when the
/// only checkable `measured` units were ones with no provenance at all.
// Needs the ingest layer, which `--no-default-features` leaves out.
#[cfg(feature = "ingest")]
#[test]
fn a_conclusion_resting_on_an_imported_measurement_may_be_derived() {
    use smysl_ingest::import::{from_csv, ImportOptions};

    let opts = ImportOptions::new("latency.csv", agent(), Hlc::zero(agent()));
    let imported = from_csv("region,p95_ms\neu-west,610\n", &opts);
    assert_eq!(imported.units.len(), 1);
    let measurement = canonical_uid(&imported.units[0]);
    assert_eq!(imported.units[0].status, Status::Measured);

    let conclusion = UnitCoreBuilder::new(
        KernelType::Finding,
        "eu-west is over its latency budget",
        Status::Derived,
    )
    .grounds([measurement])
    .build()
    .unwrap();

    let mut records = imported.records();
    records.push(Record::Unit(conclusion));
    let store = Store::from_records(records);

    let report = smysl::check(&store, smysl::CheckOptions::default());
    assert!(
        report.fail_on(smysl::Severity::Error).is_ok(),
        "a conclusion on an imported measurement did not check: {report}"
    );
}

// ---------------------------------------------------------------------------
// relink × compact, end to end
// ---------------------------------------------------------------------------

/// **Does the two-step workflow actually reach a smaller, sound store?**
///
/// `compact` refuses to drop a superseded unit something still references, and `relink` is
/// what moves those references. Each was tested alone; this is the sequence a user is told
/// to run, and it has to end somewhere better than it started.
#[test]
fn relink_then_compact_shrinks_the_store_and_leaves_it_sound() {
    let old = unit("the original evidence", vec![]);
    let new = unit("the corrected evidence", vec![]);
    let (o, n) = (canonical_uid(&old), canonical_uid(&new));
    let rests = unit("rests on the evidence", vec![o]);

    let store = Store::from_records(vec![
        Record::Unit(old),
        Record::Unit(new),
        Record::Unit(rests),
        Record::Relation(Relation::new(RelKind::Supersedes, n, o)),
    ]);

    // Compaction alone can do nothing: the reference pins the superseded unit in place.
    let first = compact(&store);
    assert!(first.is_empty());
    assert_eq!(first.still_referenced.len(), 1);

    // Relink moves the reference, then compaction has something to drop.
    let mut records: Vec<Record> = store.iter().cloned().collect();
    records.extend(relink(&store).records);
    let linked = Store::from_records(records);

    let second = compact(&linked);
    assert!(!second.is_empty(), "relinking did not unblock compaction");

    let final_store = Store::from_records(second.records);
    assert!(
        final_store.iter().count() < linked.iter().count(),
        "the store did not shrink"
    );
    let mut report = smysl::Report::new();
    final_store.report_dangling(&mut report);
    assert!(
        report.is_empty(),
        "compaction left a dangling reference: {report}"
    );
    assert!(
        final_store.get(&n).is_some(),
        "the correction itself was dropped"
    );
}

/// A label bound in one store and a *different* uid bound to the same label in another is a
/// `label-collision` contention. This pins that it survives the wire.
///
/// It could not, before `Record::LabelBinding`. Labels reached `merge` out of band — one map
/// per input, supplied by whoever parsed the surface — and a CBOR store had no labels to
/// supply, so the CLI handed merge an empty map and the detection had nothing to compare.
/// The format carried a dedicated contention kind for disagreements about a thing it could
/// not store.
///
/// Written as an interaction test because neither half shows it alone: the codec round trip
/// looks fine without merge, and merge looks fine when handed surface files.
#[test]
fn a_label_collision_is_detected_across_a_cbor_round_trip() {
    use smysl::{from_cbor_seq, merge, to_cbor_seq, Label, LabelBinding, MergeOptions};

    // Same label, two different units — so two different uids.
    let mk = |gist: &str| -> Vec<Record> {
        let core = unit(gist, vec![]);
        let uid = canonical_uid(&core);
        vec![
            Record::Unit(core),
            Record::LabelBinding(LabelBinding::new(Label::new("c/cause").unwrap(), uid)),
        ]
    };

    // Through the wire and back, which is the step that used to lose the bindings.
    let via_cbor = |records: Vec<Record>| -> Store {
        let bytes = to_cbor_seq(&records);
        let (back, _) = from_cbor_seq(&bytes).expect("a store we just wrote must decode");
        Store::from_records(back)
    };

    let a = via_cbor(mk("the pool is saturated"));
    let b = via_cbor(mk("the index is missing"));

    // Recover each store's labels from its bindings, exactly as the CLI does.
    let labels_of = |s: &Store| -> std::collections::BTreeMap<Label, Uid> {
        s.iter()
            .filter_map(|r| match r {
                Record::LabelBinding(lb) => Some((lb.label.clone(), lb.uid)),
                _ => None,
            })
            .collect()
    };

    let mut store = a;
    let mut opts = MergeOptions::default();
    opts.now = Some(Hlc::new(0, 0, agent()));
    opts.labels = vec![labels_of(&store), labels_of(&b)];

    let report = merge(&mut store, &b, opts).expect("merge must not fail");
    assert!(
        report
            .contentions
            .iter()
            .any(|c| c.detected.kind == smysl::DetectionKind::LabelCollision),
        "a label bound to two different uids went undetected: {:?}",
        report
            .contentions
            .iter()
            .map(|c| c.detected.kind)
            .collect::<Vec<_>>()
    );
}
