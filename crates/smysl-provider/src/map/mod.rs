//! Mappers (§21.2).
//!
//! Each is one file implementing five things: build the request body, apply the structured
//! mechanism for its mode, parse the completion, extract usage, and map errors onto
//! [`ProviderError`]. **No vendor SDK** - five SDKs would be five dependency subtrees for
//! what is, in each case, one POST with a JSON body.
//!
//! **Verification status.** The RFC's implementation note asks that endpoint paths and field
//! names be checked against a running server. Three have been; two have not, and say so here
//! rather than letting a reader assume the table is uniform.
//!
//! | Mapper | Status | Verified against |
//! |---|---|---|
//! | `ollama` | verified | local server, `llama3.2` |
//! | `deepseek` | verified | live endpoint, `deepseek-chat` |
//! | `gemini` | verified | live endpoint, `gemini-3.5-flash-lite` / `-flash` (2026-07-27) |
//! | `anthropic` | **implemented, but not tested** | recorded fixtures only |
//! | `openai` | **implemented, but not tested** | recorded fixtures only |
//!
//! An untested mapper is a reading of the documentation, and this crate has already had one
//! reading turn out wrong: Gemini's response schema was written as a subset of draft 2020-12
//! and is not one, which no fixture could have caught. Treat the two above the same way -
//! verify before relying on them. Endpoints change without notice; the mapper contract does
//! not.

pub mod auth;

// The concrete mappers are `#[doc(hidden)]` rather than `pub(crate)`, and the distinction is
// deliberate. `build` returns `Box<dyn Provider>`, so no consumer needs `Ollama` by name — but
// four integration tests construct these types directly, and an integration test is a separate
// crate that sees `pub` and not `pub(crate)`. Hiding keeps those tests compiling while taking
// the types out of the contract: `cargo-semver-checks` has a `struct_now_doc_hidden` lint that
// treats hiding as removal from the public API, and `cargo public-api` drops them too.
//
// The tests are worth the arrangement. Mutation testing in 0.12 put 25 survivors on the
// `status_error` cluster alone, and `capabilities_are_honest` needs the concrete types because
// what it checks is that each mapper's declared capabilities match what it actually does.
#[cfg(feature = "anthropic")]
#[doc(hidden)]
pub mod anthropic;
#[cfg(feature = "deepseek")]
#[doc(hidden)]
pub mod deepseek;
#[cfg(feature = "gemini")]
#[doc(hidden)]
pub mod gemini;
#[cfg(feature = "ollama")]
#[doc(hidden)]
pub mod ollama;
#[cfg(feature = "openai")]
#[doc(hidden)]
pub mod openai;
#[cfg(any(feature = "openai", feature = "deepseek"))]
#[doc(hidden)]
pub mod openai_compat;

/// Full jitter needs a random number and the pure crates have none. The nanosecond of the
/// wall clock is a poor random source and a perfectly good jitter source: it only has to
/// decorrelate clients, not resist anyone.
#[cfg(feature = "http-client")]
pub(crate) fn jitter() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() % 1000) as f64 / 1000.0)
        .unwrap_or(0.5)
}

use smysl_core::error::ProviderError;

use crate::config::ProviderConfig;
use crate::Provider;

/// Build the provider a configuration names.
///
/// A `kind` no compiled mapper handles is an error rather than a silent omission: a
/// deployment that configured `anthropic` in a build without it should be told, not left
/// wondering why a task never routes.
pub fn build(cfg: &ProviderConfig) -> Result<Box<dyn Provider>, ProviderError> {
    match cfg.kind.as_str() {
        #[cfg(feature = "ollama")]
        "ollama" => Ok(Box::new(ollama::Ollama::new(cfg.clone()))),
        #[cfg(feature = "deepseek")]
        "deepseek" => Ok(Box::new(deepseek::DeepSeek::new(cfg.clone()))),
        #[cfg(feature = "anthropic")]
        "anthropic" => Ok(Box::new(anthropic::Anthropic::new(cfg.clone()))),
        #[cfg(feature = "openai")]
        "openai" => Ok(Box::new(openai::OpenAi::new(cfg.clone()))),
        #[cfg(feature = "gemini")]
        "gemini" => Ok(Box::new(gemini::Gemini::new(cfg.clone()))),
        other => Err(ProviderError::Malformed(format!(
            "provider kind `{other}` is not compiled into this build (have: {})",
            match crate::compiled_mappers().as_slice() {
                [] => "none".to_string(),
                m => m.join(", "),
            }
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProviderId;

    #[test]
    fn an_uncompiled_kind_says_what_is_available() {
        let mut cfg = ProviderConfig::new(ProviderId::new("p").unwrap(), "telepathy");
        cfg.endpoint = "http://127.0.0.1".into();
        let e = match build(&cfg) {
            Err(e) => e,
            Ok(_) => panic!("an uncompiled kind must not build"),
        };
        assert!(e.to_string().contains("telepathy"), "{e}");
        assert!(e.to_string().contains("not compiled"), "{e}");
    }

    #[cfg(feature = "ollama")]
    #[test]
    fn a_compiled_kind_builds() {
        let mut cfg = ProviderConfig::new(ProviderId::new("local").unwrap(), "ollama");
        cfg.endpoint = "http://127.0.0.1:11434".into();
        let p = build(&cfg).expect("ollama is compiled in");
        assert_eq!(p.id(), ProviderId::new("local").unwrap());
    }
}
