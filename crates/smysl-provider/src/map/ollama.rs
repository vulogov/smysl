//! The Ollama mapper (§21.2) - the CI conformance reference.
//!
//! The only provider exercisable without keys, cost, or egress: loopback HTTP, no TLS, no
//! auth. That is what makes it the reference, and why `local` is the default feature.
//!
//! Endpoints, verified against a running server:
//!
//! | Path | Purpose |
//! |---|---|
//! | `GET /api/tags` | installed models; also the reachability probe |
//! | `POST /api/show` | one model's capabilities and `context_length` |
//! | `POST /api/chat` | completion, streaming or not |
//!
//! Usage reporting is partial: `prompt_eval_count` and `eval_count` are present on a
//! completed response but absent when the server stops early, so the mapper marks the count
//! estimated rather than reporting a zero as if it were a measurement.

use std::time::Duration;

use serde_json::{json, Value};
use smysl_core::error::ProviderError;

use super::StatusMapping;

use crate::config::ProviderConfig;
use crate::http;
use crate::stream::{Emitter, StreamMsg};
use crate::{
    Capabilities, Completion, Probe, Provider, ProviderId, Request, StructuredMode, TokenCount,
    Usage,
};

pub struct Ollama {
    cfg: ProviderConfig,
}

impl Ollama {
    pub fn new(cfg: ProviderConfig) -> Ollama {
        Ollama { cfg }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.cfg.endpoint.trim_end_matches('/'))
    }

    fn timeout(&self) -> Duration {
        Duration::from_secs(self.cfg.timeout_secs)
    }

    /// Responsibility 1 and 2: the request body, and the structured mechanism.
    ///
    /// Ollama takes a JSON schema in `format` directly, which is `JsonSchema`. `Grammar` is
    /// accepted where the backend supports GBNF; anything else is refused rather than
    /// silently downgraded, because a caller that asked for enforced structure and got
    /// unenforced structure would parse the result as if it were guaranteed.
    pub fn body(&self, req: &Request, stream: bool) -> Result<Value, ProviderError> {
        let mut messages = Vec::new();
        if !req.system.is_empty() {
            messages.push(json!({"role": "system", "content": req.system}));
        }
        for m in &req.messages {
            messages.push(json!({"role": m.role.as_str(), "content": m.content}));
        }
        if messages.is_empty() {
            return Err(ProviderError::Malformed(
                "a request with no messages".into(),
            ));
        }

        let mut body = json!({
            "model": if req.model.is_empty() { self.cfg.model.clone() } else { req.model.clone() },
            "messages": messages,
            "stream": stream,
            "options": {
                "temperature": req.temperature,
                "num_predict": req.max_output as i64,
            },
        });

        match req.structured {
            StructuredMode::None => {}
            StructuredMode::JsonSchema => {
                let raw = req.schema.as_deref().ok_or_else(|| {
                    ProviderError::Malformed("json-schema requested without a schema".into())
                })?;
                let schema: Value = serde_json::from_str(raw)
                    .map_err(|e| ProviderError::Malformed(format!("schema is not JSON: {e}")))?;
                body["format"] = schema;
            }
            StructuredMode::JsonMode => body["format"] = json!("json"),
            StructuredMode::Grammar => {
                let g = req.schema.as_deref().ok_or_else(|| {
                    ProviderError::Malformed("grammar requested without one".into())
                })?;
                body["options"]["grammar"] = json!(g);
            }
            // Refused rather than downgraded: a caller that asked for enforced structure
            // and silently got unenforced structure would parse the result as guaranteed.
            // `ToolForce` is the case that exists today; the wildcard is for whatever
            // `StructuredMode` grows next, which will also not be Ollama's mechanism.
            _ => return Err(ProviderError::StructuredUnsupported),
        }

        Ok(body)
    }

    /// Responsibilities 3 and 4: the text, and the usage.
    pub fn parse(&self, raw: &str, retries: u32) -> Result<Completion, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;

        if let Some(err) = v.get("error").and_then(Value::as_str) {
            return Err(classify(err));
        }

        let text = v
            .pointer("/message/content")
            .and_then(Value::as_str)
            // `/api/generate` answers in `response`; accepting both costs one line and
            // saves a confusing failure if a caller points this at the other endpoint.
            .or_else(|| v.get("response").and_then(Value::as_str))
            .ok_or_else(|| ProviderError::Malformed("no message content in response".into()))?
            .to_string();

        let input = v.get("prompt_eval_count").and_then(Value::as_u64);
        let output = v.get("eval_count").and_then(Value::as_u64);

        // Partial usage reporting: a missing count is marked estimated rather than
        // reported as a zero that would read like a measurement.
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
            structured: self.cfg.structured.is_enforced(),
        })
    }

    /// Parse `/api/tags` into a model list.
    pub fn parse_tags(raw: &str) -> Result<Vec<String>, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("tags: {e}")))?;
        let mut out: Vec<String> = v
            .get("models")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| {
                        m.get("name")
                            .or_else(|| m.get("model"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// Parse `/api/show` into capabilities.
    ///
    /// The context length is under `model_info` keyed by architecture -
    /// `llama.context_length`, `qwen2.context_length` - so the key is found by suffix
    /// rather than by guessing the architecture. A model that does not report one keeps the
    /// configured value, which is the honest default: the configuration is a claim and the
    /// probe is a measurement, and an absent measurement does not refute the claim.
    pub fn parse_show(&self, raw: &str) -> Result<Capabilities, ProviderError> {
        let v: Value = serde_json::from_str(raw)
            .map_err(|e| ProviderError::Malformed(format!("show: {e}")))?;

        let context_window = v
            .get("model_info")
            .and_then(Value::as_object)
            .and_then(|o| {
                o.iter()
                    .find(|(k, _)| k.ends_with(".context_length"))
                    .and_then(|(_, v)| v.as_u64())
            })
            .map(|n| n as usize)
            .unwrap_or(self.cfg.context_window);

        let reported: Vec<&str> = v
            .get("capabilities")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();

        Ok(Capabilities {
            context_window,
            max_output: self.cfg.max_output,
            // Ollama takes a schema in `format` regardless of what the model advertises,
            // so structured support is a property of the server, not of the tags.
            structured: self.cfg.structured,
            // A server that lists no capabilities at all is an older one, not a mute one,
            // so absence is read as "yes" rather than as a refusal.
            streaming: reported.contains(&"completion") || reported.is_empty(),
            usage_reporting: true,
            offline: self.cfg.is_local(),
        })
    }
}

/// Responsibility 5: map an error string onto the vocabulary.
fn classify(msg: &str) -> ProviderError {
    let m = msg.to_ascii_lowercase();
    if m.contains("not found") || m.contains("try pulling") {
        // A model that is not installed is a configuration error, and one a fallback must
        // not paper over - so it is `Malformed`, not `Unreachable`.
        return ProviderError::Malformed(msg.to_string());
    }
    if m.contains("context") && (m.contains("exceed") || m.contains("too long")) {
        return ProviderError::ContextExceeded {
            limit: 0,
            requested: 0,
        };
    }
    ProviderError::Upstream(200, msg.to_string())
}

impl Provider for Ollama {
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
            // From the endpoint, so `--offline` rests on a fact rather than on a claim the
            // configuration makes about itself.
            offline: self.cfg.is_local(),
        }
    }

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let body = self.body(req, false)?.to_string();
        let url = self.url("/api/chat");
        let timeout = self.timeout();

        // The runtime owns the call (D-12), so nothing outside this crate touches a thread
        // or a socket.
        let resp = crate::runtime::run(move || {
            http::post_json(&url, &[], &body, timeout, std::thread::sleep, super::jitter)
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
        // Ollama streams newline-delimited JSON objects. `ureq` gives a blocking reader,
        // which is exactly what the dedicated thread is for.
        let body = self.body(req, true)?.to_string();
        let url = self.url("/api/chat");
        let timeout = self.timeout();
        let fallback_model = self.cfg.model.clone();

        crate::runtime::run(move || {
            let mut emitter = Emitter::new(tx);
            let resp = match http::post_json(&url, &[], &body, timeout, std::thread::sleep, || 0.5)
            {
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
            for line in resp.body.lines().filter(|l| !l.trim().is_empty()) {
                let Ok(v) = serde_json::from_str::<Value>(line) else {
                    continue;
                };
                if let Some(t) = v.pointer("/message/content").and_then(Value::as_str) {
                    if !t.is_empty() && !emitter.token(t) {
                        // The receiver went away: a cancelled operation, not a failure.
                        break;
                    }
                }
                if v.get("done").and_then(Value::as_bool) == Some(true) {
                    let i = v.get("prompt_eval_count").and_then(Value::as_u64);
                    let o = v.get("eval_count").and_then(Value::as_u64);
                    usage = Usage {
                        input_tokens: i.unwrap_or(0),
                        output_tokens: o.unwrap_or((emitter.output_chars() as u64).div_ceil(4)),
                        estimated: i.is_none() || o.is_none(),
                        retries: resp.retries,
                    };
                }
            }
            let _ = &fallback_model;
            emitter.done(usage);
            Ok(usage)
        })
    }

    fn count_tokens(&self, text: &str) -> TokenCount {
        // Ollama exposes no tokeniser endpoint, so this is the bundled estimator and says
        // so (D-2). Reporting it as exact would make a budget look trustworthy when it is
        // not.
        TokenCount::Estimated(smysl_core::tokens(text) as u64)
    }

    fn probe(&self) -> Result<Probe, ProviderError> {
        let tags_url = self.url("/api/tags");
        let timeout = Duration::from_secs(5);
        let tags = crate::runtime::run(move || http::get_with(&tags_url, &[], timeout));

        let models = match tags {
            Ok(r) if r.status < 400 => Ollama::parse_tags(&r.body)?,
            Ok(r) => return Err(self.status_error(r.status, &r.body)),
            Err(ProviderError::Unreachable) => {
                return Ok(Probe::unreachable(format!(
                    "no server at {}",
                    self.cfg.endpoint
                )))
            }
            Err(e) => return Err(e),
        };

        // A probe asks what a provider *is*. `/api/show` takes a model name and returns
        // metadata; it sends no content to think about.
        let show_url = self.url("/api/show");
        let model = if self.cfg.model.is_empty() {
            models.first().cloned().unwrap_or_default()
        } else {
            self.cfg.model.clone()
        };
        let caps = if model.is_empty() {
            None
        } else {
            let body = json!({ "model": model }).to_string();
            let t = self.timeout();
            match crate::runtime::run(move || {
                http::post_json(&show_url, &[], &body, t, std::thread::sleep, || 0.5)
            }) {
                Ok(r) if r.status < 400 => self.parse_show(&r.body).ok(),
                _ => None,
            }
        };

        let detail = match models.iter().any(|m| m.starts_with(&model)) {
            true => format!("{} model(s); {model} installed", models.len()),
            false if model.is_empty() => "reachable, no models installed".into(),
            false => format!("{} model(s); {model} is NOT installed", models.len()),
        };

        Ok(Probe {
            reachable: true,
            models,
            caps,
            detail,
        })
    }
}

impl StatusMapping for Ollama {
    /// Responsibility 5, for the status path.
    ///
    /// Ollama answers a missing model with 404 *and* an explanatory body. Mapping on the
    /// status alone would call that `Upstream`, which is technically true and useless: a
    /// missing model is a configuration error, and the difference decides whether a caller
    /// goes and pulls the model or files a bug about the server.
    fn status_error(&self, status: u16, body: &str) -> ProviderError {
        // The status decides backpressure before the body gets a say: a loaded Ollama
        // answers 503 while a model loads, and `classify` would read the explanation as a
        // fault that nothing retries.
        if http::is_backpressure(status) {
            return ProviderError::RateLimited { retry_after: None };
        }
        if let Ok(v) = serde_json::from_str::<Value>(body) {
            if let Some(msg) = v.get("error").and_then(Value::as_str) {
                return classify(msg);
            }
        }
        http::status_error(status, body, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    fn cfg() -> ProviderConfig {
        let mut c = ProviderConfig::new(ProviderId::new("ollama").unwrap(), "ollama");
        c.endpoint = "http://127.0.0.1:11434".into();
        c.model = "llama3.2".into();
        c.context_window = 8192;
        c.max_output = 512;
        c.structured = StructuredMode::JsonSchema;
        c
    }

    fn provider() -> Ollama {
        Ollama::new(cfg())
    }

    // -- responsibility 1: the request body ---------------------------------

    #[test]
    fn the_body_carries_the_model_messages_and_options() {
        let req = Request::new("llama3.2", "hello").with_system("be brief");
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["model"], "llama3.2");
        assert_eq!(b["stream"], false);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][1]["content"], "hello");
        assert_eq!(b["options"]["num_predict"], 1024);
        assert_eq!(b["options"]["temperature"], 0.0);
    }

    #[test]
    fn an_empty_model_falls_back_to_the_configured_one() {
        let mut req = Request::new("", "hi");
        req.model = String::new();
        assert_eq!(provider().body(&req, false).unwrap()["model"], "llama3.2");
    }

    #[test]
    fn a_system_prompt_is_omitted_when_empty() {
        let b = provider().body(&Request::new("m", "hi"), false).unwrap();
        assert_eq!(b["messages"].as_array().unwrap().len(), 1);
        assert_eq!(b["messages"][0]["role"], "user");
    }

    #[test]
    fn assistant_turns_survive_into_the_body() {
        let mut req = Request::new("m", "one");
        req.messages.push(Message::assistant("two"));
        req.messages.push(Message::user("three"));
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["messages"][1]["role"], "assistant");
        assert_eq!(b["messages"][2]["content"], "three");
    }

    #[test]
    fn a_request_with_no_messages_is_refused() {
        let mut req = Request::new("m", "x");
        req.messages.clear();
        assert!(provider().body(&req, false).is_err());
    }

    // -- responsibility 2: the structured mechanism --------------------------

    #[test]
    fn a_json_schema_goes_into_the_format_field() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::JsonSchema, r#"{"type":"object"}"#);
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["format"]["type"], "object");
    }

    #[test]
    fn json_mode_sends_the_bare_string() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::JsonMode;
        assert_eq!(provider().body(&req, false).unwrap()["format"], "json");
    }

    #[test]
    fn a_grammar_goes_into_the_options() {
        let req = Request::new("m", "x").with_schema(StructuredMode::Grammar, "root ::= \"ok\"");
        let b = provider().body(&req, false).unwrap();
        assert_eq!(b["options"]["grammar"], "root ::= \"ok\"");
    }

    /// A caller that asked for enforced structure and silently got unenforced structure
    /// would parse the result as if it were guaranteed.
    #[test]
    fn an_unsupported_mechanism_is_refused_rather_than_downgraded() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::ToolForce;
        assert_eq!(
            provider().body(&req, false).unwrap_err(),
            ProviderError::StructuredUnsupported
        );
    }

    #[test]
    fn a_schema_request_without_a_schema_is_refused() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::JsonSchema;
        assert!(provider().body(&req, false).is_err());
    }

    #[test]
    fn a_schema_that_is_not_json_is_refused_before_it_is_sent() {
        let req = Request::new("m", "x").with_schema(StructuredMode::JsonSchema, "{not json");
        assert!(provider().body(&req, false).is_err());
    }

    // -- responsibility 3 and 4: parsing and usage ---------------------------

    /// The exact shape a running server returned, so this test fails if the mapper drifts
    /// from the endpoint rather than from my memory of it.
    const REAL_RESPONSE: &str = r#"{
        "model": "llama3.2",
        "created_at": "2026-07-27T15:19:21.465184Z",
        "message": { "role": "assistant", "content": "ok" },
        "done": true, "done_reason": "stop",
        "total_duration": 16266837875,
        "prompt_eval_count": 32,
        "eval_count": 2
    }"#;

    #[test]
    fn a_real_response_parses_into_text_and_usage() {
        let c = provider().parse(REAL_RESPONSE, 0).unwrap();
        assert_eq!(c.text, "ok");
        assert_eq!(c.model, "llama3.2");
        assert_eq!(c.usage.input_tokens, 32);
        assert_eq!(c.usage.output_tokens, 2);
        assert!(!c.usage.estimated, "the server reported both counts");
        assert!(c.structured, "the configuration enforces a schema");
    }

    /// Usage reporting is partial. A missing count marked estimated is honest; a zero
    /// reported as a measurement is not.
    #[test]
    fn a_missing_count_is_estimated_rather_than_reported_as_zero() {
        let c = provider()
            .parse(r#"{"message":{"content":"hello there"},"done":true}"#, 0)
            .unwrap();
        assert!(c.usage.estimated);
        assert!(c.usage.output_tokens > 0, "estimated from the text");
    }

    #[test]
    fn the_generate_endpoint_shape_also_parses() {
        let c = provider()
            .parse(r#"{"response":"hi","done":true}"#, 0)
            .unwrap();
        assert_eq!(c.text, "hi");
    }

    #[test]
    fn retries_reach_the_usage_and_nowhere_else() {
        let c = provider().parse(REAL_RESPONSE, 2).unwrap();
        assert_eq!(c.usage.retries, 2);
    }

    #[test]
    fn a_response_that_is_not_json_is_malformed() {
        assert!(matches!(
            provider().parse("<html>502</html>", 0),
            Err(ProviderError::Malformed(_))
        ));
    }

    #[test]
    fn a_response_with_no_content_is_malformed() {
        assert!(provider().parse(r#"{"done":true}"#, 0).is_err());
    }

    // -- responsibility 5: error mapping -------------------------------------

    /// A model that is not installed is a configuration error, and one a fallback must not
    /// paper over.
    #[test]
    fn a_missing_model_is_malformed_not_unreachable() {
        let e = provider()
            .parse(
                r#"{"error":"model 'nope' not found, try pulling it first"}"#,
                0,
            )
            .unwrap_err();
        assert!(matches!(e, ProviderError::Malformed(_)), "{e}");
        assert!(!crate::Registry::would_retry(&e));
    }

    /// The shape a running server actually returned for a model it does not have. Mapping
    /// on the status alone would call this `Upstream`, which is true and useless.
    #[test]
    fn a_404_with_an_explanatory_body_is_classified_from_the_body() {
        let e = provider().status_error(404, r#"{"error":"model 'nope' not found"}"#);
        assert!(matches!(e, ProviderError::Malformed(_)), "{e}");
        assert!(!e.is_fallback_eligible(), "a fallback would paper over it");
    }

    /// A loaded server answers 503 while a model loads. `classify` would read the
    /// explanation as a fault, so the status decides before the body gets a say.
    #[test]
    fn a_503_is_backpressure_even_with_an_explanatory_body() {
        let e = provider().status_error(503, r#"{"error":"server busy"}"#);
        assert!(matches!(e, ProviderError::RateLimited { .. }), "{e}");
    }

    #[test]
    fn a_status_with_no_usable_body_falls_back_to_the_status() {
        let e = provider().status_error(500, "<html>bad gateway</html>");
        assert!(matches!(e, ProviderError::Upstream(500, _)), "{e}");
        assert_eq!(
            provider().status_error(401, ""),
            ProviderError::Unauthorized
        );
    }

    #[test]
    fn a_context_error_maps_to_context_exceeded() {
        let e = provider()
            .parse(r#"{"error":"context window exceeded"}"#, 0)
            .unwrap_err();
        assert!(matches!(e, ProviderError::ContextExceeded { .. }), "{e}");
    }

    #[test]
    fn an_unrecognised_error_string_is_upstream() {
        let e = provider()
            .parse(r#"{"error":"something odd"}"#, 0)
            .unwrap_err();
        assert!(matches!(e, ProviderError::Upstream(_, _)), "{e}");
    }

    // -- probe parsing -------------------------------------------------------

    const REAL_TAGS: &str = r#"{"models":[
        {"name":"llama3.2:latest","model":"llama3.2:latest","size":2019393189},
        {"name":"llama3.1:latest","model":"llama3.1:latest","size":4920753328}
    ]}"#;

    #[test]
    fn tags_parse_into_a_sorted_model_list() {
        let m = Ollama::parse_tags(REAL_TAGS).unwrap();
        assert_eq!(m, vec!["llama3.1:latest", "llama3.2:latest"]);
    }

    #[test]
    fn an_empty_tag_list_is_not_an_error() {
        assert!(Ollama::parse_tags(r#"{"models":[]}"#).unwrap().is_empty());
        assert!(Ollama::parse_tags("{}").unwrap().is_empty());
    }

    /// The context length is keyed by architecture, so it is found by suffix rather than
    /// by guessing which architecture the model is.
    #[test]
    fn the_context_window_is_read_from_the_architecture_key() {
        let raw = r#"{"model_info":{"llama.context_length":131072,"general.architecture":"llama"},
                      "capabilities":["completion","tools"]}"#;
        let c = provider().parse_show(raw).unwrap();
        assert_eq!(c.context_window, 131072);
        assert!(c.streaming);
        assert!(c.offline, "loopback");
    }

    #[test]
    fn a_different_architecture_is_found_by_the_same_rule() {
        let raw = r#"{"model_info":{"qwen2.context_length":32768}}"#;
        assert_eq!(provider().parse_show(raw).unwrap().context_window, 32768);
    }

    /// The configuration is a claim and the probe is a measurement; an absent measurement
    /// does not refute the claim.
    #[test]
    fn a_model_that_reports_no_context_length_keeps_the_configured_one() {
        let c = provider().parse_show("{}").unwrap();
        assert_eq!(c.context_window, 8192);
    }

    #[test]
    fn a_hosted_endpoint_would_not_be_offline_capable() {
        let mut c = cfg();
        c.endpoint = "https://ollama.example.com".into();
        assert!(!Ollama::new(c).caps().offline);
    }

    #[test]
    fn a_loopback_endpoint_is_offline_capable() {
        assert!(provider().caps().offline);
    }

    #[test]
    fn urls_survive_a_trailing_slash() {
        let mut c = cfg();
        c.endpoint = "http://127.0.0.1:11434/".into();
        assert_eq!(
            Ollama::new(c).url("/api/chat"),
            "http://127.0.0.1:11434/api/chat"
        );
    }

    /// Ollama exposes no tokeniser endpoint, so reporting the count as exact would make a
    /// budget look trustworthy when it is not.
    #[test]
    fn token_counts_admit_they_are_estimates() {
        assert!(!provider().count_tokens("hello").is_exact());
    }
}
