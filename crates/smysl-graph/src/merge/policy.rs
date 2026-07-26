//! Merge policies (§5.3).
//!
//! Three knobs, each of which decides what happens when the store is asked to believe two
//! things at once. The defaults are the conservative ones: materialise the disagreement,
//! honour retractions transitively, and let only the origin retract.

use core::fmt;

/// What to do when two units supersede the same target and neither supersedes the other.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum SupersessionPolicy {
    /// Take the newest by HLC. Convenient, and quietly destroys the disagreement - which
    /// is the failure mode the whole design exists to avoid (F7).
    Latest,
    /// Keep every successor and say nothing. Convergent but uninformative.
    All,
    /// Materialise a contention. The default: merge converges without adjudicating.
    #[default]
    Contend,
}

impl SupersessionPolicy {
    pub const ALL: &'static [SupersessionPolicy] = &[
        SupersessionPolicy::Latest,
        SupersessionPolicy::All,
        SupersessionPolicy::Contend,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            SupersessionPolicy::Latest => "latest",
            SupersessionPolicy::All => "all",
            SupersessionPolicy::Contend => "contend",
        }
    }

    pub fn parse(s: &str) -> Option<SupersessionPolicy> {
        SupersessionPolicy::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == s)
    }
}

impl fmt::Display for SupersessionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How far a retraction reaches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RetractionPolicy {
    /// Transitive over `grounds`: a unit whose support has all been retracted is orphaned
    /// (`SMY-E050`) and reads as `unfounded`. The default.
    #[default]
    Strict,
    /// The retracted unit is flagged (`SMY-W052`) and kept believable.
    Advisory,
    /// Retractions are recorded and have no effect on status.
    Ignore,
}

impl RetractionPolicy {
    pub const ALL: &'static [RetractionPolicy] = &[
        RetractionPolicy::Strict,
        RetractionPolicy::Advisory,
        RetractionPolicy::Ignore,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            RetractionPolicy::Strict => "strict",
            RetractionPolicy::Advisory => "advisory",
            RetractionPolicy::Ignore => "ignore",
        }
    }

    pub fn parse(s: &str) -> Option<RetractionPolicy> {
        RetractionPolicy::ALL
            .iter()
            .copied()
            .find(|p| p.as_str() == s)
    }

    /// Whether a retraction changes what a unit's effective status is.
    pub const fn affects_status(self) -> bool {
        matches!(self, RetractionPolicy::Strict | RetractionPolicy::Advisory)
    }

    /// Whether a retraction propagates to units grounded on the retracted one.
    pub const fn is_transitive(self) -> bool {
        matches!(self, RetractionPolicy::Strict)
    }
}

impl fmt::Display for RetractionPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who may retract a unit.
///
/// The default is the defence against a single adversarial agent censoring a corpus: only
/// an agent that attested a unit can retract it, so nobody can silence work they had no
/// hand in (§29).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum RetractionAuthority {
    /// Only an agent that attested the unit.
    #[default]
    Origin,
    /// Anyone.
    Any,
    /// At least `n` distinct agents must issue the retraction.
    Quorum(u32),
}

impl RetractionAuthority {
    pub fn as_string(self) -> String {
        match self {
            RetractionAuthority::Origin => "origin".into(),
            RetractionAuthority::Any => "any".into(),
            RetractionAuthority::Quorum(n) => format!("quorum:{n}"),
        }
    }

    pub fn parse(s: &str) -> Option<RetractionAuthority> {
        match s {
            "origin" => Some(RetractionAuthority::Origin),
            "any" => Some(RetractionAuthority::Any),
            other => other
                .strip_prefix("quorum:")
                .and_then(|n| n.parse().ok())
                .filter(|n| *n > 0)
                .map(RetractionAuthority::Quorum),
        }
    }

    /// How many distinct retracting agents are needed.
    pub const fn required_agents(self) -> u32 {
        match self {
            RetractionAuthority::Origin | RetractionAuthority::Any => 1,
            RetractionAuthority::Quorum(n) => n,
        }
    }

    /// Whether a retracting agent must already have attested the unit.
    pub const fn requires_origin(self) -> bool {
        matches!(self, RetractionAuthority::Origin)
    }
}

impl fmt::Display for RetractionAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_conservative_ones() {
        assert_eq!(SupersessionPolicy::default(), SupersessionPolicy::Contend);
        assert_eq!(RetractionPolicy::default(), RetractionPolicy::Strict);
        assert_eq!(RetractionAuthority::default(), RetractionAuthority::Origin);
    }

    #[test]
    fn supersession_policies_round_trip() {
        for p in SupersessionPolicy::ALL {
            assert_eq!(SupersessionPolicy::parse(p.as_str()), Some(*p));
        }
        assert_eq!(SupersessionPolicy::parse("newest"), None);
    }

    #[test]
    fn retraction_policies_round_trip() {
        for p in RetractionPolicy::ALL {
            assert_eq!(RetractionPolicy::parse(p.as_str()), Some(*p));
        }
        assert_eq!(RetractionPolicy::parse("soft"), None);
    }

    #[test]
    fn only_strict_propagates_and_only_ignore_is_inert() {
        assert!(RetractionPolicy::Strict.is_transitive());
        assert!(!RetractionPolicy::Advisory.is_transitive());
        assert!(!RetractionPolicy::Ignore.is_transitive());

        assert!(RetractionPolicy::Strict.affects_status());
        assert!(RetractionPolicy::Advisory.affects_status());
        assert!(!RetractionPolicy::Ignore.affects_status());
    }

    #[test]
    fn authorities_round_trip_including_quorum() {
        for a in [
            RetractionAuthority::Origin,
            RetractionAuthority::Any,
            RetractionAuthority::Quorum(3),
        ] {
            assert_eq!(RetractionAuthority::parse(&a.as_string()), Some(a));
        }
        assert_eq!(
            RetractionAuthority::parse("quorum:0"),
            None,
            "a quorum of nobody"
        );
        assert_eq!(RetractionAuthority::parse("quorum:x"), None);
        assert_eq!(RetractionAuthority::parse("nobody"), None);
    }

    #[test]
    fn quorum_sizes_are_reported() {
        assert_eq!(RetractionAuthority::Origin.required_agents(), 1);
        assert_eq!(RetractionAuthority::Any.required_agents(), 1);
        assert_eq!(RetractionAuthority::Quorum(3).required_agents(), 3);
    }

    /// Only `origin` demands that the retractor already had a hand in the unit - that is
    /// the anti-censorship property.
    #[test]
    fn only_origin_requires_prior_involvement() {
        assert!(RetractionAuthority::Origin.requires_origin());
        assert!(!RetractionAuthority::Any.requires_origin());
        assert!(!RetractionAuthority::Quorum(2).requires_origin());
    }
}
