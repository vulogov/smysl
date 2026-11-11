//! The Anthropic mapper (§21.2).
//!
//! `ToolForce` — declare a single tool whose input schema is the unit schema, leave the
//! choice to the model, and tell it in the system prompt to answer with that tool and nothing
//! else. The forced choice (`tool_choice: tool`) this mapper was written with is refused by
//! every current-generation model (Opus 5.5, Sonnet 5.5, Haiku 5.5, Fable 5.x: a 400,
//! `tool_choice: type "tool" and "any" are not supported for this model`). What forcing
//! bought was never schema conformance — a tool's input is a suggestion to the model unless
//! the tool is strict — only that a call happened; that is now read from the response, and
//! `Completion::structured` says so, as it always did.
//!
//! Not strict, deliberately. `strict: true` would make a call that happens match the schema,
//! but strict mode takes a subset of JSON Schema in which every object is closed, and the
//! kernel's `payload` is an open object by design: the model writes `ingest:node` into it to
//! say which node it read a claim in, and the text ingest writes its attribution keys there.
//! The endpoint refuses the schema outright (`For 'object' type, 'additionalProperties' must
//! be explicitly set to false`), and closing `payload` would silence the one hint the model
//! gives. So the schema is passed through unchanged, as it was, and conformance is decided by
//! `check` on the way in, where it was always going to be decided.
//!
//! The response is a **block list**, not a string. Text blocks are concatenated for the
//! surface path; the `tool_use` block carries the JSON-AST; `thinking` blocks (always present
//! on these models, empty by default) and `fallback` blocks are neither. A mapper that read
//! only the first block would now read a thinking block.
//!
//! API version `2026-10-21`: no `temperature` (a 400 on every model at that version; the
//! `Request` keeps its field for the other mappers), and thinking is always on, so the
//! `max_tokens` a caller sizes covers the model's reasoning as well as its answer. A policy
//! decline arrives as a successful response with `stop_reason: refusal` and is an error here,
//! never an empty answer; `fallbacks: "default"` asks the API to retry such a decline on a
//! model of its choosing within the same call, so the figure a caller sees is the answer that
//! was given, by the model named in the response.
//!
//! | Path | Purpose |
//! |---|---|
//! | `POST /v1/messages` | completion, streaming or not |
//! | `GET /v1/models` | model list; also the reachability probe |
//!
//! **Verified live** on 1.15 against `api.anthropic.com`: the SM-P14 gate
//! (`crates/smysl-ingest/tests/providers_live.rs`) and `smysl ingest` through the CLI, both
//! staging conformant units. Before that, every shape here was a reading of the documentation
//! asserted against recorded fixtures, and the Gemini mapper had shown what that is worth.

use std::time::Duration;

use serde_json::{json, Value};
use smysl_core::error::ProviderError;

use super::StatusMapping;

use super::auth::{self, Secret};
use crate::config::ProviderConfig;
use crate::http;
use crate::{
    Capabilities, Completion, Probe, Provider, ProviderId, Request, StructuredMode, TokenCount,
    Usage,
};

/// The API version header. Anthropic requires one and dates its changes by it. `2026-10-21`
/// removed the sampling parameters; nothing here sends one.
const API_VERSION: &str = "2026-10-21";

/// The beta that admits `fallbacks` in the body. The scalar `"default"` form needs this
/// value exactly; an earlier `server-side-fallback-*` is a 400 with it.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";

/// What the system prompt says when a schema is requested: the steer that replaces the forced
/// choice. Named here beside the tool it names, so that renaming one cannot leave the other
/// asking for a tool that is not declared.
const STEER: &str = "Answer by calling the `emit_smysl_units` tool exactly once, with every \
                     unit in that one call, and write nothing else.";

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
            ("anthropic-beta", FALLBACK_BETA.to_string()),
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
            "messages": messages,
            // Thinking is always on at this version, and it is spent from the same budget as
            // the answer: what the caller sized is what the model has for both.
            "max_tokens": req.max_output,
            "stream": stream,
            // A policy decline is retried, within this call, on a model the API picks by the
            // decline's category; the response names the model that answered. A decline that
            // survives the chain still comes back as `refusal`, and `parse` refuses it.
            "fallbacks": "default",
        });
        // No `temperature`: `req.temperature` is for the mappers whose endpoints still take
        // one. Here it is a 400 at every value, the default included.

        // The system prompt is a top-level field here, not a message with a role.
        let mut system = req.system.clone();

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
                    // Passed through unchanged: not strict, since strict closes every object
                    // and `payload` is open by design (see the module comment).
                    "input_schema": schema,
                }]);
                // The choice is the model's — forcing it is refused — and the prompt makes
                // the choice; at most one call, since the one call carries every unit.
                body["tool_choice"] = json!({"type": "auto", "disable_parallel_tool_use": true});
                // A schema-shaped extraction is the simple case: the schema does the work,
                // and the reasoning a higher effort buys is spent from the caller's
                // `max_tokens`. Free-text requests take the model's default.
                body["output_config"] = json!({"effort": "low"});
                if !system.is_empty() {
                    system.push_str("\n\n");
                }
                system.push_str(STEER);
            }
            _ => return Err(ProviderError::StructuredUnsupported),
        }
        if !system.is_empty() {
            body["system"] = json!(system);
        }

        Ok(body)
    }

    /// Concatenate text blocks; prefer the tool-use block's input when there is one.
    ///
    /// Reading only the first block would truncate a two-block answer, which is the usual
    /// shape when a model narrates before calling a tool.
    /// Parse a completion, against the cap that was actually sent.
    ///
    /// `cap` is the request's `max_output`, not the configuration's. A mapper sends
    /// `req.max_output` — `Request::new` defaults it to 1024 — and reporting the configured
    /// number instead produced "context window exceeded: 1008 > 32768": true to its fields and
    /// nonsense to a reader, because the two came from different places.
    ///
    /// 0.10 corrected it at the call site, because widening this signature is a breaking change
    /// and the semver baseline was two unpublished releases stale. It is a parameter now, which
    /// is where it belonged: the error is built here, so the number it needs belongs here.
    pub fn parse(&self, raw: &str, retries: u32, cap: usize) -> Result<Completion, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;

        if let Some(e) = error_of(&v) {
            return Err(e);
        }

        // A policy decline is a successful response whose content is empty or partial. Read
        // before the blocks, because an empty block list would otherwise be "empty response"
        // and a partial one an answer. `stop_details` is informational and may be absent.
        if v.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
            let category = v
                .pointer("/stop_details/category")
                .and_then(Value::as_str)
                .unwrap_or("unspecified");
            return Err(ProviderError::Upstream(
                200,
                format!("the provider declined the request (refusal, category: {category})"),
            ));
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
            // `requested` was `text.len()`, bytes against a token cap.
            return Err(ProviderError::Truncated {
                limit: Some(cap),
                used: v
                    .pointer("/usage/output_tokens")
                    .and_then(Value::as_u64)
                    .map(|o| o as usize),
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
            model: self.cfg.model.clone(),
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
        self.parse(&resp.body, resp.retries, req.max_output)
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

impl StatusMapping for Anthropic {
    fn status_error(&self, status: u16, body: &str) -> ProviderError {
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
}

#[cfg(test)]
mod tests {
    /// What Anthropic documents about a tool, checked without a key.
    ///
    /// The mirror of `openai_compat::appendix_c_tests`, and a different shape of check because
    /// the two endpoints are different. OpenAI's strict mode restricts the schema, so what is
    /// checkable there is the *translation*. Anthropic passes `input_schema` through unchanged,
    /// so what is checkable here is the envelope around it — and the envelope has rules that
    /// are easy to break silently.
    ///
    /// Gate 4 in `READINESS.md` is waived at 1.0 because no key has ever been available for
    /// this provider. Everything in this module is what "waived" is allowed to mean: the parts
    /// that do not need one are checked.
    mod tool_envelope {
        use super::*;

        fn schema() -> String {
            include_str!("../../../../fixtures/schema/unit.json").to_string()
        }

        fn body_for(mode: StructuredMode) -> Value {
            let mut cfg = ProviderConfig::new(ProviderId::new("a").unwrap(), "anthropic");
            cfg.model = "claude-x".into();
            cfg.endpoint = "http://127.0.0.1:1".into();
            cfg.structured = mode;
            let m = Anthropic::new(cfg);
            let mut req = Request::new("claude-x", "some text");
            if mode == StructuredMode::ToolForce {
                req = req.with_schema(mode, schema());
            }
            m.body(&req, false).expect("the body builds")
        }

        /// Anthropic requires a tool name matching `^[a-zA-Z0-9_-]{1,64}$`. Ours is a
        /// constant, so this can only fail when somebody edits it — which is exactly when a
        /// silent 400 would be hardest to attribute.
        #[test]
        fn the_tool_name_is_one_anthropic_accepts() {
            assert!(
                !TOOL_NAME.is_empty() && TOOL_NAME.len() <= 64,
                "{TOOL_NAME:?}"
            );
            assert!(
                TOOL_NAME
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
                "`{TOOL_NAME}` is not `^[a-zA-Z0-9_-]{{1,64}}$`"
            );
        }

        /// Nothing is forced, the tool is not strict, and at most one call is admitted.
        ///
        /// `tool_choice: tool` and `any` are a 400 on every current-generation model, which
        /// is the failure that moved this mapper off them: it fails at the endpoint, the one
        /// place nobody here can look, on every call. `strict` is a 400 too, for the open
        /// `payload` (module comment); the first live run against `claude-opus-5-5` found it.
        #[test]
        fn nothing_is_forced_and_at_most_one_call_is_admitted() {
            let body = body_for(StructuredMode::ToolForce);
            assert_eq!(body["tools"].as_array().map(Vec::len), Some(1));
            assert_eq!(body["tools"][0]["name"], TOOL_NAME);
            assert!(
                body["tools"][0].get("strict").is_none(),
                "strict closes every object, and payload is open"
            );
            assert_eq!(body["tool_choice"]["type"], "auto");
            assert_eq!(body["tool_choice"]["disable_parallel_tool_use"], true);
            assert!(
                body["tool_choice"].get("name").is_none(),
                "a named choice is a forced one"
            );
        }

        /// The steer names the tool that was declared, after whatever the caller said.
        ///
        /// This is the defect the envelope makes easy now that nothing forces the call:
        /// rename the tool, forget the prompt, and the model is asked for a tool that is
        /// not in its list — and simply answers in prose, which `structured: false` reports
        /// one call too late to be cheap.
        #[test]
        fn the_steer_names_the_declared_tool_after_the_callers_prompt() {
            let mut cfg = ProviderConfig::new(ProviderId::new("a").unwrap(), "anthropic");
            cfg.model = "claude-x".into();
            cfg.endpoint = "http://127.0.0.1:1".into();
            let m = Anthropic::new(cfg);
            let req = Request::new("claude-x", "some text")
                .with_system("be brief")
                .with_schema(StructuredMode::ToolForce, schema());
            let body = m.body(&req, false).expect("the body builds");
            let system = body["system"].as_str().expect("a system prompt");
            let declared = body["tools"][0]["name"].as_str().unwrap();
            assert!(system.starts_with("be brief\n\n"), "{system:?}");
            assert!(
                system.contains(&format!("`{declared}`")),
                "the steer does not name the declared tool: {system:?}"
            );
            assert_eq!(declared, TOOL_NAME);
        }

        /// No sampling parameter, whatever the request asked for.
        ///
        /// `2026-10-21` rejects `temperature`, `top_p` and `top_k` on every model, the default
        /// value included, so a request that carried the ingest's `temperature: 0` would be a
        /// 400 on every call.
        #[test]
        fn nothing_samples_the_model() {
            for mode in [StructuredMode::None, StructuredMode::ToolForce] {
                let mut cfg = ProviderConfig::new(ProviderId::new("a").unwrap(), "anthropic");
                cfg.model = "claude-x".into();
                cfg.endpoint = "http://127.0.0.1:1".into();
                let m = Anthropic::new(cfg);
                let mut req = Request::new("claude-x", "some text");
                req.temperature = 0.7;
                if mode == StructuredMode::ToolForce {
                    req = req.with_schema(mode, schema());
                }
                let body = m.body(&req, false).expect("the body builds");
                for key in ["temperature", "top_p", "top_k"] {
                    assert!(body.get(key).is_none(), "{mode:?}: sent {key}");
                }
            }
        }

        /// A schema-shaped request runs at low effort; a free-text one takes the default.
        #[test]
        fn a_structured_request_runs_at_low_effort() {
            let body = body_for(StructuredMode::ToolForce);
            assert_eq!(body["output_config"]["effort"], "low");
            let body = body_for(StructuredMode::None);
            assert!(body.get("output_config").is_none(), "effort pinned unasked");
        }

        /// Both modes ask for the fallback chain: a decline is retried in the same call.
        #[test]
        fn fallbacks_are_requested_by_default() {
            for mode in [StructuredMode::None, StructuredMode::ToolForce] {
                assert_eq!(body_for(mode)["fallbacks"], "default", "{mode:?}");
            }
        }

        /// Anthropic requires `input_schema` to be a JSON Schema *object*.
        ///
        /// It is passed through unchanged — there is no translation step here, unlike OpenAI —
        /// so whatever Appendix C says is what the endpoint sees.
        #[test]
        fn the_input_schema_is_an_object_schema() {
            let body = body_for(StructuredMode::ToolForce);
            let s = &body["tools"][0]["input_schema"];
            assert_eq!(s["type"], "object", "input_schema must be an object schema");
            assert!(
                s["properties"].is_object(),
                "an object schema needs properties"
            );
            assert!(
                s.get("required").is_some_and(Value::is_array),
                "the kernel's required fields must survive into the tool schema"
            );
        }

        /// The schema sent is the real one, not a reduction of it.
        ///
        /// This is the failure 0.14 found on the OpenAI side: the strict translation was
        /// tested against an inline copy with 2 of the 13 kernel types, while the code
        /// documented it as the full Appendix C. Here the schema is passed through, so the
        /// equivalent check is that what goes into the body is what `smysl-ingest` generates
        /// — `payload` still an open object, which is what strict mode would have closed.
        #[test]
        fn the_schema_sent_is_the_whole_kernel_schema() {
            let body = body_for(StructuredMode::ToolForce);
            let sent = &body["tools"][0]["input_schema"];
            let fixture: Value = serde_json::from_str(&schema()).expect("the fixture parses");
            assert_eq!(sent, &fixture, "the body altered the schema on its way in");

            let types = sent["properties"]["type"]["enum"]
                .as_array()
                .expect("kernel types");
            assert!(
                types.len() >= 13,
                "only {} kernel types reach the endpoint; the format defines more",
                types.len()
            );
            assert!(
                sent["properties"]["payload"]
                    .get("additionalProperties")
                    .is_none(),
                "payload was closed on its way in"
            );
        }

        /// No tool block at all when structure was not asked for.
        ///
        /// The control: every assertion above is about the shape of a tool request, and all of
        /// them would pass vacuously against a body that always carried one.
        #[test]
        fn no_tool_is_declared_when_none_was_requested() {
            let body = body_for(StructuredMode::None);
            assert!(body.get("tools").is_none(), "declared a tool unasked");
            assert!(body.get("tool_choice").is_none(), "chose a tool unasked");
            assert!(
                body.get("system").is_none(),
                "steered toward a tool unasked"
            );
        }
    }

    use super::*;

    fn cfg() -> ProviderConfig {
        let mut c = ProviderConfig::new(ProviderId::new("anthropic").unwrap(), "anthropic");
        c.model = "claude-opus-5-5".into();
        c.context_window = 1_000_000;
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

    /// The prompt asks for the one tool; nothing forces it, since nothing can.
    #[test]
    fn a_schema_becomes_a_single_tool_the_prompt_asks_for() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::ToolForce, r#"{"type":"object"}"#);
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["tools"].as_array().unwrap().len(), 1);
        assert_eq!(b["tools"][0]["name"], TOOL_NAME);
        assert_eq!(b["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(b["tool_choice"]["type"], "auto");
        assert_eq!(b["system"], STEER);
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
        let raw = r#"{"model":"claude-opus-5-5","stop_reason":"end_turn",
            "content":[{"type":"text","text":"first "},{"type":"text","text":"second"}],
            "usage":{"input_tokens":10,"output_tokens":4}}"#;
        let c = provider().parse(raw, 0, cfg().max_output).unwrap();
        assert_eq!(c.text, "first second");
        assert!(!c.structured, "no tool block, so no enforcement happened");
    }

    #[test]
    fn a_tool_use_block_supplies_the_json_ast() {
        let raw = r#"{"model":"claude-opus-5-5","stop_reason":"tool_use",
            "content":[{"type":"text","text":"here you go"},
                       {"type":"tool_use","name":"emit_smysl_units",
                        "input":{"units":[{"type":"claim","gist":"g","status":"speculative"}]}}],
            "usage":{"input_tokens":10,"output_tokens":20}}"#;
        let c = provider().parse(raw, 0, cfg().max_output).unwrap();
        assert!(c.structured, "the tool was used, so structure was enforced");
        assert!(c.text.contains("\"units\""), "{}", c.text);
        assert!(
            !c.text.contains("here you go"),
            "the narration is not the payload"
        );
    }

    /// Thinking blocks are always present on these models and empty by default; a fallback
    /// block marks a model switch. Neither is the answer, and reading the first block would
    /// now read a thinking block.
    #[test]
    fn thinking_and_fallback_blocks_are_not_the_answer() {
        let raw = r#"{"model":"claude-opus-4-8","stop_reason":"tool_use",
            "content":[{"type":"thinking","thinking":"","signature":"s"},
                       {"type":"fallback","from":{"model":"claude-opus-5-5"},"to":{"model":"claude-opus-4-8"}},
                       {"type":"tool_use","name":"emit_smysl_units","input":{"units":[]}}],
            "usage":{"input_tokens":10,"output_tokens":20}}"#;
        let c = provider().parse(raw, 0, cfg().max_output).unwrap();
        assert!(c.structured);
        assert_eq!(c.text, r#"{"units":[]}"#);
        assert_eq!(
            c.model, "claude-opus-4-8",
            "the model that answered, not the one asked"
        );

        let raw = r#"{"model":"claude-opus-5-5","stop_reason":"end_turn",
            "content":[{"type":"thinking","thinking":"","signature":"s"},
                       {"type":"text","text":"prose"}],
            "usage":{"input_tokens":10,"output_tokens":4}}"#;
        let c = provider().parse(raw, 0, cfg().max_output).unwrap();
        assert_eq!(c.text, "prose");
        assert!(!c.structured);
    }

    /// A policy decline is a 200 with `refusal`, before any output or after some. It is an
    /// error here, read before the blocks: an empty list would otherwise be "empty response"
    /// and a partial one an answer.
    #[test]
    fn a_refusal_is_an_error_before_content_is_read() {
        let empty = r#"{"model":"claude-opus-5-5","stop_reason":"refusal",
            "stop_details":{"type":"refusal","category":"cyber"},
            "content":[],"usage":{"input_tokens":10,"output_tokens":0}}"#;
        let e = provider().parse(empty, 0, cfg().max_output).unwrap_err();
        assert!(matches!(e, ProviderError::Upstream(200, _)), "{e}");
        assert!(e.to_string().contains("cyber"), "{e}");
        assert!(!e.is_fallback_eligible(), "a decline is not an outage");

        let partial = r#"{"model":"claude-opus-5-5","stop_reason":"refusal","stop_details":null,
            "content":[{"type":"text","text":"half an answer"}],
            "usage":{"input_tokens":10,"output_tokens":3}}"#;
        let e = provider().parse(partial, 0, cfg().max_output).unwrap_err();
        assert!(e.to_string().contains("unspecified"), "{e}");
    }

    #[test]
    fn a_max_tokens_stop_is_a_truncation() {
        let raw = r#"{"stop_reason":"max_tokens","content":[{"type":"text","text":"half"}],
                      "usage":{"input_tokens":10,"output_tokens":321}}"#;
        assert_eq!(
            provider().parse(raw, 0, 512).unwrap_err(),
            ProviderError::Truncated {
                limit: Some(512),
                used: Some(321)
            }
        );
    }

    #[test]
    fn usage_comes_from_the_documented_field_names() {
        let raw = r#"{"stop_reason":"end_turn","content":[{"type":"text","text":"x"}],
                      "usage":{"input_tokens":31,"output_tokens":7}}"#;
        let c = provider().parse(raw, 2, cfg().max_output).unwrap();
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
        assert_eq!(h[1], ("anthropic-version", "2026-10-21".to_string()));
        assert_eq!(h[2], ("anthropic-beta", FALLBACK_BETA.to_string()));
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
