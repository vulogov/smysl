//! The DeepSeek mapper (§21.2).
//!
//! `JsonMode` — JSON without schema enforcement. **No structural guarantee**, which is why
//! D-9 leaves this provider's default ingest path to measured E9 rather than assuming one:
//! `json_object` promises the response parses as JSON and nothing about what is in it.
//!
//! Verified against the live endpoint:
//!
//! | Path | Purpose |
//! |---|---|
//! | `GET /models` | model list; also the reachability probe |
//! | `POST /chat/completions` | completion, streaming or not |
//!
//! An OpenAI-compatible shape, so the body and response handling live in
//! [`super::openai_compat`]; what is here is what DeepSeek does differently.

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

/// `json_object` and nothing stronger.
const DIALECT: Dialect = Dialect {
    structured: StructuredMode::JsonMode,
    json_schema: false,
};

#[derive(Debug)]
pub struct DeepSeek {
    cfg: ProviderConfig,
}

impl DeepSeek {
    pub fn new(cfg: ProviderConfig) -> DeepSeek {
        DeepSeek { cfg }
    }

    fn endpoint(&self) -> &str {
        if self.cfg.endpoint.is_empty() {
            "https://api.deepseek.com"
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
        // `structured: false` always: `json_object` enforces that the output parses, not
        // that it conforms. Claiming enforcement here is what would make a caller skip the
        // check that catches the difference.
        openai_compat::parse(raw, &self.cfg.model, retries, false)
    }

    /// Map a status and body onto the vocabulary. The body is the informative half.
    pub fn status_error(&self, status: u16, body: &str) -> ProviderError {
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if let Some(e) = openai_compat::error_of(&v) {
                return match (status, e) {
                    // A 401 is unauthorized whatever the body chose to call it.
                    (401 | 403, _) => ProviderError::Unauthorized,
                    // Likewise the status decides backpressure: DeepSeek answers an
                    // overloaded server with 503 and a body explaining it, and the
                    // explanation must not cost the response its retry.
                    (s, _) if http::is_backpressure(s) => {
                        ProviderError::RateLimited { retry_after: None }
                    }
                    (_, mapped) => mapped,
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

impl Provider for DeepSeek {
    fn id(&self) -> ProviderId {
        self.cfg.id.clone()
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            context_window: self.cfg.context_window,
            max_output: self.cfg.max_output,
            // Whatever the configuration says, this endpoint enforces no schema. Reporting
            // otherwise would let a caller trust a guarantee that does not exist.
            structured: match self.cfg.structured {
                StructuredMode::None => StructuredMode::None,
                _ => StructuredMode::JsonMode,
            },
            streaming: true,
            usage_reporting: true,
            offline: self.cfg.is_local(),
        }
    }

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let headers = self.headers()?;
        let body = self.body(req, false)?.to_string();
        let url = self.url("/chat/completions");
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
        let url = self.url("/chat/completions");
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
        // A hosted provider with no key is a configuration fact, and `providers --probe`
        // exists to report exactly that rather than to fail on it.
        let headers = match self.headers() {
            Ok(h) => h,
            Err(e) => return Ok(Probe::unreachable(e.to_string())),
        };
        let url = self.url("/models");
        let timeout = Duration::from_secs(10);

        let resp = match crate::runtime::run(move || http::get_with(&url, &headers, timeout)) {
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

        let models = DeepSeek::parse_models(&resp.body)?;
        let known = models.contains(&self.cfg.model);
        Ok(Probe {
            reachable: true,
            detail: match (models.len(), known) {
                (n, true) => format!("{n} model(s); {} available", self.cfg.model),
                (n, false) => format!("{n} model(s); {} is NOT among them", self.cfg.model),
            },
            models,
            caps: Some(self.caps()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ProviderConfig {
        let mut c = ProviderConfig::new(ProviderId::new("deepseek").unwrap(), "deepseek");
        c.endpoint = "https://api.deepseek.com".into();
        c.model = "deepseek-chat".into();
        c.context_window = 65536;
        c.max_output = 4096;
        c.structured = StructuredMode::JsonMode;
        c.api_key_env = Some("SMYSL_TEST_DEEPSEEK_KEY".into());
        c
    }

    fn provider() -> DeepSeek {
        DeepSeek::new(cfg())
    }

    /// D-9 rests on this: `json_object` promises the response parses, not what is in it.
    #[test]
    fn json_mode_is_never_reported_as_enforced() {
        assert!(!provider().caps().structured.is_enforced());
        let c = provider()
            .parse(
                r#"{"choices":[{"message":{"content":"{}"},"finish_reason":"stop"}]}"#,
                0,
            )
            .unwrap();
        assert!(!c.structured, "json_object guarantees no structure");
    }

    /// Even a configuration claiming `json-schema` must not make the capability report say
    /// so: a caller would trust a guarantee that does not exist.
    #[test]
    fn a_configuration_cannot_claim_enforcement_this_endpoint_lacks() {
        let mut c = cfg();
        c.structured = StructuredMode::JsonSchema;
        assert_eq!(DeepSeek::new(c).caps().structured, StructuredMode::JsonMode);
    }

    #[test]
    fn a_schema_request_is_refused_rather_than_downgraded() {
        let req = Request::new("m", "x").with_schema(StructuredMode::JsonSchema, "{}");
        assert_eq!(
            provider().body(&req, false).unwrap_err(),
            ProviderError::StructuredUnsupported
        );
    }

    #[test]
    fn json_mode_reaches_the_body() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::JsonMode;
        assert_eq!(
            provider().body(&req, false).unwrap()["response_format"]["type"],
            "json_object"
        );
    }

    #[test]
    fn the_default_endpoint_is_used_when_none_is_configured() {
        let mut c = cfg();
        c.endpoint = String::new();
        assert_eq!(
            DeepSeek::new(c).url("/models"),
            "https://api.deepseek.com/models"
        );
    }

    #[test]
    fn a_hosted_provider_is_never_offline_capable() {
        assert!(!provider().caps().offline);
    }

    /// The live endpoint returns 401 for a bad key, which is the case Ollama cannot
    /// exercise at all.
    #[test]
    fn a_401_is_unauthorized_whatever_the_body_says() {
        assert_eq!(
            provider().status_error(401, r#"{"error":{"message":"Authentication Fails"}}"#),
            ProviderError::Unauthorized
        );
        assert_eq!(
            provider().status_error(403, "not json at all"),
            ProviderError::Unauthorized
        );
    }

    /// This endpoint documents 503 as "server overloaded, retry after a moment", which is
    /// backpressure however the body words it.
    #[test]
    fn a_503_is_backpressure_whatever_the_body_says() {
        let e = provider().status_error(503, r#"{"error":{"message":"Server Overloaded"}}"#);
        assert!(matches!(e, ProviderError::RateLimited { .. }), "{e}");
        assert!(http::is_retryable(&e));
    }

    #[test]
    fn an_error_body_is_classified_even_on_a_200() {
        let e = provider().status_error(400, r#"{"error":{"message":"model x does not exist"}}"#);
        assert!(matches!(e, ProviderError::Malformed(_)), "{e}");
    }

    /// The exact shape the live endpoint returned.
    #[test]
    fn the_live_model_list_parses() {
        let raw = r#"{"object":"list","data":[
            {"id":"deepseek-v4-flash","object":"model","owned_by":"deepseek"},
            {"id":"deepseek-v4-pro","object":"model","owned_by":"deepseek"}]}"#;
        assert_eq!(
            DeepSeek::parse_models(raw).unwrap(),
            vec!["deepseek-v4-flash", "deepseek-v4-pro"]
        );
    }

    #[test]
    fn an_empty_model_list_is_not_an_error() {
        assert!(DeepSeek::parse_models(r#"{"data":[]}"#).unwrap().is_empty());
    }

    /// A hosted provider with no key is a configuration fact, and `providers --probe`
    /// exists to report exactly that.
    #[test]
    fn a_provider_with_no_credential_probes_as_unreachable_not_an_error() {
        let mut c = cfg();
        c.api_key_env = None;
        c.api_key_cmd = None;
        let probe = DeepSeek::new(c).probe().expect("a probe reports");
        assert!(!probe.reachable);
        assert!(probe.detail.contains("api_key_env"), "{}", probe.detail);
    }

    #[test]
    fn an_unset_key_variable_probes_as_unreachable_naming_the_variable() {
        let probe = provider().probe().expect("a probe reports");
        assert!(!probe.reachable);
        assert!(
            probe.detail.contains("SMYSL_TEST_DEEPSEEK_KEY"),
            "{}",
            probe.detail
        );
    }
}
