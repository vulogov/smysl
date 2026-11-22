//! What a call may do, and how long it may take (RFC SMYSL-4 §9).

use std::collections::BTreeSet;
use std::path::PathBuf;

/// Which words a script may name. The gate checks this; it is not the boundary (D-B2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum WordPolicy {
    /// Only these words. [`WordPolicy::default_read_only_allowlist`] is the default.
    Allowlist(BTreeSet<String>),
    /// Any word but the refused ones of §6.1 to §6.3, which are always refused, and these.
    Denylist(BTreeSet<String>),
}

impl WordPolicy {
    /// §6.4: every word of Bund2 0.56.0 that is not refused. A word Bund2 adds later is not on
    /// it until its list is read again (§6.3).
    pub fn default_read_only_allowlist() -> WordPolicy {
        WordPolicy::Allowlist(crate::words::default_allowlist())
    }
}

/// Whether the process runs in the sandbox (§5.5, §5.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Trust {
    /// Contained by the operating system. Refused with
    /// [`BundError::SandboxUnavailable`](crate::BundError::SandboxUnavailable) on a platform
    /// without the sandbox.
    #[default]
    Sandboxed,
    /// For a script written by the person running it. Never the default for anything a model
    /// wrote.
    Trusted,
}

/// The limits and choices of one call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct QueryPolicy {
    /// An explicit `bund2`; `None` looks for `bund2` on `PATH`.
    pub bund_binary: Option<PathBuf>,
    pub word_policy: WordPolicy,
    pub trust: Trust,
    pub max_script_bytes: usize,
    pub max_nesting_depth: usize,
    pub max_output_bytes: usize,
    pub max_wall_ms: u64,
    /// To be applied with `kb.inferences` to every KB `smysl.kb` builds. Bund2 has no other step
    /// limit. **Not applied yet**: it needs `smysl.kb` wrapped in the preamble, which is BQ-P3's.
    /// Until then the wall clock is the only bound on a logic query.
    pub max_inferences: Option<u64>,
    /// Fail when a returned uid is not in the source store.
    pub verify_uids: bool,
}

impl Default for QueryPolicy {
    fn default() -> QueryPolicy {
        QueryPolicy {
            bund_binary: None,
            word_policy: WordPolicy::default_read_only_allowlist(),
            trust: Trust::Sandboxed,
            max_script_bytes: 65_536,
            max_nesting_depth: 32,
            max_output_bytes: 1_048_576,
            max_wall_ms: 2_000,
            max_inferences: Some(1_000_000),
            verify_uids: true,
        }
    }
}

/// Builders, since a caller outside this crate cannot name every field of a
/// `#[non_exhaustive]` struct.
impl QueryPolicy {
    pub fn with_binary(mut self, path: impl Into<PathBuf>) -> QueryPolicy {
        self.bund_binary = Some(path.into());
        self
    }

    pub fn with_word_policy(mut self, words: WordPolicy) -> QueryPolicy {
        self.word_policy = words;
        self
    }

    pub fn with_trust(mut self, trust: Trust) -> QueryPolicy {
        self.trust = trust;
        self
    }

    pub fn with_max_script_bytes(mut self, n: usize) -> QueryPolicy {
        self.max_script_bytes = n;
        self
    }

    pub fn with_max_nesting_depth(mut self, n: usize) -> QueryPolicy {
        self.max_nesting_depth = n;
        self
    }

    pub fn with_max_output_bytes(mut self, n: usize) -> QueryPolicy {
        self.max_output_bytes = n;
        self
    }

    pub fn with_max_wall_ms(mut self, ms: u64) -> QueryPolicy {
        self.max_wall_ms = ms;
        self
    }

    pub fn with_max_inferences(mut self, n: Option<u64>) -> QueryPolicy {
        self.max_inferences = n;
        self
    }

    pub fn with_verify_uids(mut self, verify: bool) -> QueryPolicy {
        self.verify_uids = verify;
        self
    }
}
