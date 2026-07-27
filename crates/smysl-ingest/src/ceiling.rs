//! Rule T - the trust ceiling at the ingest boundary (§9.3, §22.4).
//!
//! | Rung | Origin | Max assignable status |
//! |---|---|---|
//! | `computed` | deterministic tool, calculation, parser | `derived` |
//! | `document` | user-supplied document or dataset | `cited` (+ `source`) |
//! | `web` | fetched content, gated | `cited` (+ `source` + timestamp) |
//! | `model` | the model's own parametric knowledge | `inferred` |
//!
//! **`ingest` MUST NOT assign `measured`.** Only an instrument or tool adapter recording
//! `op: Imported` with a machine-checkable `source` may. Rule M stops laundering inside the
//! graph; rule T stops it at entry - a model asserting from its own priors is capped at
//! `inferred` however confidently it phrases the claim.
//!
//! The downgrade is **unconditional and reported**: applied after parse, with `SMY-E033`
//! emitted against the staged unit. Silently accepting the model's word would make the
//! ceiling advice; silently downgrading without a diagnostic would hide that a model tried.

use smysl_core::{Code, Diagnostic, Rung, Status, UnitCore, UnitCoreBuilder};

/// The highest status this rung may assign (§9.3).
pub const fn ceiling(rung: Rung) -> Status {
    rung.ceiling()
}

/// What the ceiling did to one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Applied {
    pub core: UnitCore,
    /// The status the model asked for, when it was not the one it got.
    pub claimed: Option<Status>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Applied {
    pub fn was_downgraded(&self) -> bool {
        self.claimed.is_some()
    }
}

/// Apply rule T to one parsed unit.
///
/// Rebuilding rather than mutating, because `UnitCore` has no setter by design: a core is
/// shape-valid from the moment it exists.
///
/// The cap is not always the ceiling. `measured` requires a `source`; `inferred` requires
/// `grounds`. A model claiming `measured` with a source and no grounds cannot become
/// `inferred` - there is nothing for it to be inferred *from* - so the status walks down to
/// the strongest one the unit's own shape supports, which is `speculative` in the worst
/// case. §9.3 anticipates exactly this: the model rung reads "`inferred` — typically
/// `speculative`".
pub fn apply(core: &UnitCore, rung: Rung, label: Option<&str>) -> Applied {
    let ceiling = ceiling(rung);
    if core.status <= ceiling {
        return Applied {
            core: core.clone(),
            claimed: None,
            diagnostics: Vec::new(),
        };
    }
    let cap = attainable(core, ceiling);

    let mut b = UnitCoreBuilder::new(core.schema.clone(), &core.gist, cap)
        .deps(core.deps.iter().copied())
        .grounds(core.grounds.iter().copied());
    if let Some(t) = &core.body {
        b = b.body(t);
    }
    if let Some(t) = &core.detail {
        b = b.detail(t);
    }
    if let Some(s) = &core.source {
        b = b.source(s.clone());
    }
    if let Some(p) = &core.payload {
        b = b.payload(p.clone());
    }

    // `attainable` already chose a status this shape supports, so a failure here means the
    // core was unbuildable for some other reason and the original is what the caller should
    // see and diagnose.
    let Ok(lowered) = b.build() else {
        return Applied {
            core: core.clone(),
            claimed: None,
            diagnostics: vec![Diagnostic::new(Code::E033).with_message(format!(
                "status {} exceeds the {rung} ceiling of {cap}, and the unit could not be \
                 rebuilt at {cap}",
                core.status
            ))],
        };
    };

    // The subject is the *capped* unit's uid, not the one the model tried to mint: that
    // unit does not exist and never will, so pointing a diagnostic at it would point at
    // nothing a reader could look up.
    let why = if cap < ceiling {
        format!("; capped at {cap} rather than {ceiling}, which its shape cannot support")
    } else {
        format!("; capped at {cap}")
    };
    let d = Diagnostic::on(Code::E033, smysl_core::canonical_uid(&lowered)).with_message(format!(
        "{} claimed {} from a {rung} source{why}",
        label.unwrap_or("unit"),
        core.status
    ));

    Applied {
        core: lowered,
        claimed: Some(core.status),
        diagnostics: vec![d],
    }
}

/// The strongest status at or below `cap` that this unit's shape can carry.
///
/// `speculative` is the floor and always attainable: it requires neither a source nor
/// grounds, which is what makes rule I's promise of progress keepable here too.
pub fn attainable(core: &UnitCore, cap: Status) -> Status {
    for &s in Status::ALL.iter().rev() {
        // `unfounded` is unauthorable (`SMY-E034`) - reachable only by retraction - so it
        // is never a status ingest may hand back, whatever cap it was asked for.
        if s > cap || s == Status::Unfounded {
            continue;
        }
        let ok = match s {
            Status::Measured | Status::Cited => core.source.is_some(),
            Status::Derived | Status::Inferred => !core.grounds.is_empty(),
            _ => true,
        };
        if ok {
            return s;
        }
    }
    Status::Speculative
}

/// Whether a rung's ceiling requires a `source` to be reachable at all.
pub const fn requires_source(rung: Rung) -> bool {
    rung.ceiling_requires_source()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, SourceKind, SourceRef, Uid};

    fn core(status: Status, with_source: bool) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a claim about something", status);
        if with_source {
            b = b.source(SourceRef::new(SourceKind::Doc, "handbook#3"));
        }
        if matches!(status, Status::Derived | Status::Inferred) {
            b = b.grounds([Uid::from_bytes([1; 32])]);
        }
        if matches!(status, Status::Measured | Status::Cited) && !with_source {
            b = b.source(SourceRef::new(SourceKind::Doc, "required by shape"));
        }
        b.build().expect("a shape-valid core")
    }

    #[test]
    fn the_ceiling_table_is_the_rfcs() {
        assert_eq!(ceiling(Rung::Computed), Status::Derived);
        assert_eq!(ceiling(Rung::Document), Status::Cited);
        assert_eq!(ceiling(Rung::Web), Status::Cited);
        assert_eq!(ceiling(Rung::Model), Status::Inferred);
    }

    /// The rule the whole boundary exists for: no rung reaches `measured`.
    #[test]
    fn ingest_can_never_assign_measured() {
        for &r in Rung::ALL {
            assert!(ceiling(r) < Status::Measured, "{r} reaches measured");
        }
    }

    #[test]
    fn a_status_within_the_ceiling_is_untouched() {
        let c = core(Status::Speculative, false);
        let out = apply(&c, Rung::Model, Some("h/guess"));
        assert_eq!(out.core, c);
        assert!(!out.was_downgraded());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn a_status_at_the_ceiling_is_untouched() {
        let c = core(Status::Inferred, false);
        assert!(!apply(&c, Rung::Model, None).was_downgraded());
    }

    /// A model asserting from its own priors is capped however confidently it phrases the
    /// claim - and the downgrade is reported, not silent.
    #[test]
    fn a_model_claiming_measured_is_capped_and_reported() {
        let c = UnitCoreBuilder::new(KernelType::Claim, "grounded claim", Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .grounds([Uid::from_bytes([1; 32])])
            .build()
            .unwrap();
        let out = apply(&c, Rung::Model, Some("c/laundered"));
        assert_eq!(out.core.status, Status::Inferred);
        assert_eq!(out.claimed, Some(Status::Measured));
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].code, Code::E033);
        assert!(out.diagnostics[0].message.contains("c/laundered"));
        assert!(out.diagnostics[0].message.contains("measured"));
    }

    #[test]
    fn every_rung_caps_a_measured_claim() {
        for &r in Rung::ALL {
            let out = apply(&core(Status::Measured, true), r, None);
            assert!(out.was_downgraded(), "{r} let measured through");
            assert!(out.core.status <= ceiling(r), "{r} exceeded its ceiling");
            assert_ne!(out.core.status, Status::Measured);
        }
    }

    #[test]
    fn a_document_rung_caps_at_cited() {
        let out = apply(&core(Status::Measured, true), Rung::Document, None);
        assert_eq!(out.core.status, Status::Cited);
    }

    #[test]
    fn a_computed_rung_caps_a_grounded_claim_at_derived() {
        let mut c = UnitCoreBuilder::new(KernelType::Claim, "grounded", Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .grounds([Uid::from_bytes([1; 32])]);
        c = c.body("b");
        let out = apply(&c.build().unwrap(), Rung::Computed, None);
        assert_eq!(out.core.status, Status::Derived);
        assert!(!out.core.grounds.is_empty(), "derived keeps its grounds");
    }

    /// `inferred` requires grounds. A model claiming `measured` with a source and nothing
    /// to infer *from* cannot become `inferred`, so it walks down to `speculative` - which
    /// is what §9.3 means by "typically speculative".
    #[test]
    fn an_ungrounded_claim_walks_past_a_status_its_shape_cannot_support() {
        let out = apply(&core(Status::Measured, true), Rung::Model, Some("c/x"));
        assert_eq!(out.core.status, Status::Speculative);
        assert!(out.was_downgraded());
        assert!(
            out.diagnostics[0].message.contains("shape cannot support"),
            "{}",
            out.diagnostics[0].message
        );
    }

    #[test]
    fn attainable_never_exceeds_the_cap_and_never_fails() {
        // Bare: neither a source nor grounds, so only `speculative` is reachable.
        let bare = core(Status::Speculative, false);
        for &cap in Status::ALL {
            assert_eq!(attainable(&bare, cap), Status::Speculative, "cap {cap}");
        }

        // Sourced: `cited` and `measured` become reachable, `inferred` does not.
        let sourced = core(Status::Cited, true);
        assert_eq!(attainable(&sourced, Status::Measured), Status::Measured);
        assert_eq!(attainable(&sourced, Status::Cited), Status::Cited);
        assert_eq!(attainable(&sourced, Status::Inferred), Status::Speculative);

        // Grounded: `derived` and `inferred` become reachable.
        let grounded = core(Status::Inferred, false);
        assert_eq!(attainable(&grounded, Status::Derived), Status::Derived);
        assert_eq!(attainable(&grounded, Status::Cited), Status::Derived);
    }

    /// Everything else about the unit survives the downgrade: only the status moves.
    #[test]
    fn a_downgrade_preserves_the_rest_of_the_unit() {
        let c = UnitCoreBuilder::new(KernelType::Evidence, "the gist", Status::Measured)
            .body("the body")
            .detail("the detail")
            .source(SourceRef::new(SourceKind::Metric, "m"))
            .grounds([Uid::from_bytes([1; 32])])
            .deps([Uid::from_bytes([2; 32])])
            .build()
            .unwrap();
        let out = apply(&c, Rung::Model, None);
        assert_eq!(out.core.gist, c.gist);
        assert_eq!(out.core.body, c.body);
        assert_eq!(out.core.detail, c.detail);
        assert_eq!(out.core.source, c.source);
        assert_eq!(out.core.deps, c.deps);
        assert_eq!(out.core.schema, c.schema);
        assert_ne!(out.core.status, c.status);
    }

    /// Lowering a status changes the uid, which is the point: a capped unit is a different
    /// claim from the one the model tried to make, and content addressing says so.
    #[test]
    fn a_downgrade_changes_the_uid() {
        let c = core(Status::Measured, true);
        let out = apply(&c, Rung::Model, None);
        assert_ne!(
            smysl_core::canonical_uid(&out.core),
            smysl_core::canonical_uid(&c)
        );
    }

    /// The diagnostic points at the *capped* unit, not the one the model tried to mint:
    /// that unit does not exist and never will, so a reader could not look it up.
    #[test]
    fn the_diagnostic_points_at_the_unit_that_exists() {
        let claimed = core(Status::Measured, true);
        let out = apply(&claimed, Rung::Model, Some("c/x"));
        assert_eq!(
            out.diagnostics[0].subject,
            smysl_core::Subject::Unit(smysl_core::canonical_uid(&out.core))
        );
        assert!(out.diagnostics[0].message.contains("c/x"));
    }

    #[test]
    fn source_requirements_track_the_rung() {
        assert!(requires_source(Rung::Document));
        assert!(requires_source(Rung::Web));
        assert!(!requires_source(Rung::Computed));
    }
}
