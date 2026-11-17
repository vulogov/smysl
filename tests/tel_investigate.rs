//! Investigation units through staging and `check` (RFC SMYSL-3 §11, TL-P6): what the builders
//! make is what a store accepts.

#![cfg(feature = "tel")]

use smysl::tel::investigate::{self, consulted, during, explains};
use smysl::{canonical_uid, AgentId, Hlc, Rung, Store};

#[test]
fn an_investigation_stages_clean() {
    let inv = investigate::open("why is checkout slow?", 1_000, Some("sre-eu"));
    let probe = investigate::probe("kubectl get pods -n shop", 2_000, None);
    let hyp = investigate::hypothesis(
        "the 14:02 deploy slowed checkout",
        &[],
        Some(canonical_uid(&probe)),
        vec![canonical_uid(&probe)],
    );
    let (closed, sup) = investigate::close(&inv, 9_000).unwrap();
    let (i, p, h) = (
        canonical_uid(&inv),
        canonical_uid(&probe),
        canonical_uid(&hyp),
    );
    let relations = vec![
        consulted(i, p),
        during(p, i),
        during(h, i),
        explains(h, p),
        sup,
    ];
    let agent = AgentId::new("human:alice").unwrap();
    let attest =
        smysl::stage::Attest::new(agent.clone(), Rung::Computed, Hlc::new(9_000, 0, agent));
    let staged = smysl::stage::prepare_declared(
        &Store::new(),
        vec![inv, probe, hyp, closed],
        relations,
        Default::default(),
        vec![smysl::schemas::tel::decl()],
        &attest,
    );
    assert!(!staged.has_errors(), "{:?}", staged.report.diagnostics);
    let errors: Vec<_> = staged
        .report
        .diagnostics
        .iter()
        .filter(|d| d.code.severity() == smysl::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let _view = investigate::view(i);
}
