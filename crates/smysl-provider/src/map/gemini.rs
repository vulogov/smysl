//! The Gemini mapper (§21.2).
//!
//! `JsonSchema` — a response schema plus a JSON mime type. The schema dialect is a *subset*
//! of draft 2020-12, which is why the generated schema (Appendix C) is the intersection of
//! what this dialect and OpenAI strict both accept.
//!
//! Three shape differences from every other mapper here, and each one bites a mapper that
//! assumes otherwise:
//!
//! - The model name is in the **path**, not the body.
//! - Messages are `contents` with `parts`, and the assistant role is called `model`.
//! - The system prompt is `system_instruction`, its own object.
//!
//! | Path | Purpose |
//! |---|---|
//! | `POST /v1beta/models/{model}:generateContent` | completion |
//! | `GET /v1beta/models` | model list; also the reachability probe |
//!
//! **Not verified against a live endpoint.** No key was available when this was written, so
//! the shapes are asserted against recorded fixtures. The RFC's implementation note applies:
//! verify before relying on it.

use std::time::Duration;

use serde_json::{json, Value};
use smysl_core::error::ProviderError;

use super::auth::{self, Secret};
use crate::config::ProviderConfig;
use crate::http;
use crate::{
    Capabilities, Completion, Probe, Provider, ProviderId, Request, Role, StructuredMode,
    TokenCount, Usage,
};

#[derive(Debug)]
pub struct Gemini {
    cfg: ProviderConfig,
}

impl Gemini {
    pub fn new(cfg: ProviderConfig) -> Gemini {
        Gemini { cfg }
    }

    fn endpoint(&self) -> &str {
        if self.cfg.endpoint.is_empty() {
            "https://generativelanguage.googleapis.com"
        } else {
            self.cfg.endpoint.trim_end_matches('/')
        }
    }

    fn key(&self) -> Result<Secret, ProviderError> {
        auth::resolve(&self.cfg)?.ok_or_else(|| {
            ProviderError::Malformed(format!(
                "{}: a hosted provider needs api_key_env or api_key_cmd",
                self.cfg.id
            ))
        })
    }

    /// The key travels in a header rather than the query string: a URL reaches proxy logs
    /// and shell history, and a header does not.
    fn headers(&self) -> Result<Vec<(&'static str, String)>, ProviderError> {
        Ok(vec![("x-goog-api-key", self.key()?.expose().to_string())])
    }

    /// The model is part of the path here, not the body.
    pub fn generate_url(&self, model: &str) -> String {
        let m = if model.is_empty() {
            self.cfg.model.as_str()
        } else {
            model
        };
        format!("{}/v1beta/models/{m}:generateContent", self.endpoint())
    }

    pub fn body(&self, req: &Request) -> Result<Value, ProviderError> {
        // `assistant` is called `model` in this dialect; a mapper that sent `assistant`
        // gets a 400 on any multi-turn request.
        let contents: Vec<Value> = req
            .messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::Assistant => "model",
                    _ => "user",
                };
                json!({"role": role, "parts": [{"text": m.content}]})
            })
            .collect();
        if contents.is_empty() {
            return Err(ProviderError::Malformed(
                "a request with no messages".into(),
            ));
        }

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "temperature": req.temperature,
                "maxOutputTokens": req.max_output,
            },
        });
        if !req.system.is_empty() {
            body["system_instruction"] = json!({"parts": [{"text": req.system}]});
        }

        match req.structured {
            StructuredMode::None => {}
            StructuredMode::JsonSchema => {
                let raw = req.schema.as_deref().ok_or_else(|| {
                    ProviderError::Malformed("json-schema requested without a schema".into())
                })?;
                let schema: Value = serde_json::from_str(raw)
                    .map_err(|e| ProviderError::Malformed(format!("schema is not JSON: {e}")))?;
                // Both halves are required: the mime type without the schema yields JSON of
                // any shape, which is `JsonMode` wearing a schema's name.
                body["generationConfig"]["responseMimeType"] = json!("application/json");
                body["generationConfig"]["responseSchema"] = schema;
            }
            StructuredMode::JsonMode => {
                body["generationConfig"]["responseMimeType"] = json!("application/json");
            }
            _ => return Err(ProviderError::StructuredUnsupported),
        }

        Ok(body)
    }

    pub fn parse(&self, raw: &str, retries: u32) -> Result<Completion, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;

        if let Some(e) = error_of(&v) {
            return Err(e);
        }

        // A candidate blocked by a safety filter has no parts at all, which is a refusal
        // rather than an empty answer and must not read as one.
        let reason = v
            .pointer("/candidates/0/finishReason")
            .and_then(Value::as_str)
            .unwrap_or_default();

        let parts = v
            .pointer("/candidates/0/content/parts")
            .and_then(Value::as_array);

        let text = match parts {
            Some(p) => p
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join(""),
            None => String::new(),
        };

        if text.is_empty() {
            return Err(ProviderError::Malformed(match reason {
                "" => "no candidate content in response".to_string(),
                r => format!("no content; finishReason {r}"),
            }));
        }
        if reason == "MAX_TOKENS" {
            return Err(ProviderError::ContextExceeded {
                limit: self.cfg.max_output,
                requested: text.len(),
            });
        }

        let input = v
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_u64);
        let output = v
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_u64);
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
                .get("modelVersion")
                .and_then(Value::as_str)
                .unwrap_or(&self.cfg.model)
                .to_string(),
            usage,
            structured: self.cfg.structured == StructuredMode::JsonSchema,
        })
    }

    pub fn status_error(&self, status: u16, body: &str) -> ProviderError {
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if let Some(e) = error_of(&v) {
                return match status {
                    401 | 403 => ProviderError::Unauthorized,
                    _ => e,
                };
            }
        }
        http::status_error(status, body, None)
    }

    /// Model names come back fully qualified as `models/gemini-…`; the bare name is what a
    /// configuration writes, so the prefix is stripped.
    pub fn parse_models(raw: &str) -> Result<Vec<String>, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("models: {e}")))?;
        let mut out: Vec<String> = v
            .get("models")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("name").and_then(Value::as_str))
                    .map(|n| n.strip_prefix("models/").unwrap_or(n).to_string())
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out.dedup();
        Ok(out)
    }
}

/// Google's error envelope: `{"error":{"code":…,"message":…,"status":…}}`.
fn error_of(v: &Value) -> Option<ProviderError> {
    let e = v.get("error")?;
    let status = e.get("status").and_then(Value::as_str).unwrap_or_default();
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unspecified provider error");
    Some(match status {
        "UNAUTHENTICATED" | "PERMISSION_DENIED" => ProviderError::Unauthorized,
        "RESOURCE_EXHAUSTED" => ProviderError::RateLimited { retry_after: None },
        "NOT_FOUND" => ProviderError::Malformed(msg.to_string()),
        "INVALID_ARGUMENT" if msg.to_ascii_lowercase().contains("token") => {
            ProviderError::ContextExceeded {
                limit: 0,
                requested: 0,
            }
        }
        _ => ProviderError::Upstream(
            e.get("code").and_then(Value::as_u64).unwrap_or(200) as u16,
            msg.to_string(),
        ),
    })
}

impl Provider for Gemini {
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
        let body = self.body(req)?.to_string();
        let url = self.generate_url(&req.model);
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
        let url = format!("{}/v1beta/models", self.endpoint());
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

        let models = Gemini::parse_models(&resp.body)?;
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
    use crate::Message;

    fn cfg() -> ProviderConfig {
        let mut c = ProviderConfig::new(ProviderId::new("gemini").unwrap(), "gemini");
        c.model = "gemini-2.5-pro".into();
        c.context_window = 1_000_000;
        c.max_output = 8192;
        c.structured = StructuredMode::JsonSchema;
        c.api_key_env = Some("SMYSL_TEST_GEMINI_KEY".into());
        c
    }

    fn provider() -> Gemini {
        Gemini::new(cfg())
    }

    /// The model is part of the path here, not the body.
    #[test]
    fn the_model_is_in_the_url() {
        assert_eq!(
            provider().generate_url(""),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-pro:generateContent"
        );
        assert!(provider()
            .generate_url("gemini-flash")
            .contains("gemini-flash:generateContent"));
        assert!(provider()
            .body(&Request::new("m", "x"))
            .unwrap()
            .get("model")
            .is_none());
    }

    /// `assistant` is called `model` here; a mapper that sent `assistant` gets a 400 on
    /// any multi-turn request.
    #[test]
    fn the_assistant_role_is_called_model() {
        let mut req = Request::new("m", "one");
        req.messages.push(Message::assistant("two"));
        let b = provider().body(&req).unwrap();
        assert_eq!(b["contents"][0]["role"], "user");
        assert_eq!(b["contents"][1]["role"], "model");
        assert_eq!(b["contents"][1]["parts"][0]["text"], "two");
    }

    #[test]
    fn the_system_prompt_is_its_own_object() {
        let req = Request::new("m", "hi").with_system("be brief");
        let b = provider().body(&req).unwrap();
        assert_eq!(b["system_instruction"]["parts"][0]["text"], "be brief");
    }

    /// Both halves are required: the mime type alone yields JSON of any shape, which is
    /// `JsonMode` wearing a schema's name.
    #[test]
    fn a_schema_sets_both_the_mime_type_and_the_response_schema() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::JsonSchema, r#"{"type":"object"}"#);
        let b = provider().body(&req).unwrap();
        assert_eq!(
            b["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert_eq!(b["generationConfig"]["responseSchema"]["type"], "object");
    }

    #[test]
    fn json_mode_sets_only_the_mime_type() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::JsonMode;
        let b = provider().body(&req).unwrap();
        assert_eq!(
            b["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert!(b["generationConfig"].get("responseSchema").is_none());
    }

    #[test]
    fn tool_force_is_not_this_mechanism() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::ToolForce;
        assert!(provider().body(&req).is_err());
    }

    #[test]
    fn generation_limits_use_the_documented_names() {
        let req = Request::new("m", "x").with_max_output(321);
        let b = provider().body(&req).unwrap();
        assert_eq!(b["generationConfig"]["maxOutputTokens"], 321);
        assert_eq!(b["generationConfig"]["temperature"], 0.0);
    }

    #[test]
    fn parts_are_concatenated() {
        let raw = r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[
            {"text":"first "},{"text":"second"}]}}],
            "usageMetadata":{"promptTokenCount":12,"candidatesTokenCount":5},
            "modelVersion":"gemini-2.5-pro-001"}"#;
        let c = provider().parse(raw, 0).unwrap();
        assert_eq!(c.text, "first second");
        assert_eq!(c.model, "gemini-2.5-pro-001");
        assert_eq!(c.usage.input_tokens, 12);
        assert!(!c.usage.estimated);
    }

    /// A candidate blocked by a safety filter has no parts at all, which is a refusal
    /// rather than an empty answer and must not read as one.
    #[test]
    fn a_blocked_candidate_is_an_error_naming_the_reason() {
        let raw = r#"{"candidates":[{"finishReason":"SAFETY"}]}"#;
        let e = provider().parse(raw, 0).unwrap_err();
        assert!(e.to_string().contains("SAFETY"), "{e}");
    }

    #[test]
    fn a_max_tokens_finish_is_a_context_error() {
        let raw = r#"{"candidates":[{"finishReason":"MAX_TOKENS",
                       "content":{"parts":[{"text":"half"}]}}]}"#;
        assert!(matches!(
            provider().parse(raw, 0),
            Err(ProviderError::ContextExceeded { .. })
        ));
    }

    #[test]
    fn google_status_strings_map_onto_the_vocabulary() {
        let e = |status: &str, msg: &str| {
            provider().status_error(
                400,
                &format!(r#"{{"error":{{"status":"{status}","message":"{msg}"}}}}"#),
            )
        };
        assert_eq!(e("UNAUTHENTICATED", "x"), ProviderError::Unauthorized);
        assert_eq!(e("PERMISSION_DENIED", "x"), ProviderError::Unauthorized);
        assert!(matches!(
            e("RESOURCE_EXHAUSTED", "x"),
            ProviderError::RateLimited { .. }
        ));
        assert!(matches!(
            e("NOT_FOUND", "no model"),
            ProviderError::Malformed(_)
        ));
        assert!(matches!(
            e("INVALID_ARGUMENT", "too many token"),
            ProviderError::ContextExceeded { .. }
        ));
    }

    /// Names come back fully qualified; the bare name is what a configuration writes.
    #[test]
    fn model_names_lose_their_prefix() {
        let raw = r#"{"models":[{"name":"models/gemini-2.5-pro"},{"name":"models/gemini-flash"}]}"#;
        assert_eq!(
            Gemini::parse_models(raw).unwrap(),
            vec!["gemini-2.5-pro", "gemini-flash"]
        );
    }

    /// A URL reaches proxy logs and shell history; a header does not.
    #[test]
    fn the_key_travels_in_a_header_not_the_query_string() {
        std::env::set_var("SMYSL_TEST_GEMINI_KEY", "AIza-test");
        let h = provider().headers().unwrap();
        assert_eq!(h[0].0, "x-goog-api-key");
        assert!(!provider().generate_url("").contains("AIza-test"));
        assert!(!provider().generate_url("").contains("key="));
        std::env::remove_var("SMYSL_TEST_GEMINI_KEY");
    }

    #[test]
    fn a_provider_with_no_credential_probes_as_unreachable() {
        let mut c = cfg();
        c.api_key_env = None;
        assert!(!Gemini::new(c).probe().unwrap().reachable);
    }
}
