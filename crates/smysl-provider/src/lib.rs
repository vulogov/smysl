//! `smysl-provider` - the model boundary (§21).
//!
//! This is the only crate in the workspace that links an async runtime or an HTTP client
//! (rule B, D-12). It owns a current-thread tokio runtime on a dedicated OS thread and
//! speaks to synchronous callers - including the TUI - over an `std::sync::mpsc` channel,
//! so async never appears in an event path (§21.5).
//!
//! No vendor SDKs: five SDKs would mean five dependency subtrees for what is, in each
//! case, one POST with a JSON body. `serde_json` is used for provider wire formats only,
//! never for smysl's canonical form.
//!
//! Filled by SM-P13 (trait, runtime, Ollama) and SM-P14 (the four hosted mappers).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::error::ProviderError;

/// How a provider can be made to emit structured output (§21.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum StructuredMode {
    JsonSchema,
    ToolForce,
    JsonMode,
    Grammar,
    None,
}

impl StructuredMode {
    /// Whether the provider structurally guarantees schema conformance. `JsonMode` does
    /// not - which is why D-9 leaves that provider's default path to measured E9.
    pub const fn is_enforced(self) -> bool {
        matches!(
            self,
            StructuredMode::JsonSchema | StructuredMode::ToolForce | StructuredMode::Grammar
        )
    }
}

/// Which mappers this build contains.
pub fn compiled_mappers() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(feature = "ollama") {
        v.push("ollama");
    }
    if cfg!(feature = "anthropic") {
        v.push("anthropic");
    }
    if cfg!(feature = "openai") {
        v.push("openai");
    }
    if cfg!(feature = "gemini") {
        v.push("gemini");
    }
    if cfg!(feature = "deepseek") {
        v.push("deepseek");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_mode_is_not_an_enforced_structure() {
        assert!(!StructuredMode::JsonMode.is_enforced());
        assert!(!StructuredMode::None.is_enforced());
        assert!(StructuredMode::JsonSchema.is_enforced());
        assert!(StructuredMode::ToolForce.is_enforced());
        assert!(StructuredMode::Grammar.is_enforced());
    }

    #[test]
    fn mapper_list_is_sorted_and_deduplicated() {
        let m = compiled_mappers();
        let mut sorted = m.clone();
        sorted.dedup();
        assert_eq!(m.len(), sorted.len());
    }
}
