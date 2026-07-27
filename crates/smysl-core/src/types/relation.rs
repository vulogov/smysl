//! The discourse plane: typed, immutable, independently attested edges (§3).

use core::fmt;
use std::collections::BTreeSet;

use crate::error::IdError;
use crate::ids::Uid;
use crate::types::provenance::Attestation;
use crate::types::unit::Extra;

/// The closed kernel relation set of §3.1, plus an extension escape hatch.
///
/// An unknown kind MUST be preserved and treated as `elaborates` for closure
/// (`SMY-W013`) - dropping it would break rule X, and refusing it would make a store
/// unreadable because someone else knew more than you.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RelKind {
    Elaborates,
    Contrasts,
    Concedes,
    Causes,
    Enables,
    Exemplifies,
    Conditions,
    Sequences,
    Answers,
    Rebuts,
    Warrant,
    Backs,
    Supersedes,
    Retracts,
    /// `x.<domain>/<kind>`, carried by text rather than by code.
    Extension(String),
}

impl RelKind {
    /// The fourteen kernel kinds, in wire-code order.
    pub const KERNEL: &'static [RelKind] = &[
        RelKind::Elaborates,
        RelKind::Contrasts,
        RelKind::Concedes,
        RelKind::Causes,
        RelKind::Enables,
        RelKind::Exemplifies,
        RelKind::Conditions,
        RelKind::Sequences,
        RelKind::Answers,
        RelKind::Rebuts,
        RelKind::Warrant,
        RelKind::Backs,
        RelKind::Supersedes,
        RelKind::Retracts,
    ];

    /// Codes 14-63 are reserved; extensions use the text form (Appendix B).
    pub const RESERVED_CODES: std::ops::RangeInclusive<u8> = 14..=63;

    pub const fn code(&self) -> Option<u8> {
        match self {
            RelKind::Elaborates => Some(0),
            RelKind::Contrasts => Some(1),
            RelKind::Concedes => Some(2),
            RelKind::Causes => Some(3),
            RelKind::Enables => Some(4),
            RelKind::Exemplifies => Some(5),
            RelKind::Conditions => Some(6),
            RelKind::Sequences => Some(7),
            RelKind::Answers => Some(8),
            RelKind::Rebuts => Some(9),
            RelKind::Warrant => Some(10),
            RelKind::Backs => Some(11),
            RelKind::Supersedes => Some(12),
            RelKind::Retracts => Some(13),
            RelKind::Extension(_) => None,
        }
    }

    pub fn from_code(c: u8) -> Option<RelKind> {
        RelKind::KERNEL.get(c as usize).cloned()
    }

    pub fn as_str(&self) -> &str {
        match self {
            RelKind::Elaborates => "elaborates",
            RelKind::Contrasts => "contrasts",
            RelKind::Concedes => "concedes",
            RelKind::Causes => "causes",
            RelKind::Enables => "enables",
            RelKind::Exemplifies => "exemplifies",
            RelKind::Conditions => "conditions",
            RelKind::Sequences => "sequences",
            RelKind::Answers => "answers",
            RelKind::Rebuts => "rebuts",
            RelKind::Warrant => "warrant",
            RelKind::Backs => "backs",
            RelKind::Supersedes => "supersedes",
            RelKind::Retracts => "retracts",
            RelKind::Extension(s) => s,
        }
    }

    pub fn parse(s: &str) -> Result<RelKind, IdError> {
        if let Some(k) = RelKind::KERNEL.iter().find(|k| k.as_str() == s) {
            return Ok(k.clone());
        }
        let err = || IdError {
            kind: "relation kind",
            found: s.to_string(),
        };
        let rest = s.strip_prefix("x.").ok_or_else(err)?;
        let (domain, kind) = rest.split_once('/').ok_or_else(err)?;
        let domain_ok = !domain.is_empty()
            && domain
                .bytes()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase())
            && domain
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_'));
        if domain_ok && crate::ids::is_ext_segment(kind) {
            Ok(RelKind::Extension(s.to_string()))
        } else {
            Err(err())
        }
    }

    pub const fn is_kernel(&self) -> bool {
        !matches!(self, RelKind::Extension(_))
    }

    /// How this kind behaves for closure. An unknown kind degrades to `elaborates`
    /// (`SMY-W013`) rather than being dropped.
    pub fn closure_kind(&self) -> RelKind {
        if self.is_kernel() {
            self.clone()
        } else {
            RelKind::Elaborates
        }
    }

    /// Whether rank flows along this edge in the salience computation (§16.4), alongside
    /// `grounds` and `deps`.
    pub const fn carries_support(&self) -> bool {
        matches!(self, RelKind::Causes | RelKind::Answers)
    }

    /// Kinds that order a thread. Kahn's algorithm runs over exactly these (§19).
    pub const fn is_ordering(&self) -> bool {
        matches!(
            self,
            RelKind::Sequences | RelKind::Causes | RelKind::Enables
        )
    }

    /// Whether this kind changes what should be believed, and so participates in the
    /// retraction and supersession machinery (§5.3).
    pub const fn is_lifecycle(&self) -> bool {
        matches!(self, RelKind::Supersedes | RelKind::Retracts)
    }
}

impl fmt::Display for RelKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// A typed edge (§3.1).
///
/// Relations are immutable, content-addressed, and independently attested, so "agent B
/// asserted that C rebuts D" is attributable rather than anonymous.
#[derive(Debug, Clone, PartialEq)]
pub struct Relation {
    pub kind: RelKind,
    pub from: Uid,
    pub to: Uid,
    /// Quantised to 1/1024 on encode, so hash stability does not depend on the float
    /// path that derived it.
    pub weight: Option<f32>,
    /// A unit carrying commentary on the edge itself.
    pub note: Option<Uid>,
    pub attestations: BTreeSet<Attestation>,
    pub extra: Extra,
}

impl Relation {
    pub fn new(kind: RelKind, from: Uid, to: Uid) -> Relation {
        Relation {
            kind,
            from,
            to,
            weight: None,
            note: None,
            attestations: BTreeSet::new(),
            extra: Extra::new(),
        }
    }

    /// Set the edge weight, clamped to `[0,1]` and quantised to 1/1024.
    ///
    /// Quantising here rather than at encode time means the in-memory value always equals
    /// the wire value, so a round trip is an identity rather than an approximation.
    pub fn with_weight(mut self, w: f32) -> Relation {
        self.weight = Some(crate::types::quantise(w.clamp(0.0, 1.0)));
        self
    }

    pub fn with_note(mut self, n: Uid) -> Relation {
        self.note = Some(n);
        self
    }

    pub fn with_attestation(mut self, a: Attestation) -> Relation {
        self.attestations.insert(a);
        self
    }

    /// The identity tuple. Two relations with the same tuple are the same edge, and
    /// merging them unions their attestations (§16.5 step 3).
    pub fn key(&self) -> (&RelKind, &Uid, &Uid) {
        (&self.kind, &self.from, &self.to)
    }

    /// The content address of this edge (§3.1).
    ///
    /// Relations are immutable and content-addressed like units, so "agent B asserted that
    /// C rebuts D" is attributable: the assertion has an identity of its own that an
    /// attestation can name.
    pub fn uid(&self) -> Uid {
        let mut bytes = Vec::with_capacity(72);
        bytes.push(0x03); // the Relation envelope code, so an edge cannot collide with a unit
        bytes.extend_from_slice(self.kind.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(self.from.as_bytes());
        bytes.extend_from_slice(self.to.as_bytes());
        Uid::from_bytes(crate::hash::hash_bytes(&bytes))
    }

    /// The earliest hop any agent attested this edge at.
    ///
    /// An edge nobody attested cannot be placed in time. That matters most for
    /// `retracts`: a retraction with no attestation is treated as having always been
    /// there, which is the conservative reading - an undated withdrawal is still a
    /// withdrawal.
    pub fn hop(&self) -> Option<u32> {
        self.attestations.iter().map(|a| a.hop).min()
    }

    /// Whether this edge is a rebuttal of `uid`, which is what rule R pins.
    pub fn rebuts(&self, uid: &Uid) -> bool {
        self.kind == RelKind::Rebuts && &self.to == uid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    #[test]
    fn fourteen_kernel_kinds_in_appendix_b_code_order() {
        assert_eq!(RelKind::KERNEL.len(), 14);
        let expected = [
            "elaborates",
            "contrasts",
            "concedes",
            "causes",
            "enables",
            "exemplifies",
            "conditions",
            "sequences",
            "answers",
            "rebuts",
            "warrant",
            "backs",
            "supersedes",
            "retracts",
        ];
        for (i, name) in expected.iter().enumerate() {
            let k = RelKind::from_code(i as u8).unwrap();
            assert_eq!(k.as_str(), *name);
            assert_eq!(k.code(), Some(i as u8));
        }
    }

    #[test]
    fn codes_past_the_kernel_set_are_reserved_not_assigned() {
        assert_eq!(RelKind::from_code(14), None);
        assert_eq!(RelKind::from_code(63), None);
        assert!(RelKind::RESERVED_CODES.contains(&14));
        assert!(RelKind::RESERVED_CODES.contains(&63));
        assert!(!RelKind::RESERVED_CODES.contains(&64));
    }

    #[test]
    fn kernel_kinds_round_trip_through_text() {
        for k in RelKind::KERNEL {
            assert_eq!(&RelKind::parse(k.as_str()).unwrap(), k);
        }
    }

    #[test]
    fn extension_kinds_take_the_x_form_and_have_no_code() {
        let k = RelKind::parse("x.sre/mitigates").unwrap();
        assert_eq!(k, RelKind::Extension("x.sre/mitigates".into()));
        assert_eq!(k.code(), None);
        assert!(!k.is_kernel());
    }

    #[test]
    fn malformed_relation_kinds_are_rejected() {
        for s in ["", "mitigates", "x.sre", "x./m", "x.sre/", "X.sre/m"] {
            assert!(RelKind::parse(s).is_err(), "`{s}` must be rejected");
        }
    }

    /// Rule X: an unknown kind stays routable. Treating it as `elaborates` for closure
    /// keeps the graph traversable without pretending to understand the edge.
    #[test]
    fn unknown_kinds_degrade_to_elaborates_for_closure() {
        let k = RelKind::parse("x.sre/mitigates").unwrap();
        assert_eq!(k.closure_kind(), RelKind::Elaborates);
        for kk in RelKind::KERNEL {
            assert_eq!(
                &kk.closure_kind(),
                kk,
                "a kernel kind is its own closure kind"
            );
        }
    }

    #[test]
    fn only_causes_and_answers_carry_support_rank() {
        let carriers: Vec<&str> = RelKind::KERNEL
            .iter()
            .filter(|k| k.carries_support())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(carriers, ["causes", "answers"]);
    }

    #[test]
    fn thread_ordering_runs_over_sequences_causes_and_enables() {
        let ordering: Vec<&str> = RelKind::KERNEL
            .iter()
            .filter(|k| k.is_ordering())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(ordering, ["causes", "enables", "sequences"]);
    }

    #[test]
    fn supersedes_and_retracts_are_the_lifecycle_kinds() {
        let lifecycle: Vec<&str> = RelKind::KERNEL
            .iter()
            .filter(|k| k.is_lifecycle())
            .map(|k| k.as_str())
            .collect();
        assert_eq!(lifecycle, ["supersedes", "retracts"]);
    }

    #[test]
    fn relations_identify_by_kind_and_endpoints() {
        let a = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6);
        let b = Relation::new(RelKind::Rebuts, uid(1), uid(2));
        assert_eq!(a.key(), b.key(), "weight is not identity");

        let c = Relation::new(RelKind::Causes, uid(1), uid(2));
        assert_ne!(a.key(), c.key());
    }

    #[test]
    fn relation_weight_is_quantised_on_construction() {
        let r = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6);
        assert_eq!(r.weight, Some(614.0 / 1024.0));
        assert!(crate::types::is_quantised(r.weight.unwrap()));
    }

    #[test]
    fn relation_weight_is_clamped() {
        assert_eq!(
            Relation::new(RelKind::Rebuts, uid(1), uid(2))
                .with_weight(9.0)
                .weight,
            Some(1.0)
        );
        assert_eq!(
            Relation::new(RelKind::Rebuts, uid(1), uid(2))
                .with_weight(-1.0)
                .weight,
            Some(0.0)
        );
    }

    /// Rule R pins rebuttals *of* a selected unit, so direction is load-bearing.
    #[test]
    fn rebuts_is_directional() {
        let r = Relation::new(RelKind::Rebuts, uid(1), uid(2));
        assert!(r.rebuts(&uid(2)));
        assert!(!r.rebuts(&uid(1)));

        let c = Relation::new(RelKind::Causes, uid(1), uid(2));
        assert!(!c.rebuts(&uid(2)));
    }

    /// An edge's identity is its own, distinct from either endpoint - otherwise an
    /// attestation could not say *which* assertion it was vouching for.
    #[test]
    fn a_relation_has_a_content_address_of_its_own() {
        let a = Relation::new(RelKind::Rebuts, uid(1), uid(2));
        let b = Relation::new(RelKind::Rebuts, uid(1), uid(2)).with_weight(0.6);
        assert_eq!(a.uid(), b.uid(), "weight is not identity");

        assert_ne!(
            a.uid(),
            Relation::new(RelKind::Causes, uid(1), uid(2)).uid()
        );
        assert_ne!(
            a.uid(),
            Relation::new(RelKind::Rebuts, uid(2), uid(1)).uid()
        );
        assert_ne!(a.uid(), uid(1));
        assert_ne!(a.uid(), uid(2));
    }

    #[test]
    fn an_unattested_relation_has_no_hop() {
        assert_eq!(Relation::new(RelKind::Rebuts, uid(1), uid(2)).hop(), None);
    }

    #[test]
    fn relations_carry_their_own_attestations() {
        use crate::ids::AgentId;
        use crate::types::provenance::{Attestation, Hlc, Op, Rung};
        let ag = AgentId::new("model:openai/gpt").unwrap();
        let r = Relation::new(RelKind::Rebuts, uid(1), uid(2))
            .with_note(uid(3))
            .with_attestation(Attestation::new(
                uid(4),
                ag.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::zero(ag),
            ));
        assert_eq!(r.attestations.len(), 1);
        assert_eq!(r.note, Some(uid(3)));
    }
}
