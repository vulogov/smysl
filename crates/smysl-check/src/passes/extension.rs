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
use smysl_graph::Store;

/// What a consumer implements, for `check --as` (§23.1).
#[derive(Debug, Clone, PartialEq, Eq)]
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
pub struct FidelityReport {
    pub overall: Fidelity,
    /// Units this consumer would read at reduced fidelity, with the schema responsible.
    pub degraded: Vec<(smysl_core::Uid, SchemaId)>,
    /// Schemas the store requires that the consumer does not implement.
    pub missing: BTreeSet<SchemaId>,
}

pub fn run(store: &Store, profile: Option<&ConsumerProfile>, report: &mut Report) {
    // `SMY-E012` - an extension that redefines a kernel type or relation kind.
    for r in store.iter() {
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
    for r in store.iter() {
        if let smysl_core::Record::Unknown { code, payload } = r {
            report.push(Diagnostic::new(Code::W014).with_message(format!(
                "record type {code} is not known to this build; \
                 {} byte(s) preserved verbatim, skipped semantically",
                payload.len()
            )));
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
fn declared(store: &Store, kind: &RelKind) -> bool {
    store.iter().any(|r| match r {
        smysl_core::Record::SchemaDecl(d) => d.relations.contains(kind),
        _ => false,
    })
}

/// What `check --as` reports (§23.1).
pub fn fidelity(store: &Store, profile: &ConsumerProfile) -> FidelityReport {
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

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, KernelType, Record, Relation, SchemaDecl, Status, Uid, UnitCoreBuilder,
        View, ViewId,
    };

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
