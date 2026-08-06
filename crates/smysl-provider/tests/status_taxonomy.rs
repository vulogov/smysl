//! What every mapper must make of an HTTP failure.
//!
//! Five mappers expose `status_error(u16, &str) -> ProviderError` with the same signature and
//! the same shape, and none of them had a test for it. Mutation testing in 0.12 put 25
//! survivors on this cluster alone: `delete match arm 401 | 403`, `replace match guard
//! is_backpressure(s) with false`, `replace >= with <` on the `status >= 400` boundary. Every
//! one of them changes what a failure *means* while leaving the success path untouched.
//!
//! This is the gap that matters most for `READINESS.md` gate 4, and it is not the gap gate 4
//! describes. Gate 4 says OpenAI and Anthropic are unverified for want of a key — but Gemini,
//! DeepSeek and Ollama have all been exercised live, and the survivors are spread evenly
//! across all five. What live testing verified is that a *successful* call works. Nobody
//! provokes a 401 against a real endpoint, so the failure taxonomy went unexercised on the
//! verified providers too, and a key would not have found it.
//!
//! The taxonomy is not decoration. `Unauthorized` stops the run; `RateLimited` is retried with
//! backoff; `Upstream` may fall through to another provider. Misclassify a 429 as a fault and
//! a transient overload becomes a failed pipeline; misclassify a 401 as backpressure and the
//! CLI retries a credential that will never work, three times, with jitter.

use smysl_core::error::ProviderError;
use smysl_provider::config::ProviderConfig;
use smysl_provider::map::StatusMapping;
use smysl_provider::{ProviderId, StructuredMode};

/// Every mapper, behind its feature, as a trait object.
///
/// This was a `Box<dyn Fn(u16, &str) -> ProviderError>` until 0.13, because `status_error` was
/// an inherent method on each mapper and there was no trait to name. The closures were the
/// visible cost of a contract shared by convention; `StatusMapping` is that contract written
/// down, so the compiler now checks the shape a sixth mapper must have.
type Classify = Box<dyn StatusMapping>;

fn cfg(id: &str) -> ProviderConfig {
    let mut c = ProviderConfig::new(ProviderId::new(id).unwrap(), id);
    c.model = "a-model".into();
    c.endpoint = "http://127.0.0.1:1".into();
    c.structured = StructuredMode::JsonMode;
    c
}

#[allow(clippy::vec_init_then_push)]
fn mappers() -> Vec<(&'static str, Classify)> {
    let mut v: Vec<(&'static str, Classify)> = Vec::new();

    #[cfg(feature = "anthropic")]
    {
        v.push((
            "anthropic",
            Box::new(smysl_provider::map::anthropic::Anthropic::new(cfg(
                "anthropic",
            ))),
        ));
    }
    #[cfg(feature = "openai")]
    {
        v.push((
            "openai",
            Box::new(smysl_provider::map::openai::OpenAi::new(cfg("openai"))),
        ));
    }
    #[cfg(feature = "gemini")]
    {
        v.push((
            "gemini",
            Box::new(smysl_provider::map::gemini::Gemini::new(cfg("gemini"))),
        ));
    }
    #[cfg(feature = "deepseek")]
    {
        v.push((
            "deepseek",
            Box::new(smysl_provider::map::deepseek::DeepSeek::new(cfg(
                "deepseek",
            ))),
        ));
    }
    #[cfg(feature = "ollama")]
    {
        v.push((
            "ollama",
            Box::new(smysl_provider::map::ollama::Ollama::new(cfg("ollama"))),
        ));
    }
    v
}

/// A body each provider's own error parser will recognise, so the *enveloped* path is taken
/// rather than the bare-status fallback. They disagree about the envelope, so this offers
/// every shape at once: whichever a mapper reads, it finds one.
const ENVELOPE: &str = r#"{"error":{"type":"invalid_request_error","message":"no","code":"x",
                            "status":"INVALID_ARGUMENT"}}"#;

/// Guards the rest: no features, no mappers, and every loop below is vacuous.
#[test]
fn there_are_mappers_to_check() {
    assert!(
        !mappers().is_empty(),
        "no mapper features enabled; run with --all-features"
    );
}

#[test]
fn a_401_or_403_is_unauthorized_everywhere() {
    for (name, classify) in mappers() {
        for status in [401u16, 403] {
            assert!(
                matches!(
                    classify.status_error(status, ENVELOPE),
                    ProviderError::Unauthorized
                ),
                "{name}: {status} must be Unauthorized — retrying a credential that will \
                 never work is the failure this classification prevents"
            );
        }
    }
}

#[test]
fn backpressure_is_rate_limited_everywhere() {
    for (name, classify) in mappers() {
        // 429 is the standard one; 503 is an overloaded server, which is worth waiting out
        // rather than reporting as a fault.
        for status in [429u16, 503] {
            assert!(
                matches!(
                    classify.status_error(status, ENVELOPE),
                    ProviderError::RateLimited { .. }
                ),
                "{name}: {status} must be RateLimited, or a transient overload ends the run"
            );
        }
    }
}

/// The control, and the half that makes the two above mean something.
///
/// A `status_error` returning `Unauthorized` for everything satisfies the first test; one
/// returning `RateLimited` for everything satisfies the second. Neither survives having to
/// tell the three classes apart on the same body.
#[test]
fn the_three_classes_are_actually_distinguished() {
    for (name, classify) in mappers() {
        let unauthorized = classify.status_error(401, ENVELOPE);
        let limited = classify.status_error(429, ENVELOPE);
        let other = classify.status_error(400, ENVELOPE);

        assert!(
            !matches!(other, ProviderError::Unauthorized),
            "{name}: a 400 was classified as Unauthorized"
        );
        assert!(
            !matches!(other, ProviderError::RateLimited { .. }),
            "{name}: a 400 was classified as RateLimited"
        );
        assert_ne!(
            std::mem::discriminant(&unauthorized),
            std::mem::discriminant(&other),
            "{name}: 401 and 400 land in the same variant"
        );
        assert_ne!(
            std::mem::discriminant(&limited),
            std::mem::discriminant(&other),
            "{name}: 429 and 400 land in the same variant"
        );
    }
}

/// A server error is neither a credential problem nor, necessarily, backpressure. It must
/// still arrive as something a caller can act on rather than as a parse failure.
#[test]
fn a_500_is_reported_rather_than_swallowed() {
    for (name, classify) in mappers() {
        let e = classify.status_error(500, "not json at all");
        assert!(
            !matches!(e, ProviderError::Unauthorized),
            "{name}: a 500 read as an authentication problem"
        );
        assert!(
            !format!("{e}").is_empty(),
            "{name}: a 500 produced an error that says nothing"
        );
    }
}
