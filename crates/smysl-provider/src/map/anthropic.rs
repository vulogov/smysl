//! The Anthropic mapper (§21.2).
//!
//! `ToolForce` — declare a single tool whose input schema is the unit schema, and force its
//! use. That is how this endpoint is made to produce structured output: there is no
//! response-format field, and asking politely in the prompt is not enforcement.
//!
//! The response is a **block list**, not a string. Text blocks are concatenated for the
//! surface path; the `tool_use` block carries the JSON-AST. A mapper that read only the
//! first block would silently truncate a two-block answer.
//!
//! | Path | Purpose |
//! |---|---|
//! | `POST /v1/messages` | completion, streaming or not |
//! | `GET /v1/models` | model list; also the reachability probe |
//!
//! **Implemented, but not tested.** No key has been available, so every shape here is
//! asserted against recorded fixtures rather than against the API - which means this file is
//! a reading of the documentation, and a reading can be wrong in ways no fixture catches.
//! The Gemini mapper was written the same way and had exactly that failure: its response
//! schema was documented as a subset of draft 2020-12, is not one, and every structured call
//! it made was refused until a live key proved it. The RFC's implementation note applies:
//! verify before relying on this.

use std::time::Duration;

use serde_json::{json, Value};
use smysl_core::error::ProviderError;

use super::auth::{self, Secret};
use crate::config::ProviderConfig;
use crate::http;
use crate::{
    Capabilities, Completion, Probe, Provider, ProviderId, Request, StructuredMode, TokenCount,
    Usage,
};

/// The API version header. Anthropic requires one and dates its changes by it.
const API_VERSION: &str = "2023-06-01";

/// The name of the forced tool. Fixed, because the model never sees it as a choice.
pub const TOOL_NAME: &str = "emit_smysl_units";

#[derive(Debug)]
pub struct Anthropic {
    cfg: ProviderConfig,
}

impl Anthropic {
    pub fn new(cfg: ProviderConfig) -> Anthropic {
        Anthropic { cfg }
    }

    fn endpoint(&self) -> &str {
        if self.cfg.endpoint.is_empty() {
            "https://api.anthropic.com"
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

    /// `x-api-key`, not `Authorization: Bearer` - this endpoint is the odd one out.
    fn headers(&self) -> Result<Vec<(&'static str, String)>, ProviderError> {
        Ok(vec![
            ("x-api-key", self.key()?.expose().to_string()),
            ("anthropic-version", API_VERSION.to_string()),
        ])
    }

    pub fn body(&self, req: &Request, stream: bool) -> Result<Value, ProviderError> {
        let messages: Vec<Value> = req
            .messages
            .iter()
            .map(|m| json!({"role": m.role.as_str(), "content": m.content}))
            .collect();
        if messages.is_empty() {
            return Err(ProviderError::Malformed(
                "a request with no messages".into(),
            ));
        }

        let mut body = json!({
            "model": if req.model.is_empty() { self.cfg.model.as_str() } else { req.model.as_str() },
            // The system prompt is a top-level field here, not a message with a role.
            "messages": messages,
            "max_tokens": req.max_output,
            "temperature": req.temperature,
            "stream": stream,
        });
        if !req.system.is_empty() {
            body["system"] = json!(req.system);
        }

        match req.structured {
            StructuredMode::None => {}
            StructuredMode::ToolForce => {
                let raw = req.schema.as_deref().ok_or_else(|| {
                    ProviderError::Malformed("tool-force requested without a schema".into())
                })?;
                let schema: Value = serde_json::from_str(raw)
                    .map_err(|e| ProviderError::Malformed(format!("schema is not JSON: {e}")))?;
                body["tools"] = json!([{
                    "name": TOOL_NAME,
                    "description": "Emit smysl kernel units for the supplied text.",
                    "input_schema": schema,
                }]);
                // Forcing the tool is what makes this enforcement rather than a suggestion.
                body["tool_choice"] = json!({"type": "tool", "name": TOOL_NAME});
            }
            _ => return Err(ProviderError::StructuredUnsupported),
        }

        Ok(body)
    }

    /// Concatenate text blocks; prefer the tool-use block's input when there is one.
    ///
    /// Reading only the first block would truncate a two-block answer, which is the usual
    /// shape when a model narrates before calling a tool.
    pub fn parse(&self, raw: &str, retries: u32) -> Result<Completion, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;

        if let Some(e) = error_of(&v) {
            return Err(e);
        }

        let blocks = v
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| ProviderError::Malformed("no content blocks in response".into()))?;

        let tool_input = blocks
            .iter()
            .find(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
            .and_then(|b| b.get("input"));

        let text = match tool_input {
            // The JSON-AST path wants the tool input verbatim, re-serialised canonically
            // enough for a parser: what matters is that it is the object the model built.
            Some(input) => input.to_string(),
            None => blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
        };

        if text.is_empty() {
            return Err(ProviderError::Malformed("empty response".into()));
        }

        // `max_tokens` means the answer was cut off, which the caller must not parse as a
        // whole one.
        if v.get("stop_reason").and_then(Value::as_str) == Some("max_tokens") {
            return Err(ProviderError::ContextExceeded {
                limit: self.cfg.max_output,
                requested: text.len(),
            });
        }

        let input = v.pointer("/usage/input_tokens").and_then(Value::as_u64);
        let output = v.pointer("/usage/output_tokens").and_then(Value::as_u64);
        let usage = match (input, output) {
            (Some(i), Some(o)) => Usage {
                input_tokens: i,
                output_tokens: o,
                estimated: false,
                retries,
            },
            _ => Usage {
                input_tokens: input.unwrap_or(0),
                output_tokens: output.unwrap_or(smysl_core::tokens(&text) as u64),
                estimated: true,
                retries,
            },
        };

        Ok(Completion {
            text,
            model: v
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&self.cfg.model)
                .to_string(),
            usage,
            structured: tool_input.is_some(),
        })
    }

    pub fn status_error(&self, status: u16, body: &str) -> ProviderError {
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if let Some(e) = error_of(&v) {
                return match status {
                    401 | 403 => ProviderError::Unauthorized,
                    // The status decides backpressure, not the envelope. Anthropic's 529 is
                    // 503 under a number of its own, and an overloaded server is worth
                    // waiting out rather than reporting as a fault.
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

/// Anthropic's error envelope: `{"type":"error","error":{"type":…,"message":…}}`.
fn error_of(v: &Value) -> Option<ProviderError> {
    let e = v.get("error")?;
    let kind = e.get("type").and_then(Value::as_str).unwrap_or_default();
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unspecified provider error");
    Some(match kind {
        "authentication_error" | "permission_error" => ProviderError::Unauthorized,
        // `overloaded_error` is the 529 envelope: capacity saying later, not a fault.
        "rate_limit_error" | "overloaded_error" => ProviderError::RateLimited { retry_after: None },
        "invalid_request_error" if msg.to_ascii_lowercase().contains("max_tokens") => {
            ProviderError::ContextExceeded {
                limit: 0,
                requested: 0,
            }
        }
        // A model name the endpoint does not know is a configuration error a fallback must
        // not paper over.
        "not_found_error" => ProviderError::Malformed(msg.to_string()),
        _ => ProviderError::Upstream(200, msg.to_string()),
    })
}

impl Provider for Anthropic {
    fn id(&self) -> ProviderId {
        self.cfg.id.clone()
    }

    fn caps(&self) -> Capabilities {
        Capabilities {
            context_window: self.cfg.context_window,
            max_output: self.cfg.max_output,
            // Whatever the configuration says, the mechanism here is a forced tool.
            structured: match self.cfg.structured {
                StructuredMode::None => StructuredMode::None,
                _ => StructuredMode::ToolForce,
            },
            // False, because this mapper does not implement `Provider::stream` and so
            // inherits the trait default, which refuses. The trait's own documentation says
            // `caps` describes what a provider *does*, "which is not always the same thing" —
            // and this said `true` for as long as the mapper has existed. Nothing in the
            // library reads the field yet, which is why nothing noticed; `Capabilities` is
            // public API, so a consumer could.
            streaming: false,
            usage_reporting: true,
            offline: self.cfg.is_local(),
        }
    }

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let headers = self.headers()?;
        let body = self.body(req, false)?.to_string();
        let url = self.url("/v1/messages");
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

        let models = Anthropic::parse_models(&resp.body)?;
        Ok(Probe {
            detail: format!("{} model(s)", models.len()),
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
        let mut c = ProviderConfig::new(ProviderId::new("anthropic").unwrap(), "anthropic");
        c.model = "claude-sonnet-5".into();
        c.context_window = 200_000;
        c.max_output = 8192;
        c.structured = StructuredMode::ToolForce;
        c.api_key_env = Some("SMYSL_TEST_ANTHROPIC_KEY".into());
        c
    }

    fn provider() -> Anthropic {
        Anthropic::new(cfg())
    }

    /// The system prompt is a top-level field here, not a message with a role.
    #[test]
    fn the_system_prompt_is_a_field_not_a_message() {
        let req = Request::new("m", "hello").with_system("be brief");
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["system"], "be brief");
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][0]["role"], "user");
    }

    /// Forcing the tool is what makes this enforcement rather than a suggestion.
    #[test]
    fn a_schema_becomes_a_single_forced_tool() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::ToolForce, r#"{"type":"object"}"#);
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["tools"].as_array().unwrap().len(), 1);
        assert_eq!(b["tools"][0]["name"], TOOL_NAME);
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(b["tool_choice"]["type"], "tool");
        assert_eq!(b["tool_choice"]["name"], TOOL_NAME);
    }

    #[test]
    fn a_json_schema_request_is_refused_because_that_is_not_the_mechanism() {
        let req = Request::new("m", "x").with_schema(StructuredMode::JsonSchema, "{}");
        assert_eq!(
            provider().body(&req, false).unwrap_err(),
            ProviderError::StructuredUnsupported
        );
    }

    /// Reading only the first block would truncate a two-block answer, which is the usual
    /// shape when a model narrates before calling a tool.
    #[test]
    fn text_blocks_are_concatenated_not_truncated() {
        let raw = r#"{"model":"claude-sonnet-5","stop_reason":"end_turn",
            "content":[{"type":"text","text":"first "},{"type":"text","text":"second"}],
            "usage":{"input_tokens":10,"output_tokens":4}}"#;
        let c = provider().parse(raw, 0).unwrap();
        assert_eq!(c.text, "first second");
        assert!(!c.structured, "no tool block, so no enforcement happened");
    }

    #[test]
    fn a_tool_use_block_supplies_the_json_ast() {
        let raw = r#"{"model":"claude-sonnet-5","stop_reason":"tool_use",
            "content":[{"type":"text","text":"here you go"},
                       {"type":"tool_use","name":"emit_smysl_units",
                        "input":{"units":[{"type":"claim","gist":"g","status":"speculative"}]}}],
            "usage":{"input_tokens":10,"output_tokens":20}}"#;
        let c = provider().parse(raw, 0).unwrap();
        assert!(c.structured, "the tool was used, so structure was enforced");
        assert!(c.text.contains("\"units\""), "{}", c.text);
        assert!(
            !c.text.contains("here you go"),
            "the narration is not the payload"
        );
    }

    #[test]
    fn a_max_tokens_stop_is_a_context_error() {
        let raw = r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"half"}]}"#;
        assert!(matches!(
            provider().parse(raw, 0),
            Err(ProviderError::ContextExceeded { .. })
        ));
    }

    #[test]
    fn usage_comes_from_the_documented_field_names() {
        let raw = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"x"}],
                      "usage":{"input_tokens":31,"output_tokens":7}}"#;
        let c = provider().parse(raw, 2).unwrap();
        assert_eq!(c.usage.input_tokens, 31);
        assert_eq!(c.usage.output_tokens, 7);
        assert!(!c.usage.estimated);
        assert_eq!(c.usage.retries, 2);
    }

    #[test]
    fn an_authentication_error_is_unauthorized() {
        let raw = r#"{"type":"error","error":{"type":"authentication_error","message":"bad"}}"#;
        assert_eq!(
            provider().status_error(401, raw),
            ProviderError::Unauthorized
        );
    }

    #[test]
    fn a_not_found_error_is_malformed_so_no_fallback_hides_it() {
        let raw = r#"{"type":"error","error":{"type":"not_found_error","message":"no model"}}"#;
        let e = provider().status_error(404, raw);
        assert!(matches!(e, ProviderError::Malformed(_)), "{e}");
        assert!(!e.is_fallback_eligible());
    }

    #[test]
    fn a_rate_limit_error_is_rate_limited() {
        let raw = r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow"}}"#;
        assert!(matches!(
            provider().status_error(429, raw),
            ProviderError::RateLimited { .. }
        ));
    }

    /// 529 is this endpoint's 503, and `overloaded_error` is its envelope. Both are
    /// capacity saying later, so both reach the retry loop.
    #[test]
    fn overloaded_is_backpressure_by_status_and_by_envelope() {
        let raw = r#"{"type":"error","error":{"type":"overloaded_error","message":"busy"}}"#;
        for (status, body) in [(529, raw), (529, "not json"), (503, raw)] {
            let e = provider().status_error(status, body);
            assert!(
                matches!(e, ProviderError::RateLimited { .. }),
                "{status}: {e}"
            );
            assert!(http::is_retryable(&e), "{status}");
        }
    }

    /// `x-api-key`, not `Authorization: Bearer` - this endpoint is the odd one out, and a
    /// mapper that copied the bearer convention would get 401 for a valid key.
    #[test]
    fn the_credential_header_is_x_api_key() {
        std::env::set_var("SMYSL_TEST_ANTHROPIC_KEY", "sk-ant-test");
        let h = provider().headers().unwrap();
        assert_eq!(h[0].0, "x-api-key");
        assert_eq!(h[0].1, "sk-ant-test");
        assert_eq!(h[1], ("anthropic-version", API_VERSION.to_string()));
        std::env::remove_var("SMYSL_TEST_ANTHROPIC_KEY");
    }

    #[test]
    fn the_capability_report_names_the_real_mechanism() {
        let mut c = cfg();
        c.structured = StructuredMode::JsonSchema;
        assert_eq!(
            Anthropic::new(c).caps().structured,
            StructuredMode::ToolForce
        );
    }

    #[test]
    fn a_provider_with_no_credential_probes_as_unreachable() {
        let mut c = cfg();
        c.api_key_env = None;
        assert!(!Anthropic::new(c).probe().unwrap().reachable);
    }
}
