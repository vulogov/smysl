//! M7-P4's provider (RFC SMYSL-10 A6-8, A6-10): the Claude API, raw HTTP, for the hosted arms.
//!
//! Not smysl's Anthropic mapper. That one is the ingest's: it declares its own tool with an
//! ingest steer and refuses a JSON-schema request, leaves thinking on, and asks for the
//! fallback chain. A6-8 froze something else, and this sends exactly that:
//!
//! - the step schema as structured output (`output_config.format`), each object closed
//!   (`additionalProperties: false`, which the endpoint requires). The schema the loop builds,
//!   and so what Ollama gets, is not changed;
//! - thinking off (`thinking: disabled`) at effort `medium`, as the local model has none;
//! - no sampling parameter (Haiku 5.5 takes none but the default) and no `fallbacks`: the answer
//!   is by the model named or by nobody, and a response naming another model is an error;
//! - the system prompt as a cache breakpoint, and automatic caching for the growing transcript.
//!
//! The HTTP is curl's: the provider crate's client is internal to it, and a dev-dependency on
//! one would add a tree to every build of smysl's tests for one example's hosted mode. A retry
//! on backpressure or a dropped connection is not a step: the loop counts replies. Each
//! request's usage and price go to `requests.jsonl`, and `max_cost` stops the process before a
//! request that would start past it. The key comes from `ANTHROPIC_API_KEY`, is held here only,
//! and is never written anywhere.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::{json, Value};
use smysl::Request;

use crate::agent::{Agent, Reply};

const ENDPOINT: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2026-10-21";
/// Attempts per request, retries included.
const ATTEMPTS: u32 = 8;

/// USD per million tokens: input, output, at a prompt of at most `long_from` tokens, and above
/// it. Cache writes are 1.25 × input (5-minute entries), cache reads 0.1 ×.
struct Price {
    input: f64,
    output: f64,
    long_input: f64,
    long_output: f64,
    long_from: u64,
}

fn price(model: &str) -> Option<Price> {
    match model {
        // A6-13; the skill's table, read 2026-11-27.
        "claude-haiku-5-5" => Some(Price {
            input: 0.10,
            output: 0.50,
            long_input: 0.50,
            long_output: 2.50,
            long_from: 100_000,
        }),
        _ => None,
    }
}

/// One response's usage. `input` is the uncached part, as the API reports it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: u64,
    pub cache_write: u64,
    pub cache_read: u64,
    pub output: u64,
}

impl Usage {
    /// Every token the model read.
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_write + self.cache_read
    }

    fn cost(&self, p: &Price) -> f64 {
        let (i, o) = if self.prompt() > p.long_from {
            (p.long_input, p.long_output)
        } else {
            (p.input, p.output)
        };
        (self.input as f64 * i
            + self.cache_write as f64 * i * 1.25
            + self.cache_read as f64 * i * 0.1
            + self.output as f64 * o)
            / 1e6
    }
}

pub struct Claude {
    model: String,
    effort: String,
    key: String,
    max_cost: f64,
    /// Spent so far, USD, from the responses' usage.
    pub spent: f64,
    /// The largest prompt sent, in the API's tokens (A6-11's check against the context limit).
    pub largest_prompt: u64,
    /// Where curl reads the body and writes the response and its headers.
    work: PathBuf,
    log: std::fs::File,
}

impl Claude {
    pub fn new(model: &str, effort: &str, max_cost: f64, out: &Path) -> Result<Claude, String> {
        price(model).ok_or_else(|| format!("no price for `{model}`: add it before spending"))?;
        let key = std::env::var("ANTHROPIC_API_KEY")
            .map_err(|_| "ANTHROPIC_API_KEY is not set".to_string())?;
        if key.is_empty() || key.contains(['"', '\\', '\n', '\r']) {
            return Err("ANTHROPIC_API_KEY is empty or not a key".into());
        }
        let work = out.join("http");
        std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
        let path = out.join("requests.jsonl");
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Claude {
            model: model.to_string(),
            effort: effort.to_string(),
            key,
            max_cost,
            spent: 0.0,
            largest_prompt: 0,
            work,
            log,
        })
    }

    /// One POST, by curl, with retries on backpressure, server errors and transport failures.
    /// The key and the headers go in curl's configuration on stdin, so no process listing shows
    /// the key.
    fn post(&self, body: &str) -> Result<String, String> {
        let req = self.work.join("request.json");
        let resp = self.work.join("response.json");
        let head = self.work.join("headers.txt");
        std::fs::write(&req, body).map_err(|e| format!("{}: {e}", req.display()))?;
        let config = curl_config(&self.key, &req, &resp, &head);
        let mut last = String::new();
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs((1u64 << attempt.min(6)).min(60)));
            }
            std::fs::remove_file(&resp).ok();
            let mut child = Command::new("curl")
                .args(["--config", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| format!("curl: {e}"))?;
            child
                .stdin
                .take()
                .expect("piped")
                .write_all(config.as_bytes())
                .map_err(|e| format!("curl's stdin: {e}"))?;
            let o = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
            let code: u16 = String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse()
                .unwrap_or(0);
            let text = std::fs::read_to_string(&resp).unwrap_or_default();
            if o.status.success() && code == 200 {
                return Ok(text);
            }
            if !o.status.success() || code == 0 {
                last = format!(
                    "transport: curl {}: {}",
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            } else if matches!(code, 429 | 500 | 502 | 503 | 504 | 529) {
                last = format!("status {code}: {text}");
                let wait = std::fs::read_to_string(&head)
                    .ok()
                    .and_then(|h| retry_after(&h));
                if let Some(s) = wait {
                    std::thread::sleep(Duration::from_secs(s.min(120)));
                }
            } else {
                return Err(format!("status {code}: {text}"));
            }
            eprintln!("  claude: retry {} after {last}", attempt + 1);
        }
        Err(format!("gave up after {ATTEMPTS} attempts: {last}"))
    }
}

/// curl's configuration for one request: everything but the body, which it reads from `req`.
fn curl_config(key: &str, req: &Path, resp: &Path, head: &Path) -> String {
    let q = |p: &Path| {
        p.display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
    };
    format!(
        "url = \"{ENDPOINT}\"\n\
         header = \"x-api-key: {key}\"\n\
         header = \"anthropic-version: {API_VERSION}\"\n\
         header = \"content-type: application/json\"\n\
         data-binary = \"@{}\"\n\
         output = \"{}\"\n\
         dump-header = \"{}\"\n\
         max-time = 300\n\
         silent\n\
         show-error\n\
         write-out = \"%{{http_code}}\"\n",
        q(req),
        q(resp),
        q(head),
    )
}

/// A `retry-after` header in seconds, from a header dump.
fn retry_after(headers: &str) -> Option<u64> {
    headers.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim()
            .eq_ignore_ascii_case("retry-after")
            .then(|| v.trim().parse().ok())
            .flatten()
    })
}

impl Agent for Claude {
    fn reply(&mut self, req: &Request) -> Result<Reply, String> {
        if self.spent >= self.max_cost {
            eprintln!(
                "claude: ${:.4} spent, at the cap of ${:.4}: stopping",
                self.spent, self.max_cost
            );
            std::process::exit(3);
        }
        let body = body(&self.model, &self.effort, req)?;
        let raw = self.post(&body)?;
        let (text, usage) = parse(&self.model, &raw);
        // A response that was billed is counted, whatever its content.
        let cost = price(&self.model).map_or(0.0, |p| usage.cost(&p));
        self.spent += cost;
        self.largest_prompt = self.largest_prompt.max(usage.prompt());
        writeln!(
            self.log,
            "{}",
            json!({
                "prompt": usage.prompt(), "largest": self.largest_prompt,
                "input": usage.input, "cache_write": usage.cache_write,
                "cache_read": usage.cache_read, "output": usage.output,
                "usd": cost, "spent": self.spent, "ok": text.is_ok(),
            })
        )
        .ok();
        Ok(Reply {
            text: text?,
            input_tokens: usage.prompt(),
            output_tokens: usage.output,
        })
    }
}

/// `additionalProperties: false` on every object schema, which structured output requires, by
/// rewriting the text. Not through `serde_json::Value`: its map sorts keys, and the endpoint's
/// grammar follows the order properties are written in. Sorted, only `services`' branch begins
/// with `thought` (`service` sorts before it elsewhere), so a reply that begins with its reasoning
/// can only be `services` — M7-P1's failure on Ollama, met again in M7-P4's pilot, where 11 of
/// 12 runs called `services` to the end of the budget while their thoughts named other tools.
/// The loop's schema writes every object as `{"type":"object",…`, and a test holds it to that.
pub fn closed(schema: &str) -> String {
    schema.replace(
        r#"{"type":"object","#,
        r#"{"type":"object","additionalProperties":false,"#,
    )
}

/// The request A6-8 froze, as text: the schema is spliced in as the loop wrote it, so its
/// property order reaches the endpoint (see `closed`).
pub fn body(model: &str, effort: &str, req: &Request) -> Result<String, String> {
    let raw = req.schema.as_deref().ok_or("a step with no schema")?;
    let schema = closed(raw);
    serde_json::from_str::<Value>(&schema).map_err(|e| format!("schema: {e}"))?;
    let messages: Vec<Value> = req
        .messages
        .iter()
        .map(|m| json!({"role": m.role.as_str(), "content": m.content}))
        .collect();
    const SLOT: &str = "\u{0}schema\u{0}";
    let body = json!({
        "model": model,
        "max_tokens": req.max_output,
        "system": [{"type": "text", "text": req.system, "cache_control": {"type": "ephemeral"}}],
        "messages": messages,
        "cache_control": {"type": "ephemeral"},
        "thinking": {"type": "disabled"},
        "output_config": {
            "effort": effort,
            "format": {"type": "json_schema", "schema": SLOT},
        },
    })
    .to_string();
    // The slot serialises as one quoted string no message can contain unescaped.
    let slot = serde_json::to_string(SLOT).expect("a string");
    Ok(body.replacen(&slot, &schema, 1))
}

/// The reply's text, or why there is none; and the usage, which is billed either way.
pub fn parse(model: &str, raw: &str) -> (Result<String, String>, Usage) {
    let v: Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => return (Err(format!("response is not JSON: {e}")), Usage::default()),
    };
    let n = |p: &str| v.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    let usage = Usage {
        input: n("/usage/input_tokens"),
        cache_write: n("/usage/cache_creation_input_tokens"),
        cache_read: n("/usage/cache_read_input_tokens"),
        output: n("/usage/output_tokens"),
    };
    let answered_by = v.get("model").and_then(Value::as_str).unwrap_or("");
    if answered_by != model {
        return (Err(format!("answered by `{answered_by}`")), usage);
    }
    if v.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        return (Err("refusal".into()), usage);
    }
    // Text blocks, in order; any other block (thinking, were it on) is not the answer.
    let text: String = v
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (Ok(text), usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl::{Message, StructuredMode};

    fn req() -> Request {
        let mut r = Request::new("ignored", String::new())
            .with_system("the system prompt".to_string())
            .with_max_output(1024)
            .with_schema(StructuredMode::JsonSchema, crate::agent::schema());
        r.messages = vec![
            Message::user("alert".to_string()),
            Message::assistant("{}".to_string()),
            Message::user("result".to_string()),
        ];
        r
    }

    #[test]
    fn the_body_is_what_a6_8_froze() {
        let text = body("claude-haiku-5-5", "medium", &req()).expect("a body");
        let b: Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(b["model"], "claude-haiku-5-5");
        assert_eq!(b["max_tokens"], 1024);
        assert_eq!(b["thinking"]["type"], "disabled");
        assert_eq!(b["output_config"]["effort"], "medium");
        assert_eq!(b["output_config"]["format"]["type"], "json_schema");
        assert_eq!(b["system"][0]["text"], "the system prompt");
        assert_eq!(b["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(b["cache_control"]["type"], "ephemeral");
        assert_eq!(b["messages"].as_array().map(Vec::len), Some(3));
        assert_eq!(b["messages"][1]["role"], "assistant");
        for k in ["temperature", "top_p", "top_k", "fallbacks", "tools"] {
            assert!(b.get(k).is_none(), "{k}");
        }
    }

    #[test]
    fn every_object_is_closed_and_the_branches_are_kept() {
        let text = body("claude-haiku-5-5", "medium", &req()).expect("a body");
        let b: Value = serde_json::from_str(&text).expect("JSON");
        let s = &b["output_config"]["format"]["schema"];
        let branches = s["anyOf"].as_array().expect("anyOf");
        assert_eq!(branches.len(), crate::agent::TOOLS.len());
        let mut objects = 0;
        fn walk(v: &Value, n: &mut usize) {
            match v {
                Value::Object(o) => {
                    if o.get("type").and_then(Value::as_str) == Some("object") {
                        assert_eq!(o.get("additionalProperties"), Some(&Value::Bool(false)));
                        *n += 1;
                    }
                    o.values().for_each(|x| walk(x, n));
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, n)),
                _ => {}
            }
        }
        walk(s, &mut objects);
        // One per branch, and the quote objects inside `hypothesise` and `answer`.
        assert!(objects > branches.len(), "{objects}");
        // The loop's own schema, which Ollama gets, is untouched.
        assert!(!crate::agent::schema().contains("additionalProperties"));
        // Every object the loop writes is spelt as `closed` expects.
        let schema = crate::agent::schema();
        assert_eq!(
            schema.matches(r#""type":"object""#).count(),
            schema.matches(r#"{"type":"object","#).count()
        );
    }

    /// The pilot's failure: a sorted schema leaves `thought` first in `services`' branch alone.
    /// The body carries the loop's text, so `thought` comes first in every branch, as sent.
    #[test]
    fn the_schema_reaches_the_endpoint_in_the_loops_order() {
        for schema in [crate::agent::schema(), crate::agent::answer_schema()] {
            let mut r = req();
            r.schema = Some(schema.clone());
            let text = body("claude-haiku-5-5", "medium", &r).expect("a body");
            assert!(
                text.contains(&closed(&schema)),
                "the schema is spliced verbatim"
            );
            // A branch is an object with a `tool`; the quote objects inside have none.
            let branches = text.matches(r#""tool":{"const""#).count();
            assert!(branches >= 1);
            assert_eq!(
                text.matches(r#""properties":{"thought""#).count(),
                branches,
                "thought first in every branch"
            );
        }
    }

    #[test]
    fn a_reply_is_its_text_blocks_and_its_usage() {
        let raw = r#"{"model":"claude-haiku-5-5","stop_reason":"end_turn",
            "content":[{"type":"text","text":"{\"tool\":"},{"type":"text","text":"\"services\"}"}],
            "usage":{"input_tokens":100,"cache_creation_input_tokens":2000,
                     "cache_read_input_tokens":30000,"output_tokens":50}}"#;
        let (t, u) = parse("claude-haiku-5-5", raw);
        assert_eq!(t.as_deref(), Ok(r#"{"tool":"services"}"#));
        assert_eq!(u.prompt(), 32_100);
        let p = price("claude-haiku-5-5").expect("a price");
        // 100 × 0.10 + 2000 × 0.125 + 30000 × 0.01 + 50 × 0.50, per million.
        assert!((u.cost(&p) - 585.0e-6).abs() < 1e-12, "{}", u.cost(&p));
    }

    #[test]
    fn another_model_or_a_refusal_is_no_answer_but_is_billed() {
        let other = r#"{"model":"claude-sonnet-5-5","stop_reason":"end_turn",
            "content":[{"type":"text","text":"{}"}],"usage":{"input_tokens":7,"output_tokens":1}}"#;
        let (t, u) = parse("claude-haiku-5-5", other);
        assert!(t.unwrap_err().contains("claude-sonnet-5-5"));
        assert_eq!(u.input, 7);
        let refused = r#"{"model":"claude-haiku-5-5","stop_reason":"refusal","content":[],
            "usage":{"input_tokens":7,"output_tokens":0}}"#;
        assert_eq!(parse("claude-haiku-5-5", refused).0, Err("refusal".into()));
    }

    #[test]
    fn curl_reads_the_key_from_its_configuration_and_the_body_from_a_file() {
        let c = curl_config(
            "sk-test",
            Path::new("/w/request.json"),
            Path::new("/w/response.json"),
            Path::new("/w/headers.txt"),
        );
        assert!(c.contains("header = \"x-api-key: sk-test\"\n"), "{c}");
        assert!(c.contains("data-binary = \"@/w/request.json\"\n"), "{c}");
        assert!(
            c.contains("header = \"anthropic-version: 2026-10-21\"\n"),
            "{c}"
        );
        assert!(c.contains("write-out = \"%{http_code}\"\n"), "{c}");
        assert_eq!(retry_after("HTTP/2 429\r\nRetry-After: 7\r\n"), Some(7));
        assert_eq!(retry_after("HTTP/2 529\r\n"), None);
    }

    #[test]
    fn a_long_prompt_is_priced_on_the_long_card() {
        let p = price("claude-haiku-5-5").expect("a price");
        let u = Usage {
            input: 100_001,
            ..Usage::default()
        };
        assert!((u.cost(&p) - 100_001.0 * 0.50 / 1e6).abs() < 1e-12);
        assert!(price("claude-opus-5-5").is_none(), "no unpriced model runs");
    }
}
