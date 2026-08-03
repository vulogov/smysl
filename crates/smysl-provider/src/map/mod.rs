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

#[cfg(feature = "anthropic")]
pub mod anthropic;
#[cfg(feature = "deepseek")]
pub mod deepseek;
#[cfg(feature = "gemini")]
pub mod gemini;
#[cfg(feature = "ollama")]
pub mod ollama;
#[cfg(feature = "openai")]
pub mod openai;
#[cfg(any(feature = "openai", feature = "deepseek"))]
pub mod openai_compat;

/// Full jitter needs a random number and the pure crates have none. The nanosecond of the
/// wall clock is a poor random source and a perfectly good jitter source: it only has to
/// decorrelate clients, not resist anyone.
#[cfg(feature = "http-client")]
/// Restate a `ContextExceeded` against the cap that was actually sent.
///
/// A mapper sends `req.max_output`; `Request::new` defaults it to 1024. A mapper's `parse`
/// has no request and can only quote `cfg.max_output`. Set one and not the other — which a
/// caller does by configuring the provider and not the call — and the error reads
/// "context window exceeded: 1008 > 32768": true to its fields, and nonsense to a reader,
/// because the two numbers come from different places.
///
/// It cost three runs of the quoting experiment to see, and the fields alone gave no hint:
/// the message was self-contradictory and still not obviously a bug in the reporter rather
/// than the request. `parse` stays as it is, because it genuinely does not know; the layer
/// that does know corrects it on the way out.
///
/// Gated on the mappers that call it. Without that, a build with neither feature has an unused
/// function and `-D warnings` refuses it — which is how this arrived in CI green on a
/// `--all-features` machine and red on the `ollama`-only and default-features jobs.
#[cfg(any(feature = "gemini", feature = "anthropic"))]
pub(crate) fn report_against(e: ProviderError, cap: usize) -> ProviderError {
    match e {
        ProviderError::ContextExceeded { requested, .. } => ProviderError::ContextExceeded {
            limit: cap,
            requested,
        },
        other => other,
    }
}

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

#[cfg(all(test, any(feature = "gemini", feature = "anthropic")))]
mod cap_tests {
    use super::*;

    /// The defect this exists for: the number reported must be the number sent.
    #[test]
    fn a_context_error_is_restated_against_the_cap_that_was_sent() {
        let from_parse = ProviderError::ContextExceeded {
            limit: 32_768,   // what the provider was *configured* with
            requested: 1008, // what the answer actually used
        };
        match report_against(from_parse, 1024) {
            ProviderError::ContextExceeded { limit, requested } => {
                assert_eq!(
                    limit, 1024,
                    "the cap the request carried, not the configured one"
                );
                assert_eq!(requested, 1008, "unchanged; only the limit was ever wrong");
            }
            other => panic!("{other:?}"),
        }
    }

    /// What this does **not** fix, recorded rather than implied.
    ///
    /// The first version of the test above asserted `requested >= limit`, on the reasoning
    /// that a truncation message ought to read as true. It does not hold, and the assertion
    /// failed on the very numbers that motivated the fix: 1008 answer tokens against a 1024
    /// cap, truncated. Gemini spends reasoning against the same cap and reports it separately,
    /// so when a response carries `thoughtsTokenCount` the mapper adds it and the comparison
    /// reads true — and when it does not, `requested` falls back to an estimate over the text
    /// and can sit below the limit that stopped it.
    ///
    /// So the invariant is the narrow one: the limit reported is the limit sent. "1008 > 1024"
    /// is still a strange sentence; it is no longer a sentence about the wrong number.
    #[test]
    fn the_reported_pair_is_not_guaranteed_to_read_as_greater() {
        let e = report_against(
            ProviderError::ContextExceeded {
                limit: 0,
                requested: 1008,
            },
            1024,
        );
        match e {
            ProviderError::ContextExceeded { limit, requested } => {
                assert!(
                    requested < limit,
                    "the case that made the stronger claim false"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// The control. A rewrite that returned `ContextExceeded` for everything would satisfy
    /// the test above while destroying every other error a mapper can raise.
    #[test]
    fn every_other_error_passes_through_untouched() {
        for e in [
            ProviderError::Unreachable,
            ProviderError::Unauthorized,
            ProviderError::OfflineViolation,
            ProviderError::StructuredUnsupported,
            ProviderError::Malformed("x".into()),
            ProviderError::Upstream(500, "y".into()),
        ] {
            let before = format!("{e}");
            assert_eq!(format!("{}", report_against(e, 99)), before);
        }
    }
}
