//! The OpenAI mapper (§21.2).
//!
//! `JsonSchema` — strict structured outputs. Strict mode rejects schemas using constructs
//! it does not support, so Appendix C is written conservatively and each mapper translates
//! what its own endpoint will not take (§21.2, responsibility 2). Appendix C is passed
//! through here unchanged, which is **the untested half of this file**: strict mode also
//! requires every key in `properties` to appear in `required`, and Appendix C's `required`
//! lists three of eleven. Gemini's equivalent mismatch was found by a live call and is
//! translated in [`gemini::dialect`]; this one is still a reading of the documentation.
//!
//! [`gemini::dialect`]: super::gemini::dialect
//!
//! | Path | Purpose |
//! |---|---|
//! | `GET /v1/models` | model list; also the reachability probe |
//! | `POST /v1/chat/completions` | completion, streaming or not |
//!
//! **Implemented, but not tested.** No key has been available, so every shape here is
//! asserted against recorded fixtures rather than against the API. The `required` mismatch
//! above is the concrete thing to check first when a key exists; it is the same class of
//! defect that a live Gemini call exposed. The RFC's implementation note applies: verify
//! before relying on this.

use std::time::Duration;

use serde_json::Value;
use smysl_core::error::ProviderError;

use super::auth::{self, Secret};
use super::openai_compat::{self, Dialect};
use crate::config::ProviderConfig;
use crate::http;
use crate::stream::{Emitter, StreamMsg};
use crate::{
    Capabilities, Completion, Probe, Provider, ProviderId, Request, StructuredMode, TokenCount,
    Usage,
};

const DIALECT: Dialect = Dialect {
    structured: StructuredMode::JsonSchema,
    json_schema: true,
};

#[derive(Debug)]
pub struct OpenAi {
    cfg: ProviderConfig,
}

impl OpenAi {
    pub fn new(cfg: ProviderConfig) -> OpenAi {
        OpenAi { cfg }
    }

    fn endpoint(&self) -> &str {
        if self.cfg.endpoint.is_empty() {
            "https://api.openai.com"
        } else {
            self.cfg.endpoint.trim_end_matches('/')
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.endpoint())
    }

    fn key(&self) -> Result<Secret, ProviderError> {
        auth::resolve(&self.cfg)?.ok_or_else(|| {
            ProviderError::Malformed(format!(
                "{}: a hosted provider needs api_key_env or api_key_cmd",
                self.cfg.id
            ))
        })
    }

    fn headers(&self) -> Result<Vec<(&'static str, String)>, ProviderError> {
        Ok(vec![("authorization", auth::bearer(&self.key()?))])
    }

    pub fn body(&self, req: &Request, stream: bool) -> Result<Value, ProviderError> {
        openai_compat::body(req, &self.cfg.model, DIALECT, stream)
    }

    pub fn parse(&self, raw: &str, retries: u32) -> Result<Completion, ProviderError> {
        let enforced = self.cfg.structured == StructuredMode::JsonSchema;
        openai_compat::parse(raw, &self.cfg.model, retries, enforced)
    }

    pub fn status_error(&self, status: u16, body: &str) -> ProviderError {
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if let Some(e) = openai_compat::error_of(&v) {
                return match status {
                    401 | 403 => ProviderError::Unauthorized,
                    // The status decides backpressure, not the envelope: an overloaded
                    // endpoint that explains itself must still arrive as something the
                    // retry layer acts on.
                    s if http::is_backpressure(s) => {
                        ProviderError::RateLimited { retry_after: None }
                    }
                    _ => e,
                };
            }
        }
        http::status_error(status, body, None)
    }

    pub fn parse_models(raw: &str) -> Result<Vec<String>, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("models: {e}")))?;
        let mut out: Vec<String> = v
            .get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

impl Provider for OpenAi {
    fn id(&self) -> ProviderId {
        self.cfg.id.clone()
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            context_window: self.cfg.context_window,
            max_output: self.cfg.max_output,
            structured: self.cfg.structured,
            streaming: true,
            usage_reporting: true,
            offline: self.cfg.is_local(),
        }
    }

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let headers = self.headers()?;
        let body = self.body(req, false)?.to_string();
        let url = self.url("/v1/chat/completions");
        let timeout = Duration::from_secs(self.cfg.timeout_secs);

        let resp = crate::runtime::run(move || {
            http::post_json(
                &url,
                &headers,
                &body,
                timeout,
                std::thread::sleep,
                super::jitter,
            )
        })?;

        if resp.status >= 400 {
            return Err(self.status_error(resp.status, &resp.body));
        }
        self.parse(&resp.body, resp.retries)
    }

    fn stream(
        &self,
        req: &Request,
        tx: std::sync::mpsc::Sender<StreamMsg>,
    ) -> Result<Usage, ProviderError> {
        let headers = self.headers()?;
        let body = self.body(req, true)?.to_string();
        let url = self.url("/v1/chat/completions");
        let timeout = Duration::from_secs(self.cfg.timeout_secs);

        crate::runtime::run(move || {
            let mut emitter = Emitter::new(tx);
            let resp = match http::post_json(
                &url,
                &headers,
                &body,
                timeout,
                std::thread::sleep,
                super::jitter,
            ) {
                Ok(r) => r,
                Err(e) => {
                    emitter.fail(e.clone());
                    return Err(e);
                }
            };
            let mut usage = Usage {
                estimated: true,
                retries: resp.retries,
                ..Usage::default()
            };
            for line in resp.body.lines() {
                if let Some(u) = openai_compat::sse_usage(line, resp.retries) {
                    usage = u;
                }
                if let Some(t) = openai_compat::sse_delta(line) {
                    if !emitter.token(&t) {
                        break;
                    }
                }
            }
            if usage.estimated {
                usage.output_tokens = (emitter.output_chars() as u64).div_ceil(4);
            }
            emitter.done(usage);
            Ok(usage)
        })
    }

    fn count_tokens(&self, text: &str) -> TokenCount {
        TokenCount::Estimated(smysl_core::tokens(text) as u64)
    }

    fn probe(&self) -> Result<Probe, ProviderError> {
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return Ok(Probe::unreachable(e.to_string())),
        };
        let url = self.url("/v1/models");
        let resp = match crate::runtime::run(move || {
            http::get_with(&url, &headers, Duration::from_secs(10))
        }) {
            Ok(r) => r,
            Err(ProviderError::Unreachable) => {
                return Ok(Probe::unreachable(format!(
                    "cannot reach {}",
                    self.endpoint()
                )))
            }
            Err(e) => return Err(e),
        };

        if resp.status == 401 || resp.status == 403 {
            return Ok(Probe::unreachable("credentials rejected".to_string()));
        }
        if resp.status >= 400 {
            return Err(self.status_error(resp.status, &resp.body));
        }

        let models = OpenAi::parse_models(&resp.body)?;
        let known = models.contains(&self.cfg.model);
        Ok(Probe {
            detail: match known {
                true => format!("{} model(s); {} available", models.len(), self.cfg.model),
                false => format!(
                    "{} model(s); {} is NOT among them",
                    models.len(),
                    self.cfg.model
                ),
            },
            reachable: true,
            models,
            caps: Some(self.caps()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ProviderConfig {
        let mut c = ProviderConfig::new(ProviderId::new("openai").unwrap(), "openai");
        c.model = "gpt-5".into();
        c.context_window = 128_000;
        c.max_output = 8192;
        c.structured = StructuredMode::JsonSchema;
        c.api_key_env = Some("SMYSL_TEST_OPENAI_KEY".into());
        c
    }

    fn provider() -> OpenAi {
        OpenAi::new(cfg())
    }

    #[test]
    fn strict_structured_output_is_the_mechanism() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::JsonSchema, r#"{"type":"object"}"#);
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn enforcement_is_reported_when_the_configuration_asks_for_a_schema() {
        let c = provider()
            .parse(
                r#"{"choices":[{"message":{"content":"{}"},"finish_reason":"stop"}]}"#,
                0,
            )
            .unwrap();
        assert!(c.structured);
    }

    #[test]
    fn the_default_endpoint_is_the_public_api() {
        assert_eq!(
            provider().url("/v1/models"),
            "https://api.openai.com/v1/models"
        );
    }

    #[test]
    fn an_endpoint_override_is_honoured_for_a_compatible_gateway() {
        let mut c = cfg();
        c.endpoint = "https://gateway.example.com/openai/".into();
        assert_eq!(
            OpenAi::new(c).url("/v1/models"),
            "https://gateway.example.com/openai/v1/models"
        );
    }

    #[test]
    fn a_401_is_unauthorized() {
        assert_eq!(
            provider().status_error(401, r#"{"error":{"message":"bad key"}}"#),
            ProviderError::Unauthorized
        );
    }

    #[test]
    fn models_parse_from_the_documented_shape() {
        let raw = r#"{"object":"list","data":[{"id":"gpt-5"},{"id":"gpt-4.1"}]}"#;
        assert_eq!(OpenAi::parse_models(raw).unwrap(), vec!["gpt-4.1", "gpt-5"]);
    }

    #[test]
    fn a_provider_with_no_credential_probes_as_unreachable() {
        let mut c = cfg();
        c.api_key_env = None;
        assert!(!OpenAi::new(c).probe().unwrap().reachable);
    }

    #[test]
    fn a_hosted_provider_is_never_offline_capable() {
        assert!(!provider().caps().offline);
    }
}
