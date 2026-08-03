//! `caps()` must describe what a mapper does, not what its endpoint could do.
//!
//! The `Provider` trait says so in as many words — "what it does, which is not always the same
//! thing" — and two mappers said otherwise for as long as they had existed. Anthropic and
//! Gemini both declared `streaming: true` while implementing no `stream`, so both inherited the
//! trait default, which refuses. A caller that checked the capability before streaming — the
//! only reason a capability struct exists — would have been told yes and then refused.
//!
//! It went unnoticed because nothing in the library reads the field. That is not a defence:
//! `Capabilities` is public API, published since 0.9.0, and a consumer outside this repository
//! has no other way to ask.
//!
//! Found by reading the Anthropic mapper against Anthropic's documentation, with no key —
//! which is the method `READINESS.md` gate 4 recommends, and the second real defect it has
//! turned up without one.

use std::sync::mpsc;

use smysl_core::error::ProviderError;
use smysl_provider::config::ProviderConfig;
use smysl_provider::{Provider, ProviderId, Request};

/// Every mapper the build implements, behind its feature.
///
/// Built by pushing rather than as a literal because each entry is `#[cfg]`-gated: which
/// mappers exist depends on the feature set, and a literal cannot be conditional per element.
#[allow(clippy::vec_init_then_push)]
fn mappers() -> Vec<(&'static str, Box<dyn Provider>)> {
    #[allow(unused_mut)]
    let mut v: Vec<(&'static str, Box<dyn Provider>)> = Vec::new();

    #[cfg(feature = "anthropic")]
    v.push((
        "anthropic",
        Box::new(smysl_provider::map::anthropic::Anthropic::new(cfg(
            "anthropic",
        ))),
    ));
    #[cfg(feature = "gemini")]
    v.push((
        "gemini",
        Box::new(smysl_provider::map::gemini::Gemini::new(cfg("gemini"))),
    ));
    #[cfg(feature = "openai")]
    v.push((
        "openai",
        Box::new(smysl_provider::map::openai::OpenAi::new(cfg("openai"))),
    ));
    #[cfg(feature = "deepseek")]
    v.push((
        "deepseek",
        Box::new(smysl_provider::map::deepseek::DeepSeek::new(cfg(
            "deepseek",
        ))),
    ));
    #[cfg(feature = "ollama")]
    v.push((
        "ollama",
        Box::new(smysl_provider::map::ollama::Ollama::new(cfg("ollama"))),
    ));
    v
}

fn cfg(id: &str) -> ProviderConfig {
    let mut c = ProviderConfig::new(ProviderId::new(id).unwrap(), id);
    c.model = "a-model".into();
    // An address nothing listens on, so a mapper that genuinely tries to stream fails at the
    // transport rather than reaching anybody. A mapper that does not implement streaming
    // refuses before any I/O, which is the difference this test reads.
    c.endpoint = "http://127.0.0.1:1".into();
    c
}

/// Guards the rest: a feature set that built no mappers would make the loop below vacuous.
#[test]
fn there_are_mappers_to_check() {
    assert!(
        !mappers().is_empty(),
        "no mapper features enabled; run with --all-features"
    );
}

#[test]
fn a_mapper_that_declares_streaming_actually_attempts_it() {
    for (name, p) in mappers() {
        if !p.caps().streaming {
            continue;
        }
        let (tx, _rx) = mpsc::channel();
        let err = p
            .stream(&Request::new("a-model", "text"), tx)
            .expect_err("nothing is listening on port 1, so this cannot succeed");

        // The trait default returns `StructuredUnsupported` without touching the network. A
        // mapper that really implements streaming gets as far as the transport and fails
        // there. So this one error value is the signature of "declared but not implemented".
        assert!(
            !matches!(err, ProviderError::StructuredUnsupported),
            "{name} declares streaming: true but inherits the trait default, which refuses. \
             Either implement `stream` or say `streaming: false`."
        );
    }
}

/// The control. Without it the test above passes trivially if every mapper declares `false`,
/// which is the cheap way to make it green and the wrong one.
#[test]
fn at_least_one_mapper_declares_streaming() {
    let n = mappers().iter().filter(|(_, p)| p.caps().streaming).count();
    assert!(
        n > 0,
        "every mapper now declares streaming: false, so the check above tests nothing"
    );
}
