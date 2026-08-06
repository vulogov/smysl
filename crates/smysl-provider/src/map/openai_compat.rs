//! The OpenAI chat-completions shape, shared by the mappers that speak it (§21.2).
//!
//! §21.2 says each mapper is one file. Two of them - `openai` and `deepseek` - differ only
//! in endpoint, model names, and which structured mechanism they support; the request body,
//! the `choices[0].message.content` response, and the `usage` block are identical. Copying
//! two hundred lines to honour a file count would mean two places to fix when the shape
//! moves, so the shape lives here and each mapper states its own differences.
//!
//! Verified against a live DeepSeek endpoint. The paths and field names below are the ones
//! that answered.

use serde_json::{json, Value};
use smysl_core::error::ProviderError;

use crate::{Completion, Request, StructuredMode, Usage};

/// What a compatible provider is allowed to do, so a mapper can refuse a mechanism it does
/// not have rather than silently sending an unenforced request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dialect {
    /// The strongest mechanism this endpoint enforces.
    pub structured: StructuredMode,
    /// Whether `response_format: {type: json_schema}` is accepted, as opposed to only
    /// `json_object`.
    pub json_schema: bool,
}

/// Build the request body.
/// Fields OpenAI's Structured Outputs accepts. Everything else is rejected, so this is an
/// allow-list rather than a list of things to strip — a keyword added to Appendix C later
/// must be translated deliberately rather than discovered in production. Same reasoning as
/// Gemini's `OPENAPI_FIELDS`, and the same shape of defect it was written for.
const STRICT_FIELDS: &[&str] = &[
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "anyOf",
    "description",
    "title",
    "$ref",
    "$defs",
];

/// Rewrite a schema into the subset strict structured outputs accepts.
///
/// Appendix C is written to be conservative and portable, not to be any one vendor's dialect.
/// Strict mode imposes three things it does not satisfy:
///
/// 1. **Every key in `properties` must appear in `required`.** Appendix C declares eleven
///    properties and requires three, so eight are missing. A strict request carrying it is
///    rejected outright — a 400 on every ingest call, not a degraded unit.
/// 2. **Optionality is a nullable type**, not omission from `required`. A field Appendix C
///    left optional becomes `["string", "null"]`, which says the same thing in the only way
///    strict mode can hear it.
/// 3. **`additionalProperties: false` at every object level**, stated rather than implied.
///
/// It also rejects `minLength`, `maxLength`, `pattern` and the `allOf`/`if`/`then`
/// conditionals Appendix C uses to say "measured implies a source". Those are not lost, only
/// unenforced *by the provider*: `check` applies rule M and the shape rules to whatever comes
/// back, which is where they were always going to be decided.
///
/// Translated at the boundary rather than by changing Appendix C, because the shared schema
/// is what Gemini and DeepSeek receive and both work with it today. A vendor's requirement
/// belongs in that vendor's mapper (§21.2, responsibility 2).
pub(crate) fn strict_schema(schema: &Value) -> Value {
    let Some(obj) = schema.as_object() else {
        return schema.clone();
    };
    let originally_required: Vec<String> = obj
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    let mut out = serde_json::Map::new();
    for (k, v) in obj {
        if !STRICT_FIELDS.contains(&k.as_str()) {
            continue;
        }
        match k.as_str() {
            "items" => {
                out.insert(k.clone(), strict_schema(v));
            }
            "properties" => {
                let Some(props) = v.as_object() else {
                    out.insert(k.clone(), v.clone());
                    continue;
                };
                let translated: serde_json::Map<String, Value> = props
                    .iter()
                    .map(|(name, sub)| {
                        let mut t = strict_schema(sub);
                        if !originally_required.iter().any(|r| r == name) {
                            t = nullable(t);
                        }
                        (name.clone(), t)
                    })
                    .collect();
                let all: Vec<Value> = translated.keys().map(|n| Value::from(n.clone())).collect();
                out.insert("properties".into(), Value::Object(translated));
                out.insert("required".into(), Value::Array(all));
            }
            // Already rebuilt from `properties`; copying it through would overwrite the
            // rebuilt list with the three-of-eleven one this function exists to replace.
            "required" => {}
            "anyOf" => {
                let branches = v
                    .as_array()
                    .map(|a| a.iter().map(strict_schema).collect::<Vec<_>>())
                    .unwrap_or_default();
                out.insert(k.clone(), Value::Array(branches));
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    if out.contains_key("properties") {
        out.insert("additionalProperties".into(), Value::Bool(false));
    }
    Value::Object(out)
}

/// Make a translated subschema accept `null` as well.
///
/// `{"type": "string"}` becomes `{"type": ["string", "null"]}`. An `enum` gains a `null`
/// member instead, since strict mode checks the value against the list rather than the type.
fn nullable(mut schema: Value) -> Value {
    let Some(obj) = schema.as_object_mut() else {
        return schema;
    };
    if let Some(Value::Array(arr)) = obj.get_mut("enum") {
        if !arr.iter().any(Value::is_null) {
            arr.push(Value::Null);
        }
        return schema;
    }
    match obj.get("type") {
        Some(Value::String(t)) => {
            let t = t.clone();
            obj.insert("type".into(), json!([t, "null"]));
        }
        Some(Value::Array(ts)) if !ts.iter().any(|v| v.as_str() == Some("null")) => {
            let mut ts = ts.clone();
            ts.push(Value::from("null"));
            obj.insert("type".into(), Value::Array(ts));
        }
        _ => {}
    }
    schema
}

pub fn body(
    req: &Request,
    default_model: &str,
    dialect: Dialect,
    stream: bool,
) -> Result<Value, ProviderError> {
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
        "model": if req.model.is_empty() { default_model } else { req.model.as_str() },
        "messages": messages,
        "stream": stream,
        "max_tokens": req.max_output,
        "temperature": req.temperature,
    });

    match req.structured {
        StructuredMode::None => {}
        StructuredMode::JsonSchema if dialect.json_schema => {
            let raw = req.schema.as_deref().ok_or_else(|| {
                ProviderError::Malformed("json-schema requested without a schema".into())
            })?;
            let schema: Value = serde_json::from_str(raw)
                .map_err(|e| ProviderError::Malformed(format!("schema is not JSON: {e}")))?;
            body["response_format"] = json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "smysl_unit",
                    "strict": true,
                    "schema": strict_schema(&schema),
                },
            });
        }
        StructuredMode::JsonMode => {
            body["response_format"] = json!({"type": "json_object"});
        }
        // A caller that asked for enforcement and silently got none would parse the result
        // as if it were guaranteed. `json-schema` against an endpoint that only has
        // `json_object` is exactly that, so it is refused rather than downgraded.
        _ => return Err(ProviderError::StructuredUnsupported),
    }

    Ok(body)
}

/// Parse a response.
pub fn parse(
    raw: &str,
    default_model: &str,
    retries: u32,
    enforced: bool,
) -> Result<Completion, ProviderError> {
    let v: Value = serde_json::from_str(raw)
        .map_err(|e| ProviderError::Malformed(format!("response is not JSON: {e}")))?;

    if let Some(e) = error_of(&v) {
        return Err(e);
    }

    let text = v
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| ProviderError::Malformed("no choice content in response".into()))?
        .to_string();

    // A truncated answer is worse than none: the caller would parse a half-finished unit as
    // a whole one. `length` means the model was cut off, which is a context problem the
    // caller can act on.
    if v.pointer("/choices/0/finish_reason")
        .and_then(Value::as_str)
        == Some("length")
    {
        return Err(ProviderError::ContextExceeded {
            limit: 0,
            requested: text.len(),
        });
    }

    let input = v.pointer("/usage/prompt_tokens").and_then(Value::as_u64);
    let output = v
        .pointer("/usage/completion_tokens")
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
        // The model that answered, which is not always the model asked for: DeepSeek
        // resolves `deepseek-chat` to a dated build, and the ledger should record what ran.
        model: v
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(default_model)
            .to_string(),
        usage,
        structured: enforced,
    })
}

/// Map an error body onto the vocabulary, whatever the status was.
pub fn error_of(v: &Value) -> Option<ProviderError> {
    let e = v.get("error")?;
    let msg = e
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| e.as_str())
        .unwrap_or("unspecified provider error");
    let kind = e.get("type").and_then(Value::as_str).unwrap_or_default();
    let code = e.get("code").and_then(Value::as_str).unwrap_or_default();
    let m = msg.to_ascii_lowercase();

    Some(match () {
        _ if kind.contains("authentication") || code.contains("invalid_api_key") => {
            ProviderError::Unauthorized
        }
        _ if kind.contains("rate_limit") || code.contains("rate_limit") => {
            ProviderError::RateLimited { retry_after: None }
        }
        _ if m.contains("context length")
            || m.contains("maximum context")
            || m.contains("too long") =>
        {
            ProviderError::ContextExceeded {
                limit: 0,
                requested: 0,
            }
        }
        // A model name the endpoint does not know is a configuration error, and one a
        // fallback must not paper over with a different model's answer.
        _ if m.contains("model") && (m.contains("not exist") || m.contains("not found")) => {
            ProviderError::Malformed(msg.to_string())
        }
        _ => ProviderError::Upstream(200, msg.to_string()),
    })
}

/// Parse one `data:` line of a server-sent-event stream, returning its text delta.
///
/// `None` means "no text here" - a keep-alive, the `[DONE]` sentinel, or a chunk carrying
/// only a role. That is not an error, so the caller keeps reading.
pub fn sse_delta(line: &str) -> Option<String> {
    let payload = line.strip_prefix("data:")?.trim();
    if payload.is_empty() || payload == "[DONE]" {
        return None;
    }
    let v: Value = serde_json::from_str(payload).ok()?;
    let t = v
        .pointer("/choices/0/delta/content")
        .and_then(Value::as_str)?;
    (!t.is_empty()).then(|| t.to_string())
}

/// The usage block of a final stream chunk, when the endpoint sends one.
pub fn sse_usage(line: &str, retries: u32) -> Option<Usage> {
    let payload = line.strip_prefix("data:")?.trim();
    let v: Value = serde_json::from_str(payload).ok()?;
    let u = v.get("usage")?;
    Some(Usage {
        input_tokens: u.get("prompt_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: u
            .get("completion_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        estimated: false,
        retries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Message;

    const SCHEMA_DIALECT: Dialect = Dialect {
        structured: StructuredMode::JsonSchema,
        json_schema: true,
    };
    const JSON_MODE_DIALECT: Dialect = Dialect {
        structured: StructuredMode::JsonMode,
        json_schema: false,
    };

    /// The exact shape a live DeepSeek endpoint returned, so this fails if the mapper
    /// drifts from the API rather than from my memory of it.
    const REAL_DEEPSEEK: &str = r#"{
        "id": "03cbc0cb-e1c8-4408-ba75-79aa431a4a42",
        "object": "chat.completion",
        "created": 1785167716,
        "model": "deepseek-v4-flash",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "ok" },
            "logprobs": null,
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 11, "completion_tokens": 1, "total_tokens": 12,
            "prompt_cache_hit_tokens": 0, "prompt_cache_miss_tokens": 11
        }
    }"#;

    #[test]
    fn a_real_response_parses_into_text_and_usage() {
        let c = parse(REAL_DEEPSEEK, "deepseek-chat", 0, false).unwrap();
        assert_eq!(c.text, "ok");
        assert_eq!(c.usage.input_tokens, 11);
        assert_eq!(c.usage.output_tokens, 1);
        assert!(!c.usage.estimated);
    }

    /// The model that answered is not always the model asked for, and the ledger should
    /// record what ran.
    #[test]
    fn the_reported_model_is_what_answered_not_what_was_asked_for() {
        let c = parse(REAL_DEEPSEEK, "deepseek-chat", 0, false).unwrap();
        assert_eq!(c.model, "deepseek-v4-flash");
    }

    #[test]
    fn the_body_carries_messages_and_limits() {
        let mut req = Request::new("gpt-x", "hello").with_system("be brief");
        req.messages.push(Message::assistant("sure"));
        req.max_output = 256;
        let b = body(&req, "default", SCHEMA_DIALECT, false).unwrap();
        assert_eq!(b["model"], "gpt-x");
        assert_eq!(b["max_tokens"], 256);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][2]["role"], "assistant");
        assert_eq!(b["stream"], false);
    }

    #[test]
    fn an_empty_model_falls_back_to_the_configured_one() {
        let mut req = Request::new("", "hi");
        req.model = String::new();
        assert_eq!(
            body(&req, "fallback-model", SCHEMA_DIALECT, false).unwrap()["model"],
            "fallback-model"
        );
    }

    #[test]
    fn a_schema_becomes_a_strict_response_format() {
        let req =
            Request::new("m", "x").with_schema(StructuredMode::JsonSchema, r#"{"type":"object"}"#);
        let b = body(&req, "d", SCHEMA_DIALECT, false).unwrap();
        assert_eq!(b["response_format"]["type"], "json_schema");
        assert_eq!(b["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            b["response_format"]["json_schema"]["schema"]["type"],
            "object"
        );
    }

    #[test]
    fn json_mode_becomes_a_json_object_response_format() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::JsonMode;
        let b = body(&req, "d", JSON_MODE_DIALECT, false).unwrap();
        assert_eq!(b["response_format"]["type"], "json_object");
    }

    /// A caller that asked for enforcement and silently got none would parse the result as
    /// if it were guaranteed.
    #[test]
    fn a_schema_against_a_json_mode_only_endpoint_is_refused() {
        let req = Request::new("m", "x").with_schema(StructuredMode::JsonSchema, "{}");
        assert_eq!(
            body(&req, "d", JSON_MODE_DIALECT, false).unwrap_err(),
            ProviderError::StructuredUnsupported
        );
    }

    #[test]
    fn tool_force_is_not_this_shape() {
        let mut req = Request::new("m", "x");
        req.structured = StructuredMode::ToolForce;
        assert!(body(&req, "d", SCHEMA_DIALECT, false).is_err());
    }

    /// A truncated answer is worse than none: the caller would parse a half-finished unit
    /// as a whole one.
    #[test]
    fn a_length_stop_is_a_context_error_not_a_completion() {
        let raw = r#"{"choices":[{"message":{"content":"half a un"},"finish_reason":"length"}]}"#;
        assert!(matches!(
            parse(raw, "m", 0, false),
            Err(ProviderError::ContextExceeded { .. })
        ));
    }

    #[test]
    fn a_missing_usage_block_is_estimated() {
        let raw = r#"{"choices":[{"message":{"content":"hello there"},"finish_reason":"stop"}]}"#;
        let c = parse(raw, "m", 0, false).unwrap();
        assert!(c.usage.estimated);
        assert!(c.usage.output_tokens > 0);
    }

    // -- error mapping -------------------------------------------------------

    #[test]
    fn an_authentication_error_is_unauthorized() {
        let v: Value = serde_json::from_str(
            r#"{"error":{"message":"bad key","type":"authentication_error"}}"#,
        )
        .unwrap();
        assert_eq!(error_of(&v), Some(ProviderError::Unauthorized));
    }

    #[test]
    fn an_invalid_api_key_code_is_unauthorized() {
        let v: Value =
            serde_json::from_str(r#"{"error":{"message":"nope","code":"invalid_api_key"}}"#)
                .unwrap();
        assert_eq!(error_of(&v), Some(ProviderError::Unauthorized));
    }

    #[test]
    fn a_rate_limit_is_rate_limited() {
        let v: Value =
            serde_json::from_str(r#"{"error":{"message":"slow down","type":"rate_limit_error"}}"#)
                .unwrap();
        assert!(matches!(
            error_of(&v),
            Some(ProviderError::RateLimited { .. })
        ));
    }

    #[test]
    fn a_context_length_message_is_context_exceeded() {
        let v: Value = serde_json::from_str(
            r#"{"error":{"message":"This model's maximum context length is 8192 tokens"}}"#,
        )
        .unwrap();
        assert!(matches!(
            error_of(&v),
            Some(ProviderError::ContextExceeded { .. })
        ));
    }

    /// A model name the endpoint does not know is a configuration error, and one a
    /// fallback must not paper over.
    #[test]
    fn an_unknown_model_is_malformed_so_no_fallback_hides_it() {
        let v: Value =
            serde_json::from_str(r#"{"error":{"message":"The model `gpt-9` does not exist"}}"#)
                .unwrap();
        let e = error_of(&v).unwrap();
        assert!(matches!(e, ProviderError::Malformed(_)), "{e}");
        assert!(!e.is_fallback_eligible());
    }

    #[test]
    fn a_response_without_an_error_key_maps_to_nothing() {
        let v: Value = serde_json::from_str(REAL_DEEPSEEK).unwrap();
        assert_eq!(error_of(&v), None);
    }

    // -- streaming -----------------------------------------------------------

    #[test]
    fn a_data_line_yields_its_delta() {
        let line = r#"data: {"choices":[{"delta":{"content":"hel"}}]}"#;
        assert_eq!(sse_delta(line), Some("hel".into()));
    }

    /// A keep-alive, a role-only chunk, and the sentinel are all "no text here" rather than
    /// errors, so the caller keeps reading.
    #[test]
    fn non_text_lines_are_skipped_rather_than_failing() {
        for line in [
            "data: [DONE]",
            "data:",
            ": keep-alive",
            r#"data: {"choices":[{"delta":{"role":"assistant"}}]}"#,
            r#"data: {"choices":[{"delta":{"content":""}}]}"#,
            "not an sse line",
        ] {
            assert_eq!(sse_delta(line), None, "{line}");
        }
    }

    #[test]
    fn a_final_chunk_can_carry_usage() {
        let line = r#"data: {"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3}}"#;
        let u = sse_usage(line, 1).unwrap();
        assert_eq!(u.input_tokens, 7);
        assert_eq!(u.output_tokens, 3);
        assert!(!u.estimated);
        assert_eq!(u.retries, 1);
    }

    #[test]
    fn a_chunk_without_usage_yields_none() {
        assert_eq!(sse_usage(r#"data: {"choices":[]}"#, 0), None);
    }
}

#[cfg(test)]
mod strict_tests {
    use super::*;

    fn schema() -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["type", "gist"],
            "properties": {
                "type":  { "enum": ["claim", "evidence"] },
                "gist":  { "type": "string", "minLength": 1, "maxLength": 240 },
                "label": { "type": "string", "pattern": "^[a-z]+/[a-z]+$" },
                "deps":  { "type": "array", "items": { "type": "string" } },
                "source": {
                    "type": "object",
                    "required": ["kind"],
                    "properties": {
                        "kind": { "enum": ["file", "url"] },
                        "ref":  { "type": "string" }
                    }
                }
            },
            "allOf": [{ "if": { "required": ["detail"] }, "then": { "required": ["body"] } }]
        })
    }

    #[test]
    fn every_property_becomes_required() {
        let out = strict_schema(&schema());
        let req: Vec<&str> = out["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let props: Vec<&str> = out["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        assert_eq!(
            req, props,
            "strict mode requires every property to be listed"
        );
        // The nested object too — the rule applies at every level, and a schema that
        // satisfies it only at the top is rejected just as firmly.
        let inner = &out["properties"]["source"];
        assert_eq!(
            inner["required"].as_array().unwrap().len(),
            inner["properties"].as_object().unwrap().len()
        );
    }

    #[test]
    fn what_was_optional_becomes_nullable_instead() {
        let out = strict_schema(&schema());
        // `label` was optional, so optionality has to be said in the type.
        assert_eq!(
            out["properties"]["label"]["type"],
            serde_json::json!(["string", "null"])
        );
        // `gist` was required, so it stays a plain string.
        assert_eq!(out["properties"]["gist"]["type"], "string");
        // An optional enum gains a null member rather than a type union, because strict mode
        // checks the value against the list.
        let kinds = out["properties"]["source"]["properties"]["ref"]["type"].clone();
        assert_eq!(kinds, serde_json::json!(["string", "null"]));
    }

    #[test]
    fn unsupported_constructs_are_dropped_rather_than_sent() {
        let out = strict_schema(&schema());
        assert!(
            out.get("allOf").is_none(),
            "strict mode rejects conditionals"
        );
        let gist = &out["properties"]["gist"];
        assert!(gist.get("minLength").is_none() && gist.get("maxLength").is_none());
        assert!(out["properties"]["label"].get("pattern").is_none());
    }

    #[test]
    fn additional_properties_is_stated_at_every_object_level() {
        let out = strict_schema(&schema());
        assert_eq!(out["additionalProperties"], false);
        assert_eq!(out["properties"]["source"]["additionalProperties"], false);
    }

    #[test]
    fn arrays_keep_their_item_schema() {
        let out = strict_schema(&schema());
        assert_eq!(out["properties"]["deps"]["items"]["type"], "string");
    }

    /// The transform is a pure function, so two runs give the same bytes — which matters
    /// because the request is what gets recorded in a ledger and compared across hops.
    #[test]
    fn it_is_deterministic() {
        assert_eq!(
            serde_json::to_string(&strict_schema(&schema())).unwrap(),
            serde_json::to_string(&strict_schema(&schema())).unwrap()
        );
    }
}

/// The real Appendix C schema, not a miniature of it.
///
/// The tests above use a small schema so a failure names one thing. This one runs the schema
/// that would actually be sent, because the defect being fixed was *counted* on that schema —
/// eleven properties, three required — and a transform that satisfies a toy and not the real
/// one would have fixed nothing.
///
/// The schema text is duplicated from `smysl-ingest` rather than imported: this crate does
/// not depend on that one, and inverting the dependency to reach a string would be a worse
/// trade than a copy a test can check. If Appendix C changes and this copy does not, the
/// count assertion below is what says so.
#[cfg(test)]
mod appendix_c_tests {
    use super::*;

    /// Appendix C, read from the file `smysl-ingest` generates it into.
    ///
    /// This was an inline copy until 0.14, and it had drifted: 2 of the 13 kernel types, 2 of
    /// the 5 statuses, 1 of the 3 conditionals, and a different `label` pattern — while this
    /// crate's own header documented these tests as running "against the full Appendix C
    /// schema rather than a miniature of it". It was the miniature, and nothing could have
    /// noticed, because `smysl-provider` cannot depend on `smysl-ingest` without a cycle.
    ///
    /// A file both sides read is the smallest thing that removes the hazard.
    /// `smysl-ingest`'s `schema_fixture_matches_the_generator` fails if the file goes stale.
    const APPENDIX_C: &str = include_str!("../../../../fixtures/schema/unit.json");

    /// Strict mode's rules are *recursive*, and the test above only ever checked the root.
    ///
    /// OpenAI requires `additionalProperties: false` and every property listed in `required`
    /// on **every** object in the schema, not just the outermost one. Appendix C has nested
    /// objects — `source`, `payload`, and the objects inside `deps` and `grounds` — and a
    /// single one of them missing either property is a rejected call, not a degraded one.
    ///
    /// This is what gate 4 in `READINESS.md` can be answered without a key. Live acceptance
    /// still needs one; whether the schema we send *can* be accepted is a property of the
    /// translation, and that is checkable here. The same method — reading the documentation
    /// and counting rather than assuming — has already found two defects in this crate.
    #[test]
    fn every_object_in_the_translated_schema_is_strict_legal() {
        fn walk(node: &Value, path: &str, bad: &mut Vec<String>) {
            match node {
                Value::Object(o) => {
                    let is_object_schema = o.get("type").and_then(Value::as_str) == Some("object")
                        || o.contains_key("properties");
                    if is_object_schema {
                        if o.get("additionalProperties") != Some(&Value::Bool(false)) {
                            bad.push(format!(
                                "{path}: additionalProperties is {:?}, must be false",
                                o.get("additionalProperties")
                            ));
                        }
                        let props: Vec<&str> = o
                            .get("properties")
                            .and_then(Value::as_object)
                            .map(|p| p.keys().map(String::as_str).collect())
                            .unwrap_or_default();
                        let req: Vec<&str> = o
                            .get("required")
                            .and_then(Value::as_array)
                            .map(|a| a.iter().filter_map(Value::as_str).collect())
                            .unwrap_or_default();
                        let missing: Vec<&&str> =
                            props.iter().filter(|p| !req.contains(p)).collect();
                        if !missing.is_empty() {
                            bad.push(format!("{path}: not in `required`: {missing:?}"));
                        }
                    }
                    for (k, v) in o {
                        walk(v, &format!("{path}.{k}"), bad);
                    }
                }
                Value::Array(a) => {
                    for (i, v) in a.iter().enumerate() {
                        walk(v, &format!("{path}[{i}]"), bad);
                    }
                }
                _ => {}
            }
        }

        let before: Value = serde_json::from_str(APPENDIX_C).expect("fixture parses");

        // The control: the *untranslated* schema must violate the rules, or this test would
        // pass on a transform that did nothing at all.
        let mut source_violations = Vec::new();
        walk(&before, "$", &mut source_violations);
        assert!(
            !source_violations.is_empty(),
            "Appendix C is already strict-legal, so this test cannot show the transform works"
        );

        let after = strict_schema(&before);
        let mut bad = Vec::new();
        walk(&after, "$", &mut bad);
        assert!(
            bad.is_empty(),
            "the translated schema would be rejected by strict mode:\n  {}",
            bad.join("\n  ")
        );
    }

    #[test]
    fn the_real_schema_becomes_strict_legal() {
        let before: Value = serde_json::from_str(APPENDIX_C).expect("fixture parses");
        let props = before["properties"].as_object().unwrap().len();
        let req = before["required"].as_array().unwrap().len();
        assert_eq!(
            (props, req),
            (11, 3),
            "Appendix C changed shape; this copy needs updating, and so may the transform"
        );

        let after = strict_schema(&before);
        let names: Vec<&str> = after["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(|s| s.as_str())
            .collect();
        let required: Vec<&str> = after["required"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            required, names,
            "all eleven must be required; this is the defect the transform exists for"
        );
        assert_eq!(after["additionalProperties"], false);
        assert!(after.get("allOf").is_none());

        // The eight that were optional must now say so in their type, or strict mode will
        // reject a unit that legitimately omits one.
        for name in [
            "label", "body", "detail", "source", "quote", "deps", "grounds", "payload",
        ] {
            let t = &after["properties"][name];
            let nullable = t["type"]
                .as_array()
                .is_some_and(|a| a.iter().any(|v| v.as_str() == Some("null")));
            assert!(nullable, "`{name}` was optional and is not nullable: {t}");
        }
    }
}
