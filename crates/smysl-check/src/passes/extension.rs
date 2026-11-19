//! Pass 9 - extension and conformance (§2.2, §17).
//!
//! Rule X is the reason a store written by someone who knew more than you is still
//! readable. With only the kernel, a consumer can identify, merge, summarise, weigh,
//! status-check, and traverse an unknown unit; only `payload` interpretation and
//! full-fidelity rendering are lost.
//!
//! The negotiation that follows is three-valued, and the third value matters: `full` when
//! every required schema is implemented, `degraded` when one is not, and **refuse** when
//! the kernel major is absent. A consumer MUST NOT silently degrade at that last step.

use std::collections::BTreeSet;

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{Fidelity, RelKind, SchemaId};
use smysl_graph::{HotKind, StoreRead};

/// What a consumer implements, for `check --as` (§23.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConsumerProfile {
    pub name: String,
    /// Extension schemas this consumer understands. The kernel is always implied.
    pub implemented: BTreeSet<SchemaId>,
    /// The kernel major this consumer implements.
    pub kernel_major: u32,
}

impl Default for ConsumerProfile {
    fn default() -> ConsumerProfile {
        ConsumerProfile {
            name: "kernel-only".into(),
            implemented: BTreeSet::new(),
            kernel_major: smysl_core::KERNEL_MAJOR,
        }
    }
}

impl ConsumerProfile {
    pub fn new(name: impl Into<String>) -> ConsumerProfile {
        ConsumerProfile {
            name: name.into(),
            ..ConsumerProfile::default()
        }
    }

    pub fn implementing(mut self, s: impl IntoIterator<Item = SchemaId>) -> ConsumerProfile {
        self.implemented = s.into_iter().collect();
        self
    }

    /// Whether this consumer can interpret a unit of this schema at full fidelity.
    pub fn understands(&self, schema: &SchemaId) -> bool {
        schema.is_kernel() || self.implemented.contains(schema)
    }
}

/// What a consumer gets from a store, per unit and overall.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FidelityReport {
    pub overall: Fidelity,
    /// Units this consumer would read at reduced fidelity, with the schema responsible.
    pub degraded: Vec<(smysl_core::Uid, SchemaId)>,
    /// Schemas the store requires that the consumer does not implement.
    pub missing: BTreeSet<SchemaId>,
}

pub fn run(
    store: &(impl StoreRead + ?Sized),
    profile: Option<&ConsumerProfile>,
    report: &mut Report,
) {
    // `SMY-E012` - an extension that redefines a kernel type or relation kind.
    for r in store.hot_records(HotKind::SchemaDecls) {
        if let smysl_core::Record::SchemaDecl(d) = r {
            if d.redefines_kernel() {
                report.push(Diagnostic::new(Code::E012).with_message(format!(
                    "schema {} redefines a kernel type or relation kind",
                    d.id
                )));
            }
        }
    }

    // `SMY-W013` - an unknown relation kind, treated as `elaborates` for closure.
    let mut unknown_kinds: BTreeSet<String> = BTreeSet::new();
    for rel in store.relations() {
        if !rel.kind.is_kernel() && !declared(store, &rel.kind) {
            unknown_kinds.insert(rel.kind.as_str().to_string());
        }
    }
    for k in unknown_kinds {
        report.push(Diagnostic::new(Code::W013).with_message(format!(
            "relation kind `{k}` is undeclared; treated as elaborates"
        )));
    }

    // `SMY-W014` - a whole record type this build does not know.
    //
    // The code was in the registry from 0.1 with no emission site anywhere, so an unknown
    // record was preserved in perfect silence: a reader was never told the store held
    // something it could not interpret. Preservation is rule X working; saying nothing
    // about it is how a reader comes to believe they have seen the whole document.
    for r in store.hot_records(HotKind::Unknown) {
        if let smysl_core::Record::Unknown { code, payload } = r {
            // Record 16 is not merely unknown but reserved (SMYSL-3 D-T6): `SMY-E612` below
            // says so, and saying `W014` as well would describe one record twice.
            if *code == smysl_core::types::record::code::HOLD {
                continue;
            }
            report.push(Diagnostic::new(Code::W014).with_message(format!(
                "record type {code} is not known to this build; \
                 {} byte(s) preserved verbatim, skipped semantically",
                payload.len()
            )));
        }
    }

    // `SMY-W432` - a payload key named `lang`, which is now unit core key 9 spelled unquoted.
    //
    // The lint was the migration, and the migration has happened: FC-1 activated in TX-P5 step 1,
    // so a bare `lang:` on a unit *is* the core key and a payload key of that name can only
    // arrive quoted or from CBOR. What the warning says has therefore changed from "this will
    // move" to "you have the core key's name in your payload" — the field exists now, and a unit
    // carrying the language in two hashed places is a unit whose language nothing can agree on.
    // The code stays rather than being retired: a store written before this release still holds
    // such units, and the advice is still the advice.
    for r in store.records() {
        if let smysl_core::Record::Unit(u) = &*r {
            if let Some(bytes) = &u.payload {
                if let Ok(o) = smysl_core::surface::payload_to_object(bytes) {
                    if o.iter().any(|(k, _)| k.value == "lang") {
                        report.push(Diagnostic::new(Code::W432).with_message(
                            "a payload key named `lang` is unit core key 9 spelled unquoted \
                             (1.10): the language belongs in the core key, and `\"lang\":` is \
                             how a payload key of that name is written",
                        ));
                    }
                }
            }
        }
    }

    reserved(store, report);
    entity_like(store, report);
    windows(store, report);
    quotes(store, report);
    unmeasured(store, report);

    // `SMY-W409` - an enumeration *code* this build does not know, which is W014's argument
    // one level down. 1.9 opened four enumerations so that an unrecognised code no longer
    // fails the decode, and 1.10 opened the fifth, `admission`; preserving it silently would
    // repeat the mistake W014 was added to fix, where a reader is given a document it cannot
    // fully interpret and told nothing.
    //
    // The code is reported, not the value: `Unknown` is what the build calls it, and the raw
    // byte is the only thing that identifies which future kind was meant.
    for r in store.records() {
        match &*r {
            smysl_core::Record::Unit(u) => {
                if let Some(s) = u.source.as_ref() {
                    if s.kind == smysl_core::SourceKind::Unknown {
                        report.push(Diagnostic::new(Code::W409).with_message(format!(
                            "source kind {} is not known to this build; \
                             preserved verbatim, treated as unknown",
                            s.kind_code()
                        )));
                    }
                }
            }
            smysl_core::Record::Thread(th) => {
                if th.schema == smysl_core::ThreadSchema::Unknown {
                    report.push(Diagnostic::new(Code::W409).with_message(format!(
                        "thread schema {} is not known to this build; \
                         preserved verbatim, and the thread defines no roles",
                        th.schema_code()
                    )));
                }
                for st in &th.steps {
                    if st.role == smysl_core::Role::Unknown {
                        report.push(Diagnostic::new(Code::W409).with_message(format!(
                            "step role {} is not known to this build; \
                             preserved verbatim, treated as unknown",
                            st.role_code()
                        )));
                    }
                }
            }
            smysl_core::Record::View(v)
                if v.granularity.admission == smysl_core::Admission::Unknown =>
            {
                report.push(Diagnostic::new(Code::W409).with_message(format!(
                    "admission {} is not known to this build; preserved verbatim, and the \
                     single-assertion check does not run for this profile",
                    v.granularity.admission_code()
                )));
            }
            smysl_core::Record::Contention(c)
                if c.detected.kind == smysl_core::DetectionKind::Unknown =>
            {
                report.push(Diagnostic::new(Code::W409).with_message(format!(
                    "detection kind {} is not known to this build; \
                     preserved verbatim, reported as a plain disagreement",
                    c.detected.kind_code()
                )));
            }
            _ => {}
        }
    }

    // `SMY-W010` - a type this *build* does not know, which is stronger than the
    // consumer-profile case below and does not depend on one being supplied. A unit whose
    // type arrived from a later version decodes and round-trips (rule X), but nothing here
    // can interpret it, and a reader who is not told that has been misled by silence.
    for (uid, unit) in store.units() {
        if let smysl_core::SchemaId::UnknownKernel(name) = &unit.core.schema {
            report.push(Diagnostic::on(Code::W010, *uid).with_message(format!(
                "unit type `{name}` is not known to this build; \
                 preserved verbatim, interpretation lost"
            )));
        }
    }

    // `SMY-W010` - a schema this consumer does not implement.
    if let Some(p) = profile {
        for (uid, unit) in store.units() {
            if !p.understands(&unit.core.schema) {
                report.push(Diagnostic::on(Code::W010, *uid).with_message(format!(
                    "schema {} is not implemented by `{}`; payload preserved, interpretation lost",
                    unit.core.schema, p.name
                )));
            }
        }
    }
}

/// Whether a `SchemaDecl` in this store declares the kind.
fn declared(store: &(impl StoreRead + ?Sized), kind: &RelKind) -> bool {
    store.hot_records(HotKind::SchemaDecls).any(|r| match r {
        smysl_core::Record::SchemaDecl(d) => d.relations.contains(kind),
        _ => false,
    })
}

/// What `check --as` reports (§23.1).
pub fn fidelity(store: &(impl StoreRead + ?Sized), profile: &ConsumerProfile) -> FidelityReport {
    let mut degraded = Vec::new();
    for (uid, unit) in store.units() {
        if !profile.understands(&unit.core.schema) {
            degraded.push((*uid, unit.core.schema.clone()));
        }
    }

    let mut missing: BTreeSet<SchemaId> = BTreeSet::new();
    let mut kernel_ok = true;
    for v in store.views() {
        for r in &v.requires {
            match r.kernel_major() {
                Some(m) if m != profile.kernel_major => kernel_ok = false,
                Some(_) => {}
                None if !profile.understands(r) => {
                    missing.insert(r.clone());
                }
                None => {}
            }
        }
    }
    for (_, s) in &degraded {
        missing.insert(s.clone());
    }

    let overall = if !kernel_ok {
        Fidelity::Refuse
    } else if missing.is_empty() {
        Fidelity::Full
    } else {
        Fidelity::Degraded
    };

    FidelityReport {
        overall,
        degraded,
        missing,
    }
}

/// `SMY-E607` - a series or instrument unit that breaks SMYSL-3 §4.1's entity-like rule.
///
/// The rule is `x.text/v1` rule 7's argument for telemetry: a series is what is measured, not
/// when, so its identity is a function of its identity tuple and one series seen by fifty
/// scrapes is one unit. Give it an `observed` and it becomes fifty. The implementation is
/// `smysl_core::schemas::tel::entity_like_violations`, shared with the producer, for the reason
/// the library pass gives for sharing rule 7's.
fn entity_like(store: &(impl StoreRead + ?Sized), report: &mut Report) {
    use smysl_core::schemas::tel;
    for (uid, unit) in store.units() {
        let broken = tel::entity_like_violations(&unit.core);
        if broken.is_empty() {
            continue;
        }
        report.push(
            Diagnostic::on(Code::E607, *uid)
                .with_message(format!(
                    "an `{}` carries what its identity forbids: {}",
                    unit.core.schema.as_str(),
                    broken.join(", ")
                ))
                .with_suggestion(
                    "a series or instrument has no span, manifest, published or observed time, \
                     and no grounds, and its source is exactly `series:<scheme>:<key>` (metric) \
                     or `instrument:<key>` (tool); what varies over time belongs in its `-meta` \
                     unit"
                        .to_string(),
                ),
        );
    }
}

/// SMYSL-3's window evidence (TL-P4): rule TW per window, and the windows of one key.
///
/// - `SMY-E603`: a window whose status or attestation is not the one rule TW gives its basis
///   and fields (§9.4), or a span or log template (TL-P9) not attested as an exact or a sketched
///   window is. The rule is `smysl_core::schemas::tel::rule_tw_violations`, which the
///   producer attests by, so the two cannot drift.
/// - `SMY-W614`: several windows of one window key with different digests (§9.2). Each is true
///   of its own sample set, so this is a report, not a contention.
///
/// Recomputing a window against the substrate (`SMY-E604`, `SMY-W605`) needs the substrate,
/// which a store does not hold; that is `smysl_tel::evidence::verify`'s, and `tel check`'s.
fn windows(store: &(impl StoreRead + ?Sized), report: &mut Report) {
    use smysl_core::schemas::tel;
    type Key = (String, String, u64, u64, String);
    let mut by_key: std::collections::BTreeMap<Key, BTreeSet<[u8; 32]>> =
        std::collections::BTreeMap::new();
    for (uid, unit) in store.units() {
        let ty = unit.core.schema.as_str();
        if ty == tel::SPAN || ty == tel::LOG_TEMPLATE {
            let atts: Vec<smysl_core::Attestation> = unit.attestations.iter().cloned().collect();
            let broken = tel::rule_tw_violations(&unit.core, &atts);
            if !broken.is_empty() {
                let what = if ty == tel::SPAN {
                    "a span"
                } else {
                    "a log template"
                };
                report.push(
                    Diagnostic::on(Code::E603, *uid)
                        .with_message(format!("{what} breaks rule TW: {}", broken.join("; ")))
                        .with_suggestion(
                            "a span is measured (imported, computed); a log template is derived \
                             (transformed, computed); both carry the digest of their events"
                                .to_string(),
                        ),
                );
            }
            continue;
        }
        if ty != tel::WINDOW {
            continue;
        }
        let atts: Vec<smysl_core::Attestation> = unit.attestations.iter().cloned().collect();
        let broken = tel::rule_tw_violations(&unit.core, &atts);
        if !broken.is_empty() {
            report.push(
                Diagnostic::on(Code::E603, *uid)
                    .with_message(format!("a window breaks rule TW: {}", broken.join("; ")))
                    .with_suggestion(
                        "exact evidence from the substrate is measured (imported, computed); \
                         sketched is derived (transformed, computed); a platform's aggregate is \
                         cited (imported, document) and has no digest"
                            .to_string(),
                    ),
            );
        }
        let Some(w) = unit
            .core
            .payload
            .as_deref()
            .and_then(|p| tel::window_fields(p).ok())
        else {
            continue;
        };
        if let (Some(s), Some(i), Some(f), Some(t), Some(r), Some(d)) = (
            w.series,
            w.instrument,
            w.from_ms,
            w.to_ms,
            w.recipe,
            w.digest,
        ) {
            by_key.entry((s, i, f, t, r)).or_default().insert(d);
        }
    }
    for ((series, _, from, to, recipe), digests) in by_key {
        if digests.len() > 1 {
            report.push(Diagnostic::new(Code::W614).with_message(format!(
                "{} windows of series {series} over [{from}, {to}) by {recipe} disagree: \
                 each describes a different sample set",
                digests.len()
            )));
        }
    }
}

/// Rule TQ (SMYSL-3 §11.4, TL-P6): `SMY-W606` for each numeric quote of a hypothesis that does
/// not verify against the unit it cites. The rule is `smysl_core::schemas::tel::rule_tq_violations`,
/// a function of the record set: it reads payloads, never the substrate, so it is monotone and
/// safe under merge. The cap it implies, at most `speculative`, is `tq_capped`'s.
fn quotes(store: &(impl StoreRead + ?Sized), report: &mut Report) {
    use smysl_core::schemas::tel;
    for (uid, unit) in store.units() {
        if unit.core.schema.as_str() != tel::HYPOTHESIS {
            continue;
        }
        let failed = tel::rule_tq_violations(&unit.core, |u| {
            store
                .get(u)
                .map(|c| c.core.payload.clone().unwrap_or_default())
        });
        for (quote, why) in failed {
            report.push(
                Diagnostic::on(Code::W606, *uid)
                    .with_message(format!(
                        "quote `{quote}` does not verify: {why}; the hypothesis is at most \
                         speculative"
                    ))
                    .with_suggestion(
                        "cite a unit among the hypothesis's grounds, by a field it has, with \
                         the value's canonical spelling"
                            .to_string(),
                    ),
            );
        }
    }
}

/// `SMY-W609` (SMYSL-3 §7, TL-P7): a series with no `x.tel/measured-by`. Witness counting takes
/// it as its own origin, so it counts as independent of every other series and can overcount
/// witnesses; the warning says which series are counted that way. The rule is `smysl_core::schemas::tel::unmeasured`.
fn unmeasured(store: &(impl StoreRead + ?Sized), report: &mut Report) {
    use smysl_core::schemas::tel;
    let units: Vec<(smysl_core::Uid, smysl_core::UnitCore)> = store
        .units()
        .filter(|(_, u)| u.core.schema.as_str() == tel::SERIES)
        .map(|(uid, u)| (*uid, u.core.clone()))
        .collect();
    if units.is_empty() {
        return;
    }
    for uid in tel::unmeasured(units.iter().map(|(u, c)| (u, c)), store.relations()) {
        report.push(
            Diagnostic::on(Code::W609, uid)
                .with_message(
                    "a series with no `x.tel/measured-by`: witness counting takes it as its own \
                     origin"
                        .to_string(),
                )
                .with_suggestion(
                    "relate the series to the instrument that measures it at its origin \
                     (`smysl tel declare measured-by SERIES INSTRUMENT`)"
                        .to_string(),
                ),
        );
    }
}

/// SMYSL-3's reservations (TL-P1): what no writer may emit, found in a store that holds it.
///
/// `SMY-E611` is a record 6 of a detection kind SMYSL-3 derives, 6 or 7. A store of this build
/// refuses one at the door, as it refuses rule E's 4 and 5, so this is what a store built some
/// other way holds: a document checked as written, or a log written by a peer that predates
/// TL-P1 and kept the record as `Unknown`. 4 and 5 have no code, and stay without one: the
/// door is their whole enforcement, and SMYSL-3 §15 allocates a code for its own two only.
///
/// `SMY-E612` is a reserved number written: record 16 (`hold`, D-T6) or `source` key 7 (a
/// sub-millisecond remainder, D-T5). Both are **carried**, not refused. Record 16 decodes as
/// `Unknown` and key 7 lands in the source's `extra`, both kept byte for byte because rule X
/// says so and, for key 7, because it is inside the uid. Reporting is the whole response.
fn reserved(store: &(impl StoreRead + ?Sized), report: &mut Report) {
    use smysl_core::cbor::keys::source::RESERVED_SUB_MS;
    use smysl_core::types::record::code::HOLD;
    use smysl_core::DetectionKind;

    for c in store.contentions() {
        if matches!(
            c.detected.kind,
            DetectionKind::RivalExplanation | DetectionKind::WitnessConflict
        ) {
            report.push(Diagnostic::new(Code::E611).with_message(format!(
                "contention {} is of detection kind {} ({}), which is derived from the record \
                 set and never written; a resolution names its derived id instead",
                c.id,
                c.detected.kind.as_u8(),
                c.detected.kind
            )));
        }
    }
    for r in store.hot_records(HotKind::Unknown) {
        if let smysl_core::Record::Unknown { code, payload } = r {
            if *code == HOLD {
                report.push(Diagnostic::new(Code::E612).with_message(format!(
                    "record 16 is reserved (hold) and MUST NOT be written; \
                     {} byte(s) preserved verbatim",
                    payload.len()
                )));
            }
        }
    }
    for r in store.records() {
        if let smysl_core::Record::Unit(u) = &*r {
            if u.source
                .as_ref()
                .is_some_and(|s| s.extra.contains_key(&RESERVED_SUB_MS))
            {
                report.push(
                    Diagnostic::on(Code::E612, smysl_core::canonical_uid(u)).with_message(
                        "source key 7 is reserved (a sub-millisecond remainder) and MUST NOT \
                         be written; preserved verbatim, because it is inside the uid",
                    ),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, Relation, SchemaDecl, Status, Uid, UnitCoreBuilder,
        View, ViewId,
    };
    use smysl_graph::Store;

    fn ext(id: &str) -> SchemaId {
        SchemaId::parse(id).unwrap()
    }

    fn unit_of(schema: SchemaId) -> Record {
        Record::Unit(
            UnitCoreBuilder::new(schema, "a unit", Status::Speculative)
                .build()
                .unwrap(),
        )
    }

    fn check(records: Vec<Record>, p: Option<&ConsumerProfile>) -> Report {
        let store = Store::from_records(records);
        let mut r = Report::new();
        run(&store, p, &mut r);
        r
    }

    #[test]
    fn a_kernel_only_store_is_full_fidelity_for_everyone() {
        let store = Store::from_records(vec![unit_of(SchemaId::from(KernelType::Claim))]);
        let f = fidelity(&store, &ConsumerProfile::default());
        assert_eq!(f.overall, Fidelity::Full);
        assert!(f.degraded.is_empty());
    }

    /// Rule X: an unimplemented extension degrades, it does not refuse.
    #[test]
    fn an_unimplemented_extension_degrades() {
        let store = Store::from_records(vec![unit_of(ext("x.sre/incident"))]);
        let f = fidelity(&store, &ConsumerProfile::default());
        assert_eq!(f.overall, Fidelity::Degraded);
        assert_eq!(f.degraded.len(), 1);
        assert!(f.missing.contains(&ext("x.sre/incident")));
    }

    #[test]
    fn an_implemented_extension_is_full_fidelity() {
        let store = Store::from_records(vec![unit_of(ext("x.sre/incident"))]);
        let p = ConsumerProfile::new("sre").implementing([ext("x.sre/incident")]);
        assert_eq!(fidelity(&store, &p).overall, Fidelity::Full);
    }

    /// The one case where silent degradation is forbidden.
    #[test]
    fn a_missing_kernel_major_refuses() {
        let view = View::new(ViewId::new("v/x").unwrap(), "i")
            .requiring([SchemaId::parse("smysl.kernel/9").unwrap()]);
        let store = Store::from_records(vec![Record::View(view)]);
        assert_eq!(
            fidelity(&store, &ConsumerProfile::default()).overall,
            Fidelity::Refuse
        );
    }

    #[test]
    fn the_implemented_kernel_major_is_accepted() {
        let view = View::new(ViewId::new("v/x").unwrap(), "i")
            .requiring([SchemaId::parse("smysl.kernel/0.1").unwrap()]);
        let store = Store::from_records(vec![Record::View(view)]);
        assert_eq!(
            fidelity(&store, &ConsumerProfile::default()).overall,
            Fidelity::Full
        );
    }

    #[test]
    fn a_required_but_unimplemented_schema_degrades_even_with_no_units() {
        let view = View::new(ViewId::new("v/x").unwrap(), "i").requiring([ext("x.sre/1")]);
        let store = Store::from_records(vec![Record::View(view)]);
        let f = fidelity(&store, &ConsumerProfile::default());
        assert_eq!(f.overall, Fidelity::Degraded);
        assert!(f.missing.contains(&ext("x.sre/1")));
    }

    /// A view's `requires` naming an extension the consumer *does* implement.
    ///
    /// Its neighbour above covers the unimplemented direction, and three other tests cover an
    /// implemented extension arriving as a *unit's* schema. Nothing covered an implemented one
    /// arriving through `requires`, so mutation testing in 0.11 could force that guard to
    /// `true` — every required schema counted missing however well the consumer knew it — and
    /// no test noticed.
    ///
    /// The consequence is the whole `full`/`degraded` distinction of §23.1. A consumer that
    /// implements exactly what a view asks for would have been told it degrades, which is the
    /// answer that makes the negotiation pointless: if implementing the requirement does not
    /// earn `full`, nothing does.
    #[test]
    fn a_required_and_implemented_schema_is_full_fidelity() {
        let view = View::new(ViewId::new("v/x").unwrap(), "i").requiring([ext("x.sre/1")]);
        let store = Store::from_records(vec![Record::View(view)]);
        let p = ConsumerProfile::new("sre").implementing([ext("x.sre/1")]);
        let f = fidelity(&store, &p);
        assert_eq!(
            f.overall,
            Fidelity::Full,
            "the consumer implements exactly what the view requires"
        );
        assert!(
            f.missing.is_empty(),
            "and nothing is missing: {:?}",
            f.missing
        );
    }

    #[test]
    fn w010_names_the_unit_and_the_schema() {
        let r = check(
            vec![unit_of(ext("x.sre/incident"))],
            Some(&ConsumerProfile::default()),
        );
        assert_eq!(r.count(Code::W010), 1);
        let d = r.iter().next().unwrap();
        assert!(d.message.contains("x.sre/incident"));
        assert!(d.uid().is_some());
        assert!(r.is_clean(), "degraded fidelity is not a failure");
    }

    #[test]
    fn w010_is_silent_without_a_consumer_profile() {
        assert!(check(vec![unit_of(ext("x.sre/incident"))], None).is_empty());
    }

    #[test]
    fn an_extension_redefining_a_kernel_type_is_e012() {
        let mut d = SchemaDecl::new(ext("x.sre/1"), 1);
        d.types = vec![SchemaId::from(KernelType::Claim)];
        let r = check(vec![Record::SchemaDecl(d)], None);
        assert_eq!(r.count(Code::E012), 1);
    }

    #[test]
    fn an_extension_redefining_a_kernel_relation_is_e012() {
        let mut d = SchemaDecl::new(ext("x.sre/1"), 1);
        d.relations = vec![RelKind::Rebuts];
        assert_eq!(
            check(vec![Record::SchemaDecl(d)], None).count(Code::E012),
            1
        );
    }

    #[test]
    fn an_extension_that_only_adds_is_fine() {
        let mut d = SchemaDecl::new(ext("x.sre/1"), 1);
        d.types = vec![ext("x.sre/incident")];
        d.relations = vec![RelKind::parse("x.sre/mitigates").unwrap()];
        assert!(check(vec![Record::SchemaDecl(d)], None).is_empty());
    }

    /// An unknown relation kind stays routable rather than being dropped or refused.
    #[test]
    fn an_undeclared_relation_kind_is_w013() {
        let a = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Speculative)
            .build()
            .unwrap();
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(
            vec![
                Record::Unit(a),
                Record::Unit(b),
                Record::Relation(Relation::new(
                    RelKind::parse("x.sre/mitigates").unwrap(),
                    ua,
                    ub,
                )),
            ],
            None,
        );
        assert_eq!(r.count(Code::W013), 1);
        assert!(r.is_clean());
    }

    /// The declaration `x.text/v1` ships as a record (A-11, TX-P5 step 2), against the pass that
    /// makes shipping it necessary.
    ///
    /// Ten relation kinds were in use across three crates before the declaration existed, and
    /// every store that held one of them warned `SMY-W013` on every check — the warning was
    /// correct and unactionable, because there was nothing to declare them with. This is the
    /// half of "registered" that a registry alone does not buy.
    #[test]
    fn the_standard_text_schema_declares_every_kind_it_names() {
        use smysl_core::schemas::text;

        let a = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Speculative)
            .build()
            .unwrap();
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));

        let edges: Vec<Record> = text::RELATIONS
            .iter()
            .map(|k| {
                Record::Relation(Relation::new(
                    RelKind::parse(k).expect("a well-formed kind"),
                    ua,
                    ub,
                ))
            })
            .collect();

        let mut without = vec![Record::Unit(a.clone()), Record::Unit(b.clone())];
        without.extend(edges.clone());
        assert_eq!(
            check(without, None).count(Code::W013),
            text::RELATIONS.len(),
            "every kind should warn while nothing declares it"
        );

        let mut with = vec![
            Record::SchemaDecl(text::decl()),
            Record::Unit(a),
            Record::Unit(b),
        ];
        with.extend(edges);
        let r = check(with, None);
        assert_eq!(
            r.count(Code::W013),
            0,
            "the declaration did not cover a kind"
        );
        assert_eq!(
            r.count(Code::E012),
            0,
            "registered is not kernel, and must not claim to be"
        );
    }

    #[test]
    fn a_relation_declared_in_surface_text_does_not_warn() {
        // End to end from the text a person writes. The two tests either side of this one build
        // the `SchemaDecl` by hand, which is the only way one existed until 1.3: surface syntax
        // had no spelling for it, so a `.smy` file using `--x.code/touches-->` warned SMY-W013 on
        // every check, forever, whatever the author did.
        let src = "@schema x.code/v1 { version: 1, relations: [x.code/touches] }\n\n\
                   @claim c/a { status: speculative }\n~ A.\n\n\
                   @claim c/b { status: speculative }\n~ B.\n\n\
                   @rel c/a --x.code/touches--> c/b\n";
        let out = smysl_core::surface::parse_surface(src).unwrap();
        assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
        let r = check(out.records.clone(), None);
        assert_eq!(r.count(Code::W013), 0, "a declared kind still warned");

        // And the same file without the declaration is the control.
        let bare = src.split_once("\n\n").unwrap().1;
        let out = smysl_core::surface::parse_surface(bare).unwrap();
        assert_eq!(check(out.records, None).count(Code::W013), 1);
    }

    #[test]
    fn a_declared_extension_relation_does_not_warn() {
        let a = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Speculative)
            .build()
            .unwrap();
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let kind = RelKind::parse("x.sre/mitigates").unwrap();
        let mut d = SchemaDecl::new(ext("x.sre/1"), 1);
        d.relations = vec![kind.clone()];
        let r = check(
            vec![
                Record::Unit(a),
                Record::Unit(b),
                Record::SchemaDecl(d),
                Record::Relation(Relation::new(kind, ua, ub)),
            ],
            None,
        );
        assert_eq!(r.count(Code::W013), 0);
    }

    #[test]
    fn kernel_relation_kinds_never_warn() {
        let a = UnitCoreBuilder::new(KernelType::Claim, "a", Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Speculative)
            .build()
            .unwrap();
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let r = check(
            vec![
                Record::Unit(a),
                Record::Unit(b),
                Record::Relation(Relation::new(RelKind::Rebuts, ua, ub)),
            ],
            None,
        );
        assert!(r.is_empty());
    }

    #[test]
    fn a_consumer_always_understands_the_kernel() {
        let p = ConsumerProfile::default();
        for k in KernelType::ALL {
            assert!(p.understands(&SchemaId::from(*k)));
        }
        assert!(!p.understands(&ext("x.sre/1")));
        let _ = Uid::from_bytes([0; 32]);
    }
}
