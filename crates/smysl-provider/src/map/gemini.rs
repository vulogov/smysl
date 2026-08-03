//! The Gemini mapper (§21.2).
//!
//! `JsonSchema` — a response schema plus a JSON mime type. The dialect is not a subset of
//! draft 2020-12 but a different thing wearing its vocabulary: an OpenAPI 3.0 `Schema`
//! object, whose fields are a fixed proto message. A keyword outside it is not ignored, it
//! is a 400 naming the field. Appendix C's schema is therefore *translated* here rather
//! than passed through — see [`dialect`].
//!
//! Four shape differences from every other mapper here, and each one bites a mapper that
//! assumes otherwise:
//!
//! - The model name is in the **path**, not the body.
//! - Messages are `contents` with `parts`, and the assistant role is called `model`.
//! - The system prompt is `system_instruction`, its own object.
//! - The response schema is OpenAPI 3.0, not JSON Schema.
//!
//! | Path | Purpose |
//! |---|---|
//! | `POST /v1beta/models/{model}:generateContent` | completion |
//! | `GET /v1beta/models` | model list; also the reachability probe |
//!
//! **Verified against the live endpoint** on 2026-07-27, with `gemini-3.5-flash-lite` and
//! `gemini-3.5-flash`. Two things a caller should know before choosing a model:
//!
//! - `gemini-2.5-flash` and `gemini-2.5-pro` are *listed* by `GET /v1beta/models` and
//!   refused by `generateContent` - "no longer available to new users". A model list is a
//!   catalogue, not an entitlement.
//! - The 3.x models **think**, and the reasoning is spent against `maxOutputTokens` while
//!   being reported apart from the answer. `gemini-3.5-flash` used 1468 thought tokens for a
//!   234-token answer on a three-sentence input, so a cap of 800 can never finish. Size the
//!   cap for both halves, or pick a `-lite` model, which thinks little and answers inside
//!   the same budget.

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
                body["generationConfig"]["responseSchema"] = dialect(&schema);
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

        let input = v
            .pointer("/usageMetadata/promptTokenCount")
            .and_then(Value::as_u64);
        // A thinking model bills its reasoning as output and reports it in a field of its
        // own, so `candidatesTokenCount` alone understates the cost - and the ledger's whole
        // job is not to. The thoughts are also spent against `maxOutputTokens`, which is why
        // a budget sized for the answer alone finishes as `MAX_TOKENS`.
        let thoughts = v
            .pointer("/usageMetadata/thoughtsTokenCount")
            .and_then(Value::as_u64);
        let output = v
            .pointer("/usageMetadata/candidatesTokenCount")
            .and_then(Value::as_u64)
            .map(|o| o + thoughts.unwrap_or(0));

        if text.is_empty() {
            return Err(ProviderError::Malformed(match reason {
                "" => "no candidate content in response".to_string(),
                r => format!("no content; finishReason {r}"),
            }));
        }
        // Both numbers in *tokens*. Reporting `text.len()` here compared bytes against a
        // token budget and printed things like "50 > 800" - a comparison that is false as
        // written and hides the cause, which is that the reasoning spent the budget before
        // the answer began. `gemini-3.5-flash` observed at 1468 thought tokens against 234
        // answer tokens on a three-sentence fixture: a thinking model needs the cap sized
        // for both halves or it can never finish.
        if reason == "MAX_TOKENS" {
            return Err(ProviderError::ContextExceeded {
                limit: self.cfg.max_output,
                requested: output.unwrap_or(smysl_core::tokens(&text) as u64) as usize,
            });
        }

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
                    // The status decides backpressure, not the envelope: a body that says
                    // "high demand" must not arrive as a plain `Upstream` that nothing
                    // retries just because it came with an explanation.
                    s if http::is_backpressure(s) => {
                        ProviderError::RateLimited { retry_after: None }
                    }
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

/// The fields Google's `Schema` message actually has. Everything else is a 400 naming the
/// field, so this is an allow-list rather than a list of things to strip: a keyword added
/// to Appendix C later must be translated deliberately, not discovered in production.
const OPENAPI_FIELDS: &[&str] = &[
    "type",
    "format",
    "title",
    "description",
    "nullable",
    "enum",
    "items",
    "properties",
    "required",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "minProperties",
    "maxProperties",
    "pattern",
    "minimum",
    "maximum",
    "default",
    "anyOf",
    "allOf",
    "propertyOrdering",
];

/// Translate a draft 2020-12 schema into Gemini's OpenAPI 3.0 dialect.
///
/// Three things are lost, and each is enforced elsewhere rather than abandoned:
///
/// - **`$schema`** — a dialect declaration to a thing that is not that dialect.
/// - **`additionalProperties: false`** — the field does not exist here, so a model may
///   invent a key. The converter drops unknown keys, which is where rule X already lives.
/// - **`if`/`then`** — the conditional halves of rules M and T (`measured` implies a
///   source, `derived` implies grounds). `check` decides both after conversion, and a
///   violation spends a repair turn instead of being refused at the endpoint.
///
/// What is gained is a request that is answered at all. A schema this endpoint rejects
/// enforces nothing, because the call never happens.
pub fn dialect(schema: &Value) -> Value {
    let Some(obj) = schema.as_object() else {
        return schema.clone();
    };

    let mut out = serde_json::Map::new();
    for (k, v) in obj {
        if !OPENAPI_FIELDS.contains(&k.as_str()) {
            continue;
        }
        out.insert(
            k.clone(),
            match k.as_str() {
                // Subschema positions, each translated in turn.
                "items" => dialect(v),
                "properties" => match v.as_object() {
                    Some(props) => Value::Object(
                        props
                            .iter()
                            .map(|(name, sub)| (name.clone(), dialect(sub)))
                            .collect(),
                    ),
                    None => v.clone(),
                },
                // A branch that translated to nothing is a branch that constrained nothing;
                // keeping it would send `{}`, which reads as "anything goes" rather than as
                // the omission it is.
                "anyOf" | "allOf" => match v.as_array() {
                    Some(branches) => Value::Array(
                        branches
                            .iter()
                            .map(dialect)
                            .filter(|b| !b.as_object().is_some_and(serde_json::Map::is_empty))
                            .collect(),
                    ),
                    None => v.clone(),
                },
                _ => v.clone(),
            },
        );
    }

    // An empty composition keyword is worse than an absent one: the field exists in the
    // proto, so `"allOf": []` is accepted and constrains nothing.
    out.retain(|k, v| !matches!(k.as_str(), "anyOf" | "allOf") || !is_empty_array(v));

    // Every enum in Appendix C is an enum of strings, and this dialect wants the type said
    // out loud beside it.
    if out.contains_key("enum") && !out.contains_key("type") {
        out.insert("type".into(), json!("string"));
    }

    Value::Object(out)
}

fn is_empty_array(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.is_empty())
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
        // Both are backpressure: the first is the quota saying later, the second is the
        // capacity saying later. Observed live as "this model is currently experiencing
        // high demand", which is not a server fault and is worth waiting out.
        "RESOURCE_EXHAUSTED" | "UNAVAILABLE" => ProviderError::RateLimited { retry_after: None },
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

    /// A thinking model bills its reasoning as output and reports it separately. Observed
    /// live on `gemini-3.5-flash`, whose two-token answer cost eleven tokens of thought.
    #[test]
    fn thinking_tokens_are_counted_as_output() {
        let raw = r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"ok"}]}}],
            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":1,
                             "thoughtsTokenCount":11,"totalTokenCount":15}}"#;
        let c = provider().parse(raw, 0).unwrap();
        assert_eq!(c.usage.output_tokens, 12, "1 answered + 11 thought");
        assert!(!c.usage.estimated);
    }

    /// A model that does not think reports no such field, and must not be charged for one.
    #[test]
    fn an_absent_thought_count_adds_nothing() {
        let raw = r#"{"candidates":[{"finishReason":"STOP","content":{"parts":[{"text":"ok"}]}}],
            "usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":7}}"#;
        assert_eq!(provider().parse(raw, 0).unwrap().usage.output_tokens, 7);
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

    /// Both numbers in tokens, and the thoughts counted among them - otherwise a run that
    /// spent its whole budget reasoning reports bytes against a token cap and prints a
    /// comparison that is false as written.
    #[test]
    fn a_max_tokens_finish_reports_tokens_not_bytes() {
        let raw = r#"{"candidates":[{"finishReason":"MAX_TOKENS",
                       "content":{"parts":[{"text":"a short truncated answer"}]}}],
            "usageMetadata":{"promptTokenCount":222,"candidatesTokenCount":234,
                             "thoughtsTokenCount":1468}}"#;
        // The cap these numbers were observed against: the answer was 234 tokens and would
        // have fitted; the reasoning is what did not.
        let mut c = cfg();
        c.max_output = 800;
        match Gemini::new(c).parse(raw, 0) {
            Err(ProviderError::ContextExceeded { limit, requested }) => {
                assert_eq!(requested, 1702, "234 answered + 1468 thought");
                assert_eq!(limit, 800);
                assert!(requested > limit, "the message must read as true");
            }
            other => panic!("{other:?}"),
        }
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

    /// Observed live: a free-tier key gets this constantly, and it is capacity saying
    /// later rather than a fault. The envelope explains itself, which must not cost the
    /// response its retry.
    #[test]
    fn high_demand_is_backpressure_rather_than_a_fault() {
        let raw = r#"{"error":{"code":503,"status":"UNAVAILABLE",
            "message":"This model is currently experiencing high demand."}}"#;
        let e = provider().status_error(503, raw);
        assert!(matches!(e, ProviderError::RateLimited { .. }), "{e}");
        assert!(http::is_retryable(&e));
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

    // ---- the dialect translation -------------------------------------------------
    //
    // Every keyword below was refused by the live endpoint with a 400 naming the field,
    // on 2026-07-27. These are not guesses about what the dialect might not take.

    /// A dialect declaration, to a thing that is not that dialect.
    #[test]
    fn the_schema_keyword_is_dropped() {
        let s = dialect(&json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object"
        }));
        assert!(s.get("$schema").is_none());
        assert_eq!(s["type"], "object");
    }

    /// The field does not exist in Google's `Schema` message, so sending it is a 400 -
    /// and the closed-world guarantee it carried moves to the converter.
    #[test]
    fn additional_properties_is_dropped_at_every_depth() {
        let s = dialect(&json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "units": {
                    "type": "array",
                    "items": { "type": "object", "additionalProperties": false }
                }
            }
        }));
        assert!(s.get("additionalProperties").is_none());
        assert!(s["properties"]["units"]["items"]
            .get("additionalProperties")
            .is_none());
        assert_eq!(s["properties"]["units"]["items"]["type"], "object");
    }

    /// The conditional halves of rules M and T. `check` decides both after conversion.
    #[test]
    fn conditional_branches_are_dropped_along_with_the_composition_that_held_them() {
        let s = dialect(&json!({
            "type": "object",
            "allOf": [
                { "if": { "properties": { "status": { "enum": ["measured"] } } },
                  "then": { "required": ["source"] } }
            ]
        }));
        assert!(s.get("if").is_none() && s.get("then").is_none());
        // Not `"allOf": []`, which the proto accepts and which constrains nothing while
        // looking like it constrains something.
        assert!(s.get("allOf").is_none(), "an empty composition is not kept");
    }

    /// A branch that survives translation is kept: dropping composition wholesale would
    /// discard constraints this dialect does honour.
    #[test]
    fn a_branch_that_survives_translation_is_kept() {
        let s = dialect(&json!({
            "anyOf": [{ "type": "string" }, { "if": { "type": "number" } }]
        }));
        assert_eq!(s["anyOf"].as_array().unwrap().len(), 1);
        assert_eq!(s["anyOf"][0]["type"], "string");
    }

    /// This dialect wants the type said out loud beside an enum. Every enum in Appendix C
    /// is an enum of strings.
    #[test]
    fn a_bare_enum_gains_its_string_type() {
        let s = dialect(&json!({ "properties": { "status": { "enum": ["cited"] } } }));
        assert_eq!(s["properties"]["status"]["type"], "string");
        assert_eq!(s["properties"]["status"]["enum"][0], "cited");
    }

    /// An allow-list, not a strip-list: a keyword added to Appendix C later must be
    /// translated deliberately rather than discovered as a 400 in production.
    #[test]
    fn an_unrecognised_keyword_is_dropped_rather_than_forwarded() {
        let s = dialect(&json!({ "type": "object", "unevaluatedProperties": false }));
        assert!(s.get("unevaluatedProperties").is_none());
    }

    /// What the dialect does keep, it keeps: a translation that dropped the constraints
    /// would leave `JsonSchema` claiming an enforcement it no longer asks for.
    #[test]
    fn the_constraints_this_dialect_honours_survive() {
        let s = dialect(&json!({
            "type": "object",
            "required": ["gist"],
            "properties": {
                "gist": { "type": "string", "minLength": 1, "maxLength": 240,
                          "pattern": "^[a-z]+$" }
            }
        }));
        assert_eq!(s["required"][0], "gist");
        let gist = &s["properties"]["gist"];
        assert_eq!(gist["minLength"], 1);
        assert_eq!(gist["maxLength"], 240);
        assert_eq!(gist["pattern"], "^[a-z]+$");
    }

    /// The whole point, end to end: Appendix C's own schema, translated, contains nothing
    /// the live endpoint refused.
    #[test]
    fn the_translated_body_carries_no_refused_keyword() {
        let req = Request::new("m", "x").with_schema(
            StructuredMode::JsonSchema,
            r#"{"$schema":"x","type":"object","additionalProperties":false,
                "properties":{"units":{"type":"array","items":{
                    "type":"object","additionalProperties":false,
                    "allOf":[{"if":{"x":1},"then":{"required":["source"]}}]}}}}"#,
        );
        let sent = provider().body(&req).unwrap()["generationConfig"]["responseSchema"].to_string();
        for refused in ["$schema", "additionalProperties", "\"if\"", "\"then\""] {
            assert!(!sent.contains(refused), "`{refused}` survived: {sent}");
        }
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
