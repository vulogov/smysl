//! Pass 7 - rule T, the trust ceiling (§9.3, §17).
//!
//! Rule M prevents laundering *inside* the graph; rule T prevents it *at entry*. A model
//! asserting from its own parametric knowledge is capped at `inferred` however confidently
//! it phrases the claim, and `ingest` may never assign `measured` at all - only an
//! instrument or tool adapter recording `op: Imported` with a machine-checkable source may.
//!
//! | Rung | Origin | Ceiling |
//! |---|---|---|
//! | `computed` | deterministic tool, calculation, parser | `derived` |
//! | `document` | user-supplied document or dataset | `cited` |
//! | `web` | fetched content, gated | `cited` |
//! | `model` | the model's own parametric knowledge | `inferred` |
//!
//! Where a unit carries several attestations the **best** provenance binds, not the worst:
//! one agent importing a measurement justifies `measured` even if another merely echoed it
//! from priors. Corroboration adds support; it never subtracts it.

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{Op, Rung, Status};
use smysl_graph::Store;

pub fn run(store: &Store, report: &mut Report) {
    for (uid, unit) in store.units() {
        let status = unit.core.status;
        if unit.attestations.is_empty() {
            // Nothing was asserted about where this came from, so there is no ceiling to
            // check. `check --conformance C-Produce` is what demands attestations exist.
            continue;
        }

        // The best provenance available binds.
        let (ceiling, best_rung) = unit
            .attestations
            .iter()
            .map(|a| (a.ceiling(), a.rung))
            .max()
            .expect("non-empty");

        if status > ceiling {
            report.push(
                Diagnostic::on(Code::E033, *uid)
                    .with_message(format!(
                        "{status} exceeds the {ceiling} ceiling of its best provenance ({best_rung} rung)"
                    ))
                    .with_suggestion(format!(
                        "weaken to {ceiling}, or import it with op: imported and a checkable source"
                    )),
            );
        }

        // `SMY-W035` - only an import may claim `measured` (rule T).
        if status == Status::Measured
            && !unit.attestations.iter().any(|a| a.op.may_assign_measured())
        {
            report.push(
                Diagnostic::on(Code::W035, *uid)
                    .with_message("measured, but no attestation records op: imported")
                    .with_suggestion("record the import that produced this measurement"),
            );
        }
    }
}

/// Whether a rung could ever justify a status. Exposed because `ingest` applies the same
/// ceiling before staging, and the two must not drift apart.
pub fn permits(rung: Rung, status: Status) -> bool {
    status <= rung.ceiling()
}

/// Whether an op may claim `measured` (rule T).
pub fn may_assign(op: Op, status: Status) -> bool {
    status != Status::Measured || op.may_assign_measured()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Attestation, Hlc, KernelType, Record, SourceKind, SourceRef,
        UnitCore, UnitCoreBuilder,
    };

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    fn unit(status: Status) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a claim", status);
        if status.requires_source() {
            b = b.source(SourceRef::new(SourceKind::Metric, "m"));
        }
        if status.requires_grounds() {
            b = b.grounds([smysl_core::Uid::from_bytes([1; 32])]);
        }
        b.build().unwrap()
    }

    fn attested(core: UnitCore, op: Op, rung: Rung) -> Vec<Record> {
        let uid = canonical_uid(&core);
        let a = agent("model:anthropic/claude-opus-5");
        vec![
            Record::Unit(core),
            Record::Attestation(Attestation::new(uid, a.clone(), op, rung, Hlc::zero(a))),
        ]
    }

    fn check(records: Vec<Record>) -> Report {
        let store = Store::from_records(records);
        let mut r = Report::new();
        run(&store, &mut r);
        r
    }

    /// The table of §9.3, asserted through the pass rather than through the type.
    #[test]
    fn each_rung_permits_exactly_its_ceiling_and_below() {
        let cases = [
            (Rung::Computed, Status::Derived),
            (Rung::Document, Status::Cited),
            (Rung::Web, Status::Cited),
            (Rung::Model, Status::Inferred),
        ];
        for (rung, ceiling) in cases {
            for &s in Status::ALL {
                if s == Status::Unfounded {
                    continue;
                }
                assert_eq!(permits(rung, s), s <= ceiling, "{rung} / {s}");
            }
        }
    }

    #[test]
    fn a_model_asserting_inferred_is_within_its_ceiling() {
        let r = check(attested(unit(Status::Inferred), Op::Authored, Rung::Model));
        assert_eq!(r.count(Code::E033), 0);
    }

    /// The central anti-laundering case: however confidently a model phrases it, its own
    /// priors cap at `inferred`.
    #[test]
    fn a_model_claiming_derived_is_e033() {
        let r = check(attested(unit(Status::Derived), Op::Authored, Rung::Model));
        assert_eq!(r.count(Code::E033), 1);
        let d = r.iter().find(|d| d.code == Code::E033).unwrap();
        assert!(d.message.contains("inferred"), "{}", d.message);
        assert!(d.message.contains("model"));
    }

    #[test]
    fn no_rung_reaches_measured_by_authoring() {
        for rung in Rung::ALL {
            let r = check(attested(unit(Status::Measured), Op::Authored, *rung));
            assert_eq!(r.count(Code::E033), 1, "{rung} must not reach measured");
        }
    }

    /// Only an instrument may assign `measured`, and the op is where that is recorded.
    #[test]
    fn measured_requires_an_import() {
        let authored = check(attested(
            unit(Status::Measured),
            Op::Authored,
            Rung::Computed,
        ));
        assert_eq!(authored.count(Code::W035), 1);

        let imported = check(attested(
            unit(Status::Measured),
            Op::Imported,
            Rung::Computed,
        ));
        assert_eq!(imported.count(Code::W035), 0);
    }

    /// An import still cannot exceed its rung: `op` says how it arrived, `rung` says how
    /// far it may be trusted, and both bind.
    #[test]
    fn an_import_at_the_model_rung_still_caps_at_inferred() {
        let r = check(attested(unit(Status::Derived), Op::Imported, Rung::Model));
        assert_eq!(r.count(Code::E033), 1);
    }

    #[test]
    fn a_computed_import_may_claim_derived() {
        let r = check(attested(
            unit(Status::Derived),
            Op::Imported,
            Rung::Computed,
        ));
        assert!(r.is_empty());
    }

    #[test]
    fn a_document_import_may_claim_cited() {
        let r = check(attested(unit(Status::Cited), Op::Imported, Rung::Document));
        assert!(r.is_empty());
    }

    /// One agent importing a measurement justifies it even if another merely echoed it
    /// from priors. Corroboration adds support; it never subtracts it.
    #[test]
    fn the_best_provenance_binds_not_the_worst() {
        let core = unit(Status::Cited);
        let uid = canonical_uid(&core);
        let weak = agent("model:openai/gpt");
        let strong = agent("tool:importer");
        let records = vec![
            Record::Unit(core),
            Record::Attestation(Attestation::new(
                uid,
                weak.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::zero(weak),
            )),
            Record::Attestation(Attestation::new(
                uid,
                strong.clone(),
                Op::Imported,
                Rung::Document,
                Hlc::zero(strong),
            )),
        ];
        assert_eq!(check(records).count(Code::E033), 0);
    }

    #[test]
    fn a_unit_with_no_attestation_has_no_ceiling_to_check() {
        let store = Store::from_records(vec![Record::Unit(unit(Status::Measured))]);
        let mut r = Report::new();
        run(&store, &mut r);
        assert!(
            r.is_empty(),
            "provenance was never asserted, so nothing is violated"
        );
    }

    #[test]
    fn may_assign_matches_the_op_table() {
        for op in Op::ALL {
            assert_eq!(may_assign(*op, Status::Measured), *op == Op::Imported);
            assert!(may_assign(*op, Status::Cited), "{op} may cite");
        }
    }

    /// The two halves of the guarantee are independent: rule T caps what enters, rule M
    /// caps what the graph lets you build on it.
    #[test]
    fn rule_t_and_rule_m_bind_different_things() {
        // Within rule T's ceiling, but a unit with no grounds cannot be `inferred` at all
        // - that is the constructor's business, and neither rule's.
        assert!(permits(Rung::Model, Status::Inferred));
        assert!(!permits(Rung::Model, Status::Derived));
        assert!(permits(Rung::Computed, Status::Derived));
        assert!(!permits(Rung::Computed, Status::Cited));
    }
}
