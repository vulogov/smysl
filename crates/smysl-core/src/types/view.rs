//! Views and granularity (§1.6, §4).
//!
//! **A view is not a container.** Everything reachable from `roots` via `deps`, `grounds`,
//! and discourse edges is in it; nothing is copied or owned. That is why merging views is
//! a union, a unit belongs to many views at zero cost, and there is no document to
//! conflict over.

use core::fmt;
use std::collections::BTreeSet;

use crate::ids::{LangTag, SchemaId, ThreadId, Uid, ViewId};
use crate::types::unit::Extra;

/// How much a single unit is allowed to say (§1.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Admission {
    /// One assertion per unit. The default, and what makes rule M checkable per unit.
    SingleAssertion = 0,
    /// A topic per unit. Coarse granularity trades checkability for narrative flow.
    Topical = 1,
}

impl Admission {
    pub const ALL: &'static [Admission] = &[Admission::SingleAssertion, Admission::Topical];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Admission> {
        match v {
            0 => Some(Admission::SingleAssertion),
            1 => Some(Admission::Topical),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Admission::SingleAssertion => "single-assertion",
            Admission::Topical => "topical",
        }
    }

    pub fn parse(s: &str) -> Option<Admission> {
        Admission::ALL.iter().copied().find(|a| a.as_str() == s)
    }
}

impl fmt::Display for Admission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A granularity profile.
///
/// Granularity constrains *production*, not the store: mixed granularity in a merged
/// store is legal, not an error (D-5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GranularityProfile {
    pub profile: String,
    pub l0_max: u32,
    pub l1_min: u32,
    pub l1_max: u32,
    pub admission: Admission,
}

impl GranularityProfile {
    /// Narrative: topical admission, long bodies.
    pub fn coarse() -> GranularityProfile {
        GranularityProfile {
            profile: "coarse".into(),
            l0_max: 30,
            l1_min: 120,
            l1_max: 400,
            admission: Admission::Topical,
        }
    }

    /// Reports, docs, briefs.
    pub fn standard() -> GranularityProfile {
        GranularityProfile {
            profile: "default".into(),
            l0_max: 30,
            l1_min: 40,
            l1_max: 120,
            admission: Admission::SingleAssertion,
        }
    }

    /// Analysis and research traces.
    pub fn fine() -> GranularityProfile {
        GranularityProfile {
            profile: "fine".into(),
            l0_max: 30,
            l1_min: 20,
            l1_max: 60,
            admission: Admission::SingleAssertion,
        }
    }

    pub fn preset(name: &str) -> Option<GranularityProfile> {
        match name {
            "coarse" => Some(GranularityProfile::coarse()),
            "default" => Some(GranularityProfile::standard()),
            "fine" => Some(GranularityProfile::fine()),
            _ => None,
        }
    }

    pub fn body_in_range(&self, tokens: u32) -> bool {
        (self.l1_min..=self.l1_max).contains(&tokens)
    }

    pub fn gist_within_bound(&self, tokens: u32) -> bool {
        tokens <= self.l0_max
    }
}

impl Default for GranularityProfile {
    fn default() -> GranularityProfile {
        GranularityProfile::standard()
    }
}

/// A named root set plus the threads published over it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub id: ViewId,
    pub roots: BTreeSet<Uid>,
    pub threads: BTreeSet<ThreadId>,
    /// Schemas a consumer must implement for full fidelity. Missing them means degraded,
    /// not refused - unless a kernel major is missing (`SMY-E002`).
    pub requires: BTreeSet<SchemaId>,
    pub granularity: GranularityProfile,
    pub intent: String,
    pub lang: LangTag,
    pub extra: Extra,
}

impl View {
    pub fn new(id: ViewId, intent: impl Into<String>) -> View {
        View {
            id,
            roots: BTreeSet::new(),
            threads: BTreeSet::new(),
            requires: BTreeSet::new(),
            granularity: GranularityProfile::default(),
            intent: intent.into(),
            lang: LangTag::default(),
            extra: Extra::new(),
        }
    }

    pub fn with_roots(mut self, r: impl IntoIterator<Item = Uid>) -> View {
        self.roots = r.into_iter().collect();
        self
    }

    pub fn with_threads(mut self, t: impl IntoIterator<Item = ThreadId>) -> View {
        self.threads = t.into_iter().collect();
        self
    }

    pub fn requiring(mut self, s: impl IntoIterator<Item = SchemaId>) -> View {
        self.requires = s.into_iter().collect();
        self
    }

    pub fn with_granularity(mut self, g: GranularityProfile) -> View {
        self.granularity = g;
        self
    }

    pub fn with_lang(mut self, l: LangTag) -> View {
        self.lang = l;
        self
    }

    /// Negotiation outcome for a consumer implementing `implemented` (§2.2).
    pub fn negotiate(&self, implemented: &BTreeSet<SchemaId>, kernel_major_ok: bool) -> Fidelity {
        if !kernel_major_ok {
            Fidelity::Refuse
        } else if self.requires.is_subset(implemented) {
            Fidelity::Full
        } else {
            Fidelity::Degraded
        }
    }

    /// Merging two views is a union: neither owns anything, so there is nothing to
    /// reconcile.
    pub fn union(mut self, other: &View) -> View {
        self.roots.extend(other.roots.iter().copied());
        self.threads.extend(other.threads.iter().cloned());
        self.requires.extend(other.requires.iter().cloned());
        self
    }
}

/// What a consumer can do with a view (§2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Fidelity {
    Full,
    Degraded,
    /// A consumer MUST NOT silently degrade here.
    Refuse,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::KernelType;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    #[test]
    fn the_three_presets_match_section_1_6() {
        let c = GranularityProfile::coarse();
        assert_eq!((c.l1_min, c.l1_max), (120, 400));
        assert_eq!(c.admission, Admission::Topical);

        let d = GranularityProfile::standard();
        assert_eq!((d.l1_min, d.l1_max), (40, 120));
        assert_eq!(d.admission, Admission::SingleAssertion);

        let f = GranularityProfile::fine();
        assert_eq!((f.l1_min, f.l1_max), (20, 60));
        assert_eq!(f.admission, Admission::SingleAssertion);

        for p in [&c, &d, &f] {
            assert_eq!(p.l0_max, 30, "the gist bound does not vary by profile");
        }
    }

    #[test]
    fn default_is_the_default_profile() {
        assert_eq!(
            GranularityProfile::default(),
            GranularityProfile::standard()
        );
        assert_eq!(GranularityProfile::default().profile, "default");
    }

    #[test]
    fn presets_resolve_by_name() {
        for n in ["coarse", "default", "fine"] {
            assert_eq!(GranularityProfile::preset(n).unwrap().profile, n);
        }
        assert!(GranularityProfile::preset("medium").is_none());
    }

    #[test]
    fn range_checks_are_inclusive() {
        let d = GranularityProfile::standard();
        assert!(!d.body_in_range(39));
        assert!(d.body_in_range(40));
        assert!(d.body_in_range(120));
        assert!(!d.body_in_range(121));
        assert!(d.gist_within_bound(30));
        assert!(!d.gist_within_bound(31));
    }

    #[test]
    fn admission_round_trips() {
        for &a in Admission::ALL {
            assert_eq!(Admission::from_u8(a.as_u8()), Some(a));
            assert_eq!(Admission::parse(a.as_str()), Some(a));
        }
        assert_eq!(Admission::from_u8(2), None);
    }

    #[test]
    fn a_view_is_a_root_set_not_a_container() {
        let v = View::new(ViewId::new("v/incident").unwrap(), "incident-brief")
            .with_roots([uid(1), uid(2)])
            .with_threads([ThreadId::new("t/brief").unwrap()]);
        assert_eq!(v.roots.len(), 2);
        assert_eq!(v.threads.len(), 1);
        assert_eq!(v.lang.as_str(), "en");
        assert_eq!(v.granularity, GranularityProfile::standard());
    }

    #[test]
    fn merging_views_is_a_union() {
        let a = View::new(ViewId::new("v/a").unwrap(), "a").with_roots([uid(1), uid(2)]);
        let b = View::new(ViewId::new("v/b").unwrap(), "b").with_roots([uid(2), uid(3)]);
        let m = a.union(&b);
        let roots: Vec<u8> = m.roots.iter().map(|u| u.as_bytes()[0]).collect();
        assert_eq!(roots, [1, 2, 3]);
    }

    #[test]
    fn negotiation_is_full_when_requirements_are_implemented() {
        let need: BTreeSet<SchemaId> = [SchemaId::parse("x.sre/incident").unwrap()]
            .into_iter()
            .collect();
        let v = View::new(ViewId::new("v/x").unwrap(), "i").requiring(need.clone());
        assert_eq!(v.negotiate(&need, true), Fidelity::Full);
    }

    #[test]
    fn an_unimplemented_extension_degrades_rather_than_refusing() {
        let need: BTreeSet<SchemaId> = [SchemaId::parse("x.sre/incident").unwrap()]
            .into_iter()
            .collect();
        let have: BTreeSet<SchemaId> = [SchemaId::from(KernelType::Claim)].into_iter().collect();
        let v = View::new(ViewId::new("v/x").unwrap(), "i").requiring(need);
        assert_eq!(v.negotiate(&have, true), Fidelity::Degraded);
    }

    /// A missing kernel major is the one case where silent degradation is forbidden.
    #[test]
    fn a_missing_kernel_major_refuses() {
        let v = View::new(ViewId::new("v/x").unwrap(), "i");
        assert_eq!(v.negotiate(&BTreeSet::new(), false), Fidelity::Refuse);
    }

    #[test]
    fn an_empty_requirement_set_is_always_full_fidelity() {
        let v = View::new(ViewId::new("v/x").unwrap(), "i");
        assert_eq!(v.negotiate(&BTreeSet::new(), true), Fidelity::Full);
    }
}
