//! Investigations (RFC SMYSL-3 §11, §24, TL-P6): the ledger an on-call rotation keeps.
//!
//! ```text
//!  x.tel/investigation  ◄── View (root = the investigation; what a TUI opens)
//!     │ x.tel/consulted ──► window, change, alert, chat units …   (what was looked at)
//!     │ x.tel/during    ◄── hypothesis, probe, action             (what happened in it)
//!  x.tel/hypothesis ── grounds ──► evidence units
//!                   ── x.tel/explains ──► change / anomaly / alert / episode
//! ```
//!
//! These are builders: each returns the unit or edge, and the caller attests and commits it,
//! because who did it (a human handle, a model agent by provider and model hash) is the
//! attestation's to say, not the unit's (§24).
//!
//! **Statuses** (§3, §11.3):
//! - an investigation is `measured`: that a question was opened, when, is a recorded event;
//!   closing it is a new investigation unit that supersedes the open one;
//! - a probe or an action is `measured`: that the command ran, and what it printed;
//! - a hypothesis is `inferred` when it has grounds and `speculative` when it has none, and
//!   rule TQ (§11.4) may cap it lower when its numeric quotes do not verify.

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::{HValue, Spanned};
use smysl_core::{
    canonical_uid, RelKind, Relation, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder, View, ViewId,
};

use crate::scrape::{ms, payload};

fn build(
    ty: &str,
    gist: &str,
    status: Status,
    source: Option<SourceRef>,
    grounds: Vec<Uid>,
    fields: &[(&str, HValue)],
) -> UnitCore {
    let mut b = UnitCoreBuilder::new(
        SchemaId::Extension(ty.to_string()),
        tel::gist_of(gist),
        status,
    )
    .grounds(grounds)
    .payload(payload(fields));
    if let Some(s) = source {
        b = b.source(s);
    }
    b.build()
        .expect("an investigation unit is well formed by construction")
}

fn tool(reference: &str, at_ms: u64) -> SourceRef {
    SourceRef::new(SourceKind::Tool, reference.to_string()).observed_at(at_ms)
}

/// An open `x.tel/investigation`: `question`, opened at `opened_ms`, in `rotation`.
pub fn open(question: &str, opened_ms: u64, rotation: Option<&str>) -> UnitCore {
    let mut fields = vec![
        ("tel:question", HValue::Str(question.to_string())),
        ("tel:opened_ms", ms(opened_ms)),
    ];
    if let Some(r) = rotation {
        fields.push(("tel:rotation", HValue::Str(r.to_string())));
    }
    build(
        tel::INVESTIGATION,
        question,
        Status::Measured,
        Some(tool("smysl-tel:investigation", opened_ms)),
        Vec::new(),
        &fields,
    )
}

/// The same investigation, closed at `closed_ms`, and the `supersedes` edge from it to the open
/// one. An investigation is a unit, so it is not changed: it is superseded.
pub fn close(open_unit: &UnitCore, closed_ms: u64) -> Option<(UnitCore, Relation)> {
    let o = smysl_core::surface::payload_to_object(open_unit.payload.as_deref()?).ok()?;
    let text = |k: &str| match o.get(k).map(|v| &v.value) {
        Some(HValue::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let question = text("tel:question")?;
    let opened = match o.get("tel:opened_ms")?.value {
        HValue::Int(n) => u64::try_from(n).ok()?,
        _ => return None,
    };
    let mut fields = vec![
        ("tel:question", HValue::Str(question.clone())),
        ("tel:opened_ms", ms(opened)),
        ("tel:closed_ms", ms(closed_ms)),
    ];
    if let Some(r) = text("tel:rotation") {
        fields.push(("tel:rotation", HValue::Str(r)));
    }
    let closed = build(
        tel::INVESTIGATION,
        &format!("closed: {question}"),
        Status::Measured,
        Some(tool("smysl-tel:investigation", closed_ms)),
        Vec::new(),
        &fields,
    );
    let edge = Relation::new(
        RelKind::Supersedes,
        canonical_uid(&closed),
        canonical_uid(open_unit),
    );
    Some((closed, edge))
}

/// An `x.tel/hypothesis`: `claim`, with the numeric `quotes` it makes (§11.4, as
/// `<uid>#<field>=<value>`), what it is about (`target`), and its `grounds`.
///
/// `inferred` with grounds and `speculative` without (§3). Rule TQ is applied when the store is
/// read, not here: whether a quote verifies depends on the records, and the records can grow.
pub fn hypothesis(
    claim: &str,
    quotes: &[String],
    target: Option<Uid>,
    grounds: Vec<Uid>,
) -> UnitCore {
    let status = if grounds.is_empty() {
        Status::Speculative
    } else {
        Status::Inferred
    };
    let mut fields = vec![("tel:claim", HValue::Str(claim.to_string()))];
    if !quotes.is_empty() {
        let span = smysl_core::Span::new(0, 0);
        fields.push((
            "tel:quotes",
            HValue::Array(
                quotes
                    .iter()
                    .map(|q| Spanned::new(HValue::Str(q.clone()), span))
                    .collect(),
            ),
        ));
    }
    if let Some(t) = target {
        fields.push(("tel:target", HValue::Str(t.canonical())));
    }
    build(tel::HYPOTHESIS, claim, status, None, grounds, &fields)
}

/// An `x.tel/probe`: a read-only observation, `what` was run at `at_ms`, its output a part
/// (`output`, a tid), already redacted by the caller (§11.3).
pub fn probe(what: &str, at_ms: u64, output: Option<&str>) -> UnitCore {
    let mut fields = vec![
        ("tel:what", HValue::Str(what.to_string())),
        ("tel:at_ms", ms(at_ms)),
    ];
    if let Some(o) = output {
        fields.push(("tel:output", HValue::Str(o.to_string())));
    }
    build(
        tel::PROBE,
        &format!("probe: {what}"),
        Status::Measured,
        Some(tool(&format!("probe:{what}"), at_ms)),
        Vec::new(),
        &fields,
    )
}

/// An `x.tel/action`: something done to the system, `what` at `at_ms`; `confirmed_by` the
/// human who confirmed an unsafe command (§11.3, §24).
pub fn action(
    what: &str,
    at_ms: u64,
    confirmed_by: Option<&str>,
    output: Option<&str>,
) -> UnitCore {
    let mut fields = vec![
        ("tel:what", HValue::Str(what.to_string())),
        ("tel:at_ms", ms(at_ms)),
    ];
    if let Some(c) = confirmed_by {
        fields.push(("tel:confirmed_by", HValue::Str(c.to_string())));
    }
    if let Some(o) = output {
        fields.push(("tel:output", HValue::Str(o.to_string())));
    }
    build(
        tel::ACTION,
        &format!("action: {what}"),
        Status::Measured,
        Some(tool(&format!("action:{what}"), at_ms)),
        Vec::new(),
        &fields,
    )
}

fn edge(kind: &str, from: Uid, to: Uid) -> Relation {
    Relation::new(RelKind::Extension(kind.to_string()), from, to)
}

/// The investigation looked at `unit` (§11.2): one edge per unit looked at.
pub fn consulted(investigation: Uid, unit: Uid) -> Relation {
    edge(tel::CONSULTED, investigation, unit)
}

/// `unit` (a hypothesis, probe or action) happened in the investigation.
pub fn during(unit: Uid, investigation: Uid) -> Relation {
    edge(tel::DURING, unit, investigation)
}

/// The hypothesis offers an explanation of `target`.
pub fn explains(hypothesis: Uid, target: Uid) -> Relation {
    edge(tel::EXPLAINS, hypothesis, target)
}

/// The View an investigation is opened as (D-T8): rooted at it, requiring `x.tel/v1`.
pub fn view(investigation: Uid) -> View {
    let short = smysl_core::ids::base32(investigation.as_bytes(), 26);
    let mut v = View::new(
        ViewId::new(format!("v/investigation-{short}")).expect("a valid view id"),
        "investigation",
    )
    .with_roots([investigation]);
    v.requires.insert(SchemaId::Extension(tel::ID.to_string()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_investigation_closes_by_supersession() {
        let o = open("why is checkout slow?", 1_000, Some("sre-eu"));
        assert_eq!(o.status, Status::Measured);
        let (c, e) = close(&o, 9_000).unwrap();
        assert_eq!(e.kind, RelKind::Supersedes);
        assert_eq!((e.from, e.to), (canonical_uid(&c), canonical_uid(&o)));
        assert_ne!(canonical_uid(&c), canonical_uid(&o));
        // Opening the same question at the same instant is one unit, on any peer.
        assert_eq!(
            canonical_uid(&open("why is checkout slow?", 1_000, Some("sre-eu"))),
            canonical_uid(&o)
        );
    }

    #[test]
    fn a_hypothesis_without_grounds_is_speculative() {
        let g = Uid::from_bytes([1; 32]);
        assert_eq!(
            hypothesis("the deploy did it", &[], None, vec![g]).status,
            Status::Inferred
        );
        assert_eq!(
            hypothesis("the deploy did it", &[], None, vec![]).status,
            Status::Speculative
        );
        let q = hypothesis(
            "p99 went to 0.9",
            &[format!("{}#q[2]=0.9", g.canonical())],
            Some(g),
            vec![g],
        );
        let o = smysl_core::surface::payload_to_object(q.payload.as_deref().unwrap()).unwrap();
        assert!(matches!(
            o.get("tel:quotes").unwrap().value,
            HValue::Array(_)
        ));
    }

    #[test]
    fn probes_and_actions_are_measured_and_the_view_is_rooted() {
        assert_eq!(
            probe("kubectl get pods", 5, Some("t3:x")).status,
            Status::Measured
        );
        let a = action(
            "kubectl rollout undo deploy/checkout",
            6,
            Some("alice"),
            None,
        );
        assert_eq!(a.status, Status::Measured);
        let inv = canonical_uid(&open("q", 1, None));
        let v = view(inv);
        assert!(v.roots.contains(&inv));
        assert!(v.id.as_str().starts_with("v/investigation-"));
        assert_eq!(consulted(inv, inv).kind.as_str(), tel::CONSULTED);
    }
}
