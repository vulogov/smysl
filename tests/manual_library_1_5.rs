//! Chapter 28's 1.5 example, compiled and run. The book prints this code; if it stops building,
//! this fails rather than the page going stale.

#![cfg(feature = "stage")]

#[test]
fn read_a_repository_whose_dependencies_are_edges() {
    // --- begin manual example ---
    use smysl::stage::{self, Attest};
    use smysl::{
        canonical_uid, label_index, labels_of, quote_support_span, rests_on, AgentId, Attestation,
        Attesting, Bm25, EdgeSet, Hlc, KernelType, Label, Query, QuoteSupport, RelKind, Relation,
        Retriever, Rung, Severity, SourceKind, SourceRef, Status, Store, Tokenizer, Uid,
        UnitCoreBuilder,
    };
    use std::collections::BTreeMap;

    let file = "src/pool.rs@90ec2f781421";
    let review = "The pool is required to be drained before a reconfigure.";

    // Units the tool builds itself, anchored to the file at the commit it read.
    let anchor = SourceRef::new(SourceKind::Doc, file);
    let prerequisite = UnitCoreBuilder::new(
        KernelType::Claim,
        "the pool is required to be drained before a reconfigure",
        Status::Cited,
    )
    .source(anchor.clone())
    .build()
    .unwrap();
    let decision = UnitCoreBuilder::new(
        KernelType::Decision,
        "drain the pool in reconfigure()",
        Status::Cited,
    )
    .source(anchor)
    .build()
    .unwrap();
    let (p, d) = (canonical_uid(&prerequisite), canonical_uid(&decision));

    // Two hands: the tool proposed the units, a reviewer asserted the edge between them.
    struct TwoHands {
        tool: Attest,
        reviewer: Attest,
    }
    impl Attesting for TwoHands {
        fn for_unit(&self, uid: Uid) -> Attestation {
            self.tool.for_unit(uid)
        }
        fn for_relation(&self, rid: Uid) -> Attestation {
            self.reviewer.for_relation(rid)
        }
    }

    let tool = AgentId::new("tool:rust-smysl").unwrap();
    let person = AgentId::new("human:reviewer").unwrap();
    let attest = TwoHands {
        tool: Attest::new(tool.clone(), Rung::Document, Hlc::zero(tool.clone())),
        reviewer: Attest::new(person.clone(), Rung::Document, Hlc::zero(person.clone())),
    };

    let staged = stage::prepare_attested(
        &Store::new(),
        vec![prerequisite, decision],
        vec![Relation::new(RelKind::Conditions, p, d)],
        BTreeMap::from([
            (Label::new("c/drain-first").unwrap(), p),
            (Label::new("d/drain").unwrap(), d),
        ]),
        Vec::new(),
        &attest,
    );
    assert!(staged.report.fail_on(Severity::Error).is_ok());
    let store = Store::from_records(staged.records());

    // Who stands behind what, without knowing whether a uid names a unit or an edge.
    let rid = Relation::new(RelKind::Conditions, p, d).uid();
    assert!(store.attested_by(&rid).contains(&&person));
    assert!(store.attested_by(&d).contains(&&tool));
    assert_eq!(store.attestations_of(&d).len(), 1);

    // What the decision rests on, over edges a producer chose — `conditions`, not `grounds`.
    assert_eq!(rests_on(&store, d, &EdgeSet::premises()), vec![p]);

    // uid back to the name a person reads. One lookup, or the whole map for a run that
    // renders every uid it prints.
    assert_eq!(labels_of(&store, &d), vec![Label::new("d/drain").unwrap()]);
    assert_eq!(label_index(&store).len(), 2);

    // Every unit recorded about this file, whichever commit it was recorded at.
    assert_eq!(store.units_with_source_prefix("src/pool.rs").len(), 2);

    // Where in the review the evidence for the prerequisite is, in bytes a reader can see.
    let (support, span) = quote_support_span("pool is required to be drained", review);
    assert_eq!(support, QuoteSupport::Present);
    assert_eq!(&review[span.unwrap()], "pool is required to be drained");

    // Retrieval over a candidate set, with the English fold on: `require` finds `required`.
    let hits = Bm25::index_with(&store, Tokenizer::folding())
        .search(&Query::new("require drain", 10).within([d]));
    assert_eq!(hits.iter().map(|h| h.uid).collect::<Vec<_>>(), vec![d]);
    // --- end manual example ---
}
