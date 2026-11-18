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

/// Rule TQ (§11.4) on materialised windows: a quote verifies only with the evidence's own value,
/// in its own spelling, from a unit the hypothesis is grounded on.
#[test]
fn rule_tq_holds_quotes_to_the_evidence() {
    use smysl::series::{MemBackend, Sample, SeriesInfo, Substrate};
    use smysl::tel::evidence;
    use smysl::{Code, Record, Uid};
    use smysl_core::Decimal;

    let mut sub = Substrate::new(MemBackend::default());
    let (s, i) = (Uid::from_bytes([1; 32]), Uid::from_bytes([2; 32]));
    let info = SeriesInfo {
        scheme: "prom".into(),
        key: "lat".into(),
        class: "gauge".into(),
    };
    let samples: Vec<Sample> = ["0.2", "0.4", "0.9", "0.3"]
        .iter()
        .enumerate()
        .map(|(n, v)| {
            Sample::new(n as u64 * 1_000, None, Decimal::parse_token(v).unwrap(), 0).unwrap()
        })
        .collect();
    sub.add(&s, &i, &info, &samples).unwrap();
    let window = evidence::materialise(&sub, &s, &i, 0, 10_000).unwrap().unit;
    let qs: Vec<Decimal> = ["0.5", "0.9"]
        .iter()
        .map(|q| Decimal::parse_token(q).unwrap())
        .collect();
    let sketch = evidence::materialise_sketched(
        &sub,
        &s,
        &i,
        0,
        10_000,
        Decimal::parse_token("0.01").unwrap(),
        &qs,
    )
    .unwrap()
    .unit;
    let (w, k) = (canonical_uid(&window), canonical_uid(&sketch));
    let q1 = smysl::schemas::tel::quoted_value(sketch.payload.as_deref().unwrap(), "q", Some(1))
        .unwrap();

    let hyp = |quotes: &[String], grounds: Vec<Uid>| {
        investigate::hypothesis("latency rose", quotes, Some(s), grounds)
    };
    let w606 = |h: &smysl::UnitCore| {
        let store = Store::from_records(vec![
            Record::SchemaDecl(smysl::schemas::tel::decl()),
            Record::Unit(window.clone()),
            Record::Unit(sketch.clone()),
            Record::Unit(h.clone()),
        ]);
        smysl::check(&store, smysl::CheckOptions::default())
            .diagnostics
            .into_iter()
            .filter(|d| d.code == Code::W606)
            .map(|d| d.message)
            .collect::<Vec<_>>()
    };
    let wq = |field: &str| format!("{}#{field}", w.canonical());

    // Verifies: the max as the window holds it, and a sketch's quantile by index.
    let ok = hyp(
        &[
            wq("max=0.9"),
            wq("count=4"),
            format!("{}#q[1]={q1}", k.canonical()),
        ],
        vec![w, k],
    );
    assert!(w606(&ok).is_empty(), "{:?}", w606(&ok));
    // A wrong value, a non-canonical spelling, an uncited unit, a missing field.
    for (quotes, grounds, why) in [
        (vec![wq("max=2.3")], vec![w], "holds 0.9, not 2.3"),
        (vec![wq("max=0.90")], vec![w], "holds 0.9, not 0.90"),
        (
            vec![wq("max=0.9")],
            vec![k],
            "not among the hypothesis's grounds",
        ),
        (vec![wq("p99=0.9")], vec![w], "no number at `p99`"),
        (
            vec![format!("{}#q[7]=1", k.canonical())],
            vec![k],
            "no number at `q[7]`",
        ),
        (vec!["nonsense".to_string()], vec![w], "has no `#`"),
    ] {
        let got = w606(&hyp(&quotes, grounds));
        assert_eq!(got.len(), 1, "{quotes:?}: {got:?}");
        assert!(got[0].contains(why), "{quotes:?}: {}", got[0]);
        assert!(got[0].contains("at most speculative"));
    }
    assert_eq!(
        smysl::schemas::tel::tq_capped(smysl::Status::Inferred, 1),
        smysl::Status::Speculative
    );
    assert_eq!(
        smysl::schemas::tel::tq_capped(smysl::Status::Inferred, 0),
        smysl::Status::Inferred
    );
}
