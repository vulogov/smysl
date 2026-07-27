//! Mappers (§21.2).
//!
//! Each is one file implementing five things: build the request body, apply the structured
//! mechanism for its mode, parse the completion, extract usage, and map errors onto
//! [`ProviderError`]. **No vendor SDK** - five SDKs would be five dependency subtrees for
//! what is, in each case, one POST with a JSON body.
//!
//! The endpoint paths and field names below were verified against a running server, as the
//! RFC's implementation note requires. They change without notice; the mapper contract does
//! not.

#[cfg(feature = "ollama")]
pub mod ollama;

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
