//! `smysl-thread` - the five thread schemas and deterministic derivation (§19).
//!
//! The schema set is closed in 0.1 (D-4): the role-assignment rule language is not stable
//! enough to freeze as a user-facing surface. HJSON-defined schemas are deferred to 0.2.
//!
//! Filled by SM-P11.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

/// The closed thread-schema set of 0.1 (§4, D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ThreadSchema {
    Analysis,
    Narrative,
    Brief,
    Qa,
    Plan,
}

impl ThreadSchema {
    pub const ALL: &'static [ThreadSchema] = &[
        ThreadSchema::Analysis,
        ThreadSchema::Narrative,
        ThreadSchema::Brief,
        ThreadSchema::Qa,
        ThreadSchema::Plan,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ThreadSchema::Analysis => "analysis",
            ThreadSchema::Narrative => "narrative",
            ThreadSchema::Brief => "brief",
            ThreadSchema::Qa => "qa",
            ThreadSchema::Plan => "plan",
        }
    }

    pub fn parse(s: &str) -> Option<ThreadSchema> {
        ThreadSchema::ALL.iter().copied().find(|t| t.as_str() == s)
    }
}

impl core::fmt::Display for ThreadSchema {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_set_is_closed_at_five() {
        assert_eq!(ThreadSchema::ALL.len(), 5);
    }

    #[test]
    fn schema_names_round_trip() {
        for &s in ThreadSchema::ALL {
            assert_eq!(ThreadSchema::parse(s.as_str()), Some(s));
        }
        assert_eq!(ThreadSchema::parse("essay"), None);
    }
}
