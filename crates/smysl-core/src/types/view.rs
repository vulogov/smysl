//! Views and granularity (§1.6, §4).
//!
//! **A view is not a container.** Everything reachable from `roots` via `deps`, `grounds`,
//! and discourse edges is in it; nothing is copied or owned. That is why merging views is
//! a union, a unit belongs to many views at zero cost, and there is no document to
//! conflict over.

use core::fmt;
use std::collections::BTreeSet;

use crate::ids::{LangTag, SchemaId, ThreadId, Uid, ViewId};
use crate::types::estimate::ProfileEstimator;
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
    /// A code this build does not know (1.10, SMYSL-2.3 A-8.1).
    ///
    /// The fifth enumeration to open, and the last. 1.9 reserved 255 here and kept the
    /// enumeration closed, on the argument that `l0_max` and the granularity passes read
    /// admission and a reader that guessed would report the wrong verdict. That argument was
    /// right about guessing and wrong about the remedy: failing the decode does not avoid a
    /// wrong verdict, it refuses to open the store at all (F-12). Treating the value as
    /// unknown is what the argument actually asks for — the single-assertion check does not
    /// run, which is not a verdict either way, and `SMY-W409` says so.
    ///
    /// The raw code lives on [`GranularityProfile`], not here, so it re-encodes unchanged.
    Unknown = 255,
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
            255 => Some(Admission::Unknown),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Admission::SingleAssertion => "single-assertion",
            Admission::Topical => "topical",
            Admission::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Admission> {
        Admission::ALL.iter().copied().find(|a| a.as_str() == s)
    }
}

impl fmt::Display for Admission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// A granularity profile.
///
/// Granularity constrains *production*, not the store: mixed granularity in a merged
/// store is legal, not an error (D-5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct GranularityProfile {
    pub profile: String,
    pub l0_max: u32,
    pub l1_min: u32,
    pub l1_max: u32,
    pub admission: Admission,
    /// The wire code behind `admission`, when `admission` is `Unknown` (1.10).
    ///
    /// A view is not inside any uid, so this is not an identity hazard the way
    /// `SourceRef.kind_code` is. It is a plain round-trip obligation: two peers that disagree
    /// about whether to keep an admission code compute different record-set digests for the
    /// same store, and rule U is stated on that digest.
    admission_code: Option<u8>,
    /// How `l0_max` and `l1_range` are counted (F-2). `Unset` is the pre-F-2 meaning,
    /// `smysl_core::tokens`, and encodes to no key at all.
    pub estimator: ProfileEstimator,
    /// Keys this build does not know, kept so a profile survives a round trip.
    ///
    /// §8.1 permits a new key in any record body above that record's highest, and obliges an
    /// older reader to round-trip it byte for byte. This sub-map was the one place that was
    /// not true: `dec_granularity` collected unknown keys and dropped them, so a profile
    /// carrying one re-encoded shorter. Unlike `SourceRef.extra` this is not an identity
    /// hazard — a view is not inside a uid — but it is a plain C-Read failure, and two peers
    /// disagreeing about whether to keep a key compute different record-set digests for the
    /// same store.
    pub extra: Extra,
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
            admission_code: None,
            estimator: ProfileEstimator::Unset,
            extra: Extra::new(),
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
            admission_code: None,
            estimator: ProfileEstimator::Unset,
            extra: Extra::new(),
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
            admission_code: None,
            estimator: ProfileEstimator::Unset,
            extra: Extra::new(),
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

    /// The wire code of `admission`: the raw byte for an unknown one, the discriminant else.
    pub fn admission_code(&self) -> u8 {
        self.admission_code
            .unwrap_or_else(|| self.admission.as_u8())
    }

    /// Record an admission this build does not know, preserving its code (A-8.1).
    ///
    /// Refuses 255 itself, which is reserved and never assigned, and refuses a code this build
    /// *does* know — those have a named variant and belong in `admission`. The same shape as
    /// [`SourceRef::with_unknown_kind`](crate::types::epistemics::SourceRef::with_unknown_kind),
    /// which is the point: five enumerations are open and one design serves all five.
    pub fn with_unknown_admission(mut self, code: u8) -> Option<GranularityProfile> {
        if code == 255 || Admission::from_u8(code).is_some() {
            return None;
        }
        self.admission = Admission::Unknown;
        self.admission_code = Some(code);
        Some(self)
    }

    pub fn body_in_range(&self, tokens: u32) -> bool {
        (self.l1_min..=self.l1_max).contains(&tokens)
    }

    pub fn gist_within_bound(&self, tokens: u32) -> bool {
        tokens <= self.l0_max
    }

    /// Count `text` the way this profile's bounds are expressed.
    ///
    /// `None` when the profile names an estimator this build does not have: the bounds are
    /// then unevaluable, and a caller must say so (`SMY-W025`) rather than fall back to a
    /// different count and report a breach it cannot justify.
    pub fn tokens(&self, text: &str) -> Option<u32> {
        self.estimator.estimator().map(|e| e.count(text))
    }

    /// The longest prefix of `text` that fits `l0_max`, as a byte length.
    ///
    /// `None` for an unknown estimator, for the same reason as [`Self::tokens`].
    pub fn fit_gist(&self, text: &str) -> Option<usize> {
        self.estimator.estimator().map(|e| e.fit(text, self.l0_max))
    }

    /// As [`Self::fit_gist`], leaving room for `suffix` — an ellipsis on a shortened gist,
    /// whose own cost depends on the estimator and so cannot be reserved in bytes.
    pub fn fit_gist_with(&self, text: &str, suffix: &str) -> Option<usize> {
        self.estimator
            .estimator()
            .map(|e| e.fit_with_suffix(text, suffix, self.l0_max))
    }
}

impl Default for GranularityProfile {
    fn default() -> GranularityProfile {
        GranularityProfile::standard()
    }
}

/// A named root set plus the threads published over it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
