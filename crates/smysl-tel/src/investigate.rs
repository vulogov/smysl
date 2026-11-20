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
//! - an investigation is `cited`: a person's (or an agent's) statement that a question was
//!   opened, and when, attested as authored at the document rung, which rule T caps at `cited`.
//!   Closing it is a new investigation unit that supersedes the open one;
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
        Status::Cited,
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
        Status::Cited,
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
        assert_eq!(o.status, Status::Cited);
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

/// A quote that did not verify, and why (rule TQ).
pub type FailedQuote = (String, String);

/// A hypothesis in an investigation: its uid, its effective status after rule TQ, and the quotes
/// that failed.
pub type Reported = (Uid, Status, Vec<FailedQuote>);

/// What one investigation holds, and what it missed (§11.2, §22.2): the numbers a dossier
/// reports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub investigation: Option<Uid>,
    pub question: String,
    pub opened_ms: u64,
    pub closed_ms: Option<u64>,
    /// How far before the opening "unexplored" looked.
    pub lookback_ms: u64,
    /// Hypotheses `during` it, with their effective status after rule TQ and their failed
    /// quotes.
    pub hypotheses: Vec<Reported>,
    pub probes: Vec<Uid>,
    pub actions: Vec<Uid>,
    /// Everything it consulted.
    pub consulted: Vec<Uid>,
    /// Consulted and cited by no hypothesis in it: looked at, and left out of every
    /// explanation.
    pub unused: Vec<Uid>,
    /// Window and change units in its time, about a series a hypothesis targets, never
    /// consulted: evidence about the question that nobody looked at.
    pub unexplored: Vec<Uid>,
}

fn payload_field(u: &UnitCore, key: &str) -> Option<HValue> {
    smysl_core::surface::payload_to_object(u.payload.as_deref()?)
        .ok()?
        .get(key)
        .map(|v| v.value.clone())
}

fn payload_uid(u: &UnitCore, key: &str) -> Option<Uid> {
    match payload_field(u, key)? {
        HValue::Str(s) => s.parse().ok(),
        _ => None,
    }
}

fn payload_ms(u: &UnitCore, key: &str) -> Option<u64> {
    match payload_field(u, key)? {
        HValue::Int(n) => u64::try_from(n).ok(),
        _ => None,
    }
}

/// The report of `investigation` over a record set: `units` by uid, `relations`, and the
/// effective status each unit has after retraction and rule M (`effective`). The investigation
/// may be the open unit or a closed one; both name the same edges when the caller passes the
/// open one, which the edges point at.
///
/// "Unexplored" looks from `lookback_ms` before the investigation was opened to its close: a
/// cause usually comes before anyone asks about it, and how far before is the caller's to say.
///
/// A function of the record set, like rule TQ: it reads payloads, never the substrate.
pub fn report<'a>(
    investigation: Uid,
    units: &std::collections::BTreeMap<Uid, &'a UnitCore>,
    relations: &[&'a Relation],
    effective: impl Fn(&Uid) -> Status,
    lookback_ms: u64,
) -> Report {
    use std::collections::BTreeSet;
    let mut out = Report {
        investigation: Some(investigation),
        lookback_ms,
        ..Report::default()
    };
    if let Some(inv) = units.get(&investigation) {
        if let Some(HValue::Str(q)) = payload_field(inv, "tel:question") {
            out.question = q;
        }
        out.opened_ms = payload_ms(inv, "tel:opened_ms").unwrap_or(0);
    }
    // A closed investigation superseding this one gives the close.
    for r in relations {
        if r.kind == RelKind::Supersedes && r.to == investigation {
            if let Some(c) = units
                .get(&r.from)
                .and_then(|u| payload_ms(u, "tel:closed_ms"))
            {
                out.closed_ms = Some(out.closed_ms.map_or(c, |p: u64| p.max(c)));
            }
        }
    }
    let during = RelKind::Extension(tel::DURING.to_string());
    let consulted_kind = RelKind::Extension(tel::CONSULTED.to_string());
    let mut consulted: BTreeSet<Uid> = BTreeSet::new();
    let mut members: BTreeSet<Uid> = BTreeSet::new();
    for r in relations {
        if r.kind == during && r.to == investigation {
            members.insert(r.from);
        }
        if r.kind == consulted_kind && r.from == investigation {
            consulted.insert(r.to);
        }
    }
    let mut cited: BTreeSet<Uid> = BTreeSet::new();
    let mut targets: BTreeSet<Uid> = BTreeSet::new();
    let payload_of = |u: &Uid| units.get(u).map(|c| c.payload.clone().unwrap_or_default());
    for m in &members {
        let Some(u) = units.get(m) else { continue };
        match u.schema.as_str() {
            t if t == tel::HYPOTHESIS => {
                let failed = tel::rule_tq_violations(u, payload_of);
                let status = tel::tq_capped(effective(m), failed.len());
                out.hypotheses.push((*m, status, failed));
                cited.extend(u.grounds.iter().copied());
                for q in tel::quotes_of(u) {
                    if let Ok(p) = tel::parse_quote(&q) {
                        cited.insert(p.unit);
                    }
                }
                if let Some(t) = payload_uid(u, "tel:target") {
                    targets.insert(t);
                }
            }
            t if t == tel::PROBE => out.probes.push(*m),
            t if t == tel::ACTION => out.actions.push(*m),
            _ => {}
        }
    }
    out.unused = consulted
        .iter()
        .filter(|u| !cited.contains(u))
        .copied()
        .collect();
    out.consulted = consulted.iter().copied().collect();

    // Unexplored: windows and changes about a targeted series, in the investigation's time.
    let end = out.closed_ms.unwrap_or(u64::MAX);
    for (uid, u) in units {
        if consulted.contains(uid) {
            continue;
        }
        let (about, when) = match u.schema.as_str() {
            t if t == tel::WINDOW => {
                let Some(w) = u
                    .payload
                    .as_deref()
                    .and_then(|p| tel::window_fields(p).ok())
                else {
                    continue;
                };
                (
                    w.series.as_deref().and_then(|s| s.parse::<Uid>().ok()),
                    w.from_ms,
                )
            }
            t if t == tel::CHANGE => (
                payload_uid(u, "tel:object"),
                u.source.as_ref().and_then(|s| s.observed),
            ),
            _ => continue,
        };
        let (Some(about), Some(when)) = (about, when) else {
            continue;
        };
        let from = out.opened_ms.saturating_sub(lookback_ms);
        if targets.contains(&about) && (from..=end).contains(&when) {
            out.unexplored.push(*uid);
        }
    }
    out
}
