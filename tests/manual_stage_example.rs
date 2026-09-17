//! Chapter 28's staging example, compiled and run. The book prints this code; if it stops
//! building, this fails rather than the page going stale.

#![cfg(feature = "stage")]

#[test]
fn build_check_and_stage_units_without_a_model() {
    // --- begin manual example ---
    use smysl::stage::{self, Attest};
    use smysl::{
        canonical_uid, dependents_via, quote_support_in, resolve_label, AgentId, EdgeSet, Hlc,
        KernelType, Label, QuoteSupport, RelKind, Relation, Rung, Severity, SourceKind, SourceRef,
        Status, Store, UnitCoreBuilder,
    };
    use std::collections::BTreeMap;

    let message = "Use BLAKE3 for uids.\n\nSHA-256 was too slow on large stores.";
    let diff = "+blake3 = \"1\"";

    // Check the evidence first, against every text the change is evidenced by.
    let quote = "SHA-256 was too slow on large stores";
    let (support, from) = quote_support_in(quote, &[("message", message), ("diff", diff)]);
    assert_eq!(support, QuoteSupport::Present);
    assert_eq!(from, Some("message"));

    // The tool, not a model, knows where this came from.
    let source = SourceRef::new(SourceKind::Doc, "git:4968383");
    let prerequisite = UnitCoreBuilder::new(
        KernelType::Claim,
        "SHA-256 was too slow on large stores",
        Status::Cited,
    )
    .source(source.clone())
    .build()
    .unwrap();
    let decision = UnitCoreBuilder::new(KernelType::Decision, "use BLAKE3 for uids", Status::Cited)
        .source(source)
        .build()
        .unwrap();
    let (p, d) = (canonical_uid(&prerequisite), canonical_uid(&decision));

    // `conditions`, so rewording the prerequisite never moves the decision's uid.
    let relations = vec![Relation::new(RelKind::Conditions, p, d)];
    let labels = BTreeMap::from([
        (Label::new("c/sha-slow").unwrap(), p),
        (Label::new("d/blake3").unwrap(), d),
    ]);

    let agent = AgentId::new("tool:rust-smysl").unwrap();
    let attest = Attest::new(agent.clone(), Rung::Document, Hlc::zero(agent));
    let staged = stage::prepare(
        &Store::new(),
        vec![prerequisite, decision],
        relations,
        labels,
        &attest,
    );
    assert!(staged.report.fail_on(Severity::Error).is_ok());

    // The staged batch is records; a store built from them resolves its labels.
    let store = Store::from_records(staged.records());
    let blake3 = resolve_label(&store, &Label::new("d/blake3").unwrap()).unwrap();
    assert_eq!(blake3, d);

    // What loses a premise if the prerequisite turns out to be false.
    assert_eq!(dependents_via(&store, p, &EdgeSet::premises()), vec![d]);
    // --- end manual example ---
}
