//! Units: the immutable hashed core plus the accreting envelope around it (§1.1).

use std::collections::{BTreeMap, BTreeSet};

use crate::error::ShapeError;
use crate::ids::{Label, SchemaId, Uid};
use crate::types::epistemics::{SourceRef, Status};
use crate::types::normalise;
use crate::types::provenance::Attestation;

/// Unknown map keys read from a record written by a later minor version.
///
/// Preserved verbatim and re-emitted in key order. Without this, decoding and re-encoding
/// a forward-compatible record would change its uid, and `check --verify-hashes` would
/// report every such unit as corrupt (rule X, applied at the record level).
pub type Extra = BTreeMap<u16, Vec<u8>>;

/// The hashed part of a unit. Immutable by construction.
///
/// `uid = BLAKE3-256(det_cbor(UnitCore))`. Attestations, salience, and labels are
/// deliberately outside it: identity is content, so the same claim from two agents is one
/// unit with two attestations rather than two units.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitCore {
    pub schema: SchemaId,
    /// L0. Required, NFC, interpretable from the L0 of `deps` alone.
    pub gist: String,
    /// L1. Interpretable from the L0 of `deps` and `grounds`.
    pub body: Option<String>,
    /// L2. Unbounded, no closure requirement.
    pub detail: Option<String>,
    pub deps: BTreeSet<Uid>,
    pub grounds: BTreeSet<Uid>,
    pub status: Status,
    /// Required iff `status` is `measured` or `cited`.
    pub source: Option<SourceRef>,
    /// Opaque deterministic CBOR belonging to an extension schema (rule X).
    pub payload: Option<Vec<u8>>,
    /// Unknown keys from a future minor version, preserved verbatim.
    pub extra: Extra,
}

/// Builder for [`UnitCore`]. The only way to construct one.
///
/// The invariant a constructed `UnitCore` carries is *already NFC-normalised, already
/// shape-valid*, which is what makes encoding and hashing infallible and guarantees one
/// logical core has exactly one byte encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitCoreBuilder {
    pub schema: SchemaId,
    pub gist: String,
    pub body: Option<String>,
    pub detail: Option<String>,
    pub deps: BTreeSet<Uid>,
    pub grounds: BTreeSet<Uid>,
    pub status: Status,
    pub source: Option<SourceRef>,
    pub payload: Option<Vec<u8>>,
    pub extra: Extra,
}

impl UnitCoreBuilder {
    pub fn new(schema: impl Into<SchemaId>, gist: impl Into<String>, status: Status) -> Self {
        UnitCoreBuilder {
            schema: schema.into(),
            gist: gist.into(),
            body: None,
            detail: None,
            deps: BTreeSet::new(),
            grounds: BTreeSet::new(),
            status,
            source: None,
            payload: None,
            extra: Extra::new(),
        }
    }

    pub fn body(mut self, s: impl Into<String>) -> Self {
        self.body = Some(s.into());
        self
    }

    pub fn detail(mut self, s: impl Into<String>) -> Self {
        self.detail = Some(s.into());
        self
    }

    pub fn deps(mut self, d: impl IntoIterator<Item = Uid>) -> Self {
        self.deps = d.into_iter().collect();
        self
    }

    pub fn grounds(mut self, g: impl IntoIterator<Item = Uid>) -> Self {
        self.grounds = g.into_iter().collect();
        self
    }

    pub fn source(mut self, s: SourceRef) -> Self {
        self.source = Some(s);
        self
    }

    pub fn payload(mut self, p: Vec<u8>) -> Self {
        self.payload = Some(p);
        self
    }

    pub fn build(self) -> Result<UnitCore, ShapeError> {
        UnitCore::new(self)
    }
}

impl UnitCore {
    /// Validate and normalise. Rule M is deliberately absent: shape is local, and
    /// monotonicity needs the store.
    pub fn new(b: UnitCoreBuilder) -> Result<UnitCore, ShapeError> {
        let gist = normalise(&b.gist);
        if gist.trim().is_empty() {
            return Err(ShapeError::MissingGist);
        }
        let body = b.body.as_deref().map(normalise).filter(|s| !s.is_empty());
        let detail = b.detail.as_deref().map(normalise).filter(|s| !s.is_empty());

        if detail.is_some() && body.is_none() {
            return Err(ShapeError::DetailWithoutBody);
        }
        if b.status == Status::Unfounded {
            return Err(ShapeError::UnfoundedAuthored);
        }
        if b.status.requires_source() && b.source.is_none() {
            return Err(ShapeError::SourceRequired);
        }
        if b.status.requires_grounds() && b.grounds.is_empty() {
            return Err(ShapeError::GroundsRequired);
        }

        Ok(UnitCore {
            schema: b.schema,
            gist,
            body,
            detail,
            deps: b.deps,
            grounds: b.grounds,
            status: b.status,
            source: b.source,
            payload: b.payload,
            extra: b.extra,
        })
    }

    /// The text at a level, or `None` if the unit was not authored that deep.
    pub fn text_at(&self, lod: crate::types::epistemics::Lod) -> Option<&str> {
        use crate::types::epistemics::Lod;
        match lod {
            Lod::L0 => Some(&self.gist),
            Lod::L1 => self.body.as_deref(),
            Lod::L2 => self.detail.as_deref(),
        }
    }

    /// The deepest level this unit was authored at.
    pub fn max_lod(&self) -> crate::types::epistemics::Lod {
        use crate::types::epistemics::Lod;
        if self.detail.is_some() {
            Lod::L2
        } else if self.body.is_some() {
            Lod::L1
        } else {
            Lod::L0
        }
    }

    /// Everything this unit points at: interpretive prerequisites and evidential support.
    pub fn references(&self) -> impl Iterator<Item = &Uid> {
        self.deps.iter().chain(self.grounds.iter())
    }

    /// A gist-only unit - the normal shape of imported summary material (§1.3).
    pub fn is_gist_only(&self) -> bool {
        self.body.is_none() && self.detail.is_none()
    }
}

/// A unit: the immutable core, plus everything that accretes around it.
///
/// Only `core` is hashed. `attestations` grow monotonically, `salience` is an authored
/// override, and `labels` are view-scoped aliases - none of them are identity.
#[derive(Debug, Clone, PartialEq)]
pub struct Unit {
    pub core: UnitCore,
    pub attestations: BTreeSet<Attestation>,
    /// Authored override, clamped to `[0,1]`. Absent means derived (§1.5).
    pub salience: Option<f32>,
    pub labels: BTreeSet<Label>,
}

impl Unit {
    pub fn new(core: UnitCore) -> Unit {
        Unit {
            core,
            attestations: BTreeSet::new(),
            salience: None,
            labels: BTreeSet::new(),
        }
    }

    pub fn with_attestation(mut self, a: Attestation) -> Unit {
        self.attestations.insert(a);
        self
    }

    pub fn with_label(mut self, l: Label) -> Unit {
        self.labels.insert(l);
        self
    }

    /// Set the authored salience override, clamped to `[0,1]` and quantised to 1/1024 -
    /// the same quantum packing renormalises against.
    pub fn with_salience(mut self, s: f32) -> Unit {
        self.salience = Some(crate::types::quantise(s.clamp(0.0, 1.0)));
        self
    }

    pub fn status(&self) -> Status {
        self.core.status
    }

    /// The lowest hop at which any agent attested this unit.
    pub fn first_hop(&self) -> Option<u32> {
        self.attestations.iter().map(|a| a.hop).min()
    }

    /// Distinct corroboration groups (§16.4).
    pub fn corroboration_groups(&self) -> usize {
        self.attestations
            .iter()
            .map(|a| a.corroboration_key())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, KernelType};
    use crate::types::epistemics::{Lod, SourceKind};
    use crate::types::provenance::{Hlc, Op, Rung};

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn claim(status: Status) -> UnitCoreBuilder {
        UnitCoreBuilder::new(KernelType::Claim, "p95 auth latency tripled", status)
    }

    #[test]
    fn a_gist_only_speculative_unit_is_valid() {
        let c = claim(Status::Speculative).build().unwrap();
        assert!(c.is_gist_only());
        assert_eq!(c.max_lod(), Lod::L0);
        assert_eq!(c.text_at(Lod::L0), Some("p95 auth latency tripled"));
        assert_eq!(c.text_at(Lod::L1), None);
    }

    #[test]
    fn an_empty_gist_is_e021() {
        for g in ["", "   ", "\n\t "] {
            let e = UnitCoreBuilder::new(KernelType::Claim, g, Status::Speculative)
                .build()
                .unwrap_err();
            assert_eq!(e, ShapeError::MissingGist);
        }
    }

    #[test]
    fn detail_without_body_is_e023() {
        let e = claim(Status::Speculative)
            .detail("per-shard breakdown")
            .build()
            .unwrap_err();
        assert_eq!(e, ShapeError::DetailWithoutBody);
    }

    #[test]
    fn detail_with_body_is_fine() {
        let c = claim(Status::Speculative)
            .body("Between the 2nd and the 9th, p95 rose from 180ms to 540ms.")
            .detail("Per-shard breakdown at hourly resolution.")
            .build()
            .unwrap();
        assert_eq!(c.max_lod(), Lod::L2);
        assert!(c.text_at(Lod::L2).is_some());
    }

    /// An empty body is the same thing as no body, so `detail` with an empty body is
    /// still `SMY-E023` rather than a unit with a hole in the middle.
    #[test]
    fn an_empty_body_does_not_satisfy_the_detail_requirement() {
        let e = claim(Status::Speculative)
            .body("")
            .detail("x")
            .build()
            .unwrap_err();
        assert_eq!(e, ShapeError::DetailWithoutBody);
    }

    #[test]
    fn unfounded_cannot_be_authored() {
        let e = claim(Status::Unfounded).build().unwrap_err();
        assert_eq!(e, ShapeError::UnfoundedAuthored);
    }

    #[test]
    fn measured_and_cited_require_a_source() {
        for s in [Status::Measured, Status::Cited] {
            let e = claim(s).build().unwrap_err();
            assert_eq!(e, ShapeError::SourceRequired, "{s} must require a source");
            assert!(claim(s)
                .source(SourceRef::new(SourceKind::Metric, "pool.wait_ms"))
                .build()
                .is_ok());
        }
    }

    #[test]
    fn derived_and_inferred_require_grounds() {
        for s in [Status::Derived, Status::Inferred] {
            let e = claim(s).build().unwrap_err();
            assert_eq!(e, ShapeError::GroundsRequired, "{s} must require grounds");
            assert!(claim(s).grounds([uid(1)]).build().is_ok());
        }
    }

    #[test]
    fn speculative_needs_neither_source_nor_grounds() {
        assert!(claim(Status::Speculative).build().is_ok());
    }

    #[test]
    fn text_is_normalised_to_nfc_on_construction() {
        // "cafe" + U+0301 must become U+00E9.
        let c = UnitCoreBuilder::new(
            KernelType::Prose,
            "cafe\u{301} latency",
            Status::Speculative,
        )
        .body("cafe\u{301}")
        .build()
        .unwrap();
        assert_eq!(c.gist, "caf\u{e9} latency");
        assert_eq!(c.body.as_deref(), Some("caf\u{e9}"));
        assert!(unicode_normalization::is_nfc(&c.gist));
    }

    /// Two cores that differ only in normalisation form must be the same core, or the
    /// same claim would hash to two uids depending on the editor that typed it.
    #[test]
    fn normalisation_collapses_equivalent_text_to_one_core() {
        let a = UnitCoreBuilder::new(KernelType::Prose, "cafe\u{301}", Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Prose, "caf\u{e9}", Status::Speculative)
            .build()
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn references_covers_deps_and_grounds() {
        let c = claim(Status::Inferred)
            .deps([uid(1), uid(2)])
            .grounds([uid(3)])
            .build()
            .unwrap();
        let refs: Vec<u8> = c.references().map(|u| u.as_bytes()[0]).collect();
        assert_eq!(refs, [1, 2, 3]);
    }

    #[test]
    fn dep_and_ground_sets_deduplicate_and_sort() {
        let c = claim(Status::Speculative)
            .deps([uid(3), uid(1), uid(3)])
            .build()
            .unwrap();
        let d: Vec<u8> = c.deps.iter().map(|u| u.as_bytes()[0]).collect();
        assert_eq!(d, [1, 3]);
    }

    #[test]
    fn units_carry_attestations_labels_and_salience_outside_the_core() {
        let ag = AgentId::new("human:vladimir").unwrap();
        let core = claim(Status::Speculative).build().unwrap();
        let u = Unit::new(core.clone())
            .with_attestation(Attestation::new(
                uid(9),
                ag.clone(),
                Op::Authored,
                Rung::Document,
                Hlc::zero(ag),
            ))
            .with_label(Label::new("c/auth-p95").unwrap())
            .with_salience(2.0);

        assert_eq!(u.core, core, "the core is untouched by any of this");
        assert_eq!(u.salience, Some(1.0), "salience is clamped to [0,1]");
        assert_eq!(u.labels.len(), 1);
        assert_eq!(u.status(), Status::Speculative);
        assert_eq!(u.first_hop(), Some(0));
    }

    #[test]
    fn salience_clamps_at_both_ends() {
        let core = claim(Status::Speculative).build().unwrap();
        assert_eq!(
            Unit::new(core.clone()).with_salience(-1.0).salience,
            Some(0.0)
        );
        assert_eq!(Unit::new(core).with_salience(0.25).salience, Some(0.25));
    }

    #[test]
    fn corroboration_counts_distinct_groups_not_attestations() {
        let core = claim(Status::Speculative).build().unwrap();
        let mk = |ag: &str| {
            let a = AgentId::new(ag).unwrap();
            Attestation::new(uid(1), a.clone(), Op::Authored, Rung::Model, Hlc::zero(a))
        };
        let u = Unit::new(core)
            .with_attestation(mk("model:anthropic/claude-opus-5").at_hop(0))
            .with_attestation(mk("model:anthropic/claude-opus-5").at_hop(1))
            .with_attestation(mk("model:openai/gpt").at_hop(0));
        assert_eq!(u.attestations.len(), 3);
        assert_eq!(
            u.corroboration_groups(),
            2,
            "the same agent under the same recipe is one group, whatever the hop"
        );
    }

    #[test]
    fn a_unit_with_no_attestations_has_no_hop() {
        let u = Unit::new(claim(Status::Speculative).build().unwrap());
        assert_eq!(u.first_hop(), None);
        assert_eq!(u.corroboration_groups(), 0);
    }

    #[test]
    fn extension_schemas_carry_opaque_payload() {
        let c = UnitCoreBuilder::new(
            SchemaId::parse("x.sre/incident").unwrap(),
            "an incident",
            Status::Speculative,
        )
        .payload(vec![0xA1, 0x00, 0x01])
        .build()
        .unwrap();
        assert!(!c.schema.is_kernel());
        assert_eq!(c.payload.as_deref(), Some(&[0xA1, 0x00, 0x01][..]));
    }
}
