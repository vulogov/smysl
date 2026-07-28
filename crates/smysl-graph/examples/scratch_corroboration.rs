//! Scratch generator for the manual (Ch15): two stores, identical evidence and claim, that
//! differ only in how many independent agents attested the evidence. Not part of the crate;
//! run with `cargo run -p smysl-graph --example scratch_corroboration`.

use smysl_core::{
    canonical_uid, to_cbor_seq, AgentId, Attestation, Hlc, KernelType, Op, Record, Rung,
    SourceKind, SourceRef, Status, UnitCoreBuilder,
};

fn main() {
    let evidence = UnitCoreBuilder::new(
        KernelType::Evidence,
        "pool wait rose sevenfold",
        Status::Measured,
    )
    .source(SourceRef::new(
        SourceKind::Metric,
        "pool.wait_ms{shard=eu-west}",
    ))
    .build()
    .unwrap();
    let ue = canonical_uid(&evidence);

    let claim = UnitCoreBuilder::new(KernelType::Claim, "the pool is saturated", Status::Inferred)
        .grounds([ue])
        .build()
        .unwrap();
    let uc = canonical_uid(&claim);

    let one_agent = AgentId::new("model:openai/gpt-4").unwrap();
    let other_agent = AgentId::new("model:anthropic/claude").unwrap();

    let att = |agent: &AgentId| {
        Attestation::new(
            ue,
            agent.clone(),
            Op::Authored,
            Rung::Model,
            Hlc::zero(agent.clone()),
        )
    };

    // One attester.
    let one = vec![
        Record::Unit(evidence.clone()),
        Record::Unit(claim.clone()),
        Record::Attestation(att(&one_agent)),
    ];
    // Two independent attesters (no shared parents recorded on either attestation, so the
    // ancestry check treats them as disjoint and counts both).
    let two = vec![
        Record::Unit(evidence.clone()),
        Record::Unit(claim.clone()),
        Record::Attestation(att(&one_agent)),
        Record::Attestation(att(&other_agent)),
    ];

    std::fs::write("corrob-one.cbor", to_cbor_seq(&one)).unwrap();
    std::fs::write("corrob-two.cbor", to_cbor_seq(&two)).unwrap();
    eprintln!("evidence uid: {ue}");
    eprintln!("claim uid:    {uc}");
}
