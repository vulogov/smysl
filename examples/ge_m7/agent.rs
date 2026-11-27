//! The agent loop (RFC SMYSL-10 §4, §8): one structured completion per step, the model returning
//! `{tool, …}` and the harness running it, until `answer` or the budget. A run that spends the
//! budget without answering gets one more call whose only branch is `answer` (A4-3).
//!
//! One system prompt for every arm. The arms differ in what comes back: `hypothesise` answers
//! with a note in (a) and with the unit, its status and rule TQ's findings in (b) and (d); (d)'s
//! first message lists BARO's ranking.

use std::collections::BTreeMap;
use std::time::Instant;

use smysl::{Message, Provider, Request, StructuredMode};

use crate::world::{Quote, World, CAP, CASE_S};

pub const TOOLS: &[&str] = &[
    "services",
    "metrics",
    "window",
    "compare",
    "logs",
    "traces",
    "hypothesise",
    "answer",
];

/// The step schema: one object per tool (`anyOf`), each naming its tool as a `const` and
/// requiring its arguments. Ollama turns it into a grammar (`format`) that follows the order the
/// properties are written in, so the text is built by hand: `serde_json::Map` sorts its keys,
/// and a sorted schema put `thought` first only in `services`' branch, so every reply that began
/// with its reasoning could only be `services` (the M7-P1 smoke runs). With a flat object and
/// every argument optional, the same model dropped `from` and repeated the call to the end of
/// the budget. `thought` comes first in every branch: the model says why before it picks.
pub fn schema() -> String {
    format!(r#"{{"anyOf":[{}]}}"#, branches().join(","))
}

/// The forced step's schema (A4-3): `answer`'s branch alone.
pub fn answer_schema() -> String {
    let b = branches();
    format!(
        r#"{{"anyOf":[{}]}}"#,
        b.last().expect("answer is the last branch")
    )
}

fn branches() -> Vec<String> {
    const STR: &str = r#"{"type":"string"}"#;
    const INT: &str = r#"{"type":"integer"}"#;
    const STRS: &str = r#"{"type":"array","items":{"type":"string"}}"#;
    const QUOTES: &str = r#"{"type":"array","items":{"type":"object","properties":{"id":{"type":"string"},"field":{"type":"string"},"value":{"type":"string"}},"required":["id","field","value"]}}"#;
    let tool = |name: &str, args: &[(&str, &str)]| {
        let mut props = format!(r#""thought":{STR},"tool":{{"const":"{name}"}}"#);
        let mut req = String::from(r#""thought","tool""#);
        for (k, v) in args {
            props.push_str(&format!(r#","{k}":{v}"#));
            req.push_str(&format!(r#","{k}""#));
        }
        format!(r#"{{"type":"object","properties":{{{props}}},"required":[{req}]}}"#)
    };
    vec![
        tool("services", &[]),
        tool("metrics", &[("service", STR)]),
        tool("window", &[("series", STR), ("from", INT), ("to", INT)]),
        tool("compare", &[("series", STR), ("at", INT)]),
        tool("logs", &[("service", STR), ("from", INT), ("to", INT)]),
        tool("traces", &[("service", STR), ("from", INT), ("to", INT)]),
        tool(
            "hypothesise",
            &[
                ("service", STR),
                ("claim", STR),
                ("quotes", QUOTES),
                ("grounds", STRS),
            ],
        ),
        tool("answer", &[("ranking", STRS), ("quotes", QUOTES)]),
    ]
}

/// A tool's call as the system prompt shows it, for an argument error.
fn usage(tool: &str) -> &'static str {
    match tool {
        "metrics" => r#"{"tool":"metrics","service":"svc-NN"}"#,
        "window" => r#"{"tool":"window","series":"svc-NN/cpu","from":600,"to":720}"#,
        "compare" => r#"{"tool":"compare","series":"svc-NN/cpu","at":720}"#,
        "logs" => r#"{"tool":"logs","service":"svc-NN","from":600,"to":1080}"#,
        "traces" => r#"{"tool":"traces","service":"svc-NN","from":600,"to":1080}"#,
        _ => "",
    }
}

pub fn system_prompt(steps: usize) -> String {
    format!(
        "You are an on-call engineer finding the root cause of an incident in a microservice \
system. Services are named svc-01, svc-02, and so on; svc-NN and svc-MM below stand for any \
of them, not for a service to start with. Time is in seconds from the start of the \
recording, which lasts {CASE_S} s.\n\
\n\
Reply with exactly one JSON object per turn: a tool call, its first field `thought`, a sentence \
or two on what you know so far and why this call is next. The tools (shown without `thought`):\n\
- {{\"tool\":\"services\"}}: the services, and what data each has.\n\
- {{\"tool\":\"metrics\",\"service\":\"svc-NN\"}}: a service's metric series.\n\
- {{\"tool\":\"window\",\"series\":\"svc-NN/cpu\",\"from\":600,\"to\":720}}: a summary of a \
series over [from, to): an exact summary (count, min, max, sum, mean, first, last) and a sketched \
one (q[0] p50, q[1] p90, q[2] p99), each with an id.\n\
- {{\"tool\":\"compare\",\"series\":\"svc-NN/cpu\",\"at\":720}}: window over the 360 s before \
`at` and the 360 s after.\n\
- {{\"tool\":\"logs\",\"service\":\"svc-NN\",\"from\":600,\"to\":1080}}: log templates with \
counts, each with an id.\n\
- {{\"tool\":\"traces\",\"service\":\"svc-NN\",\"from\":600,\"to\":1080}}: per traced \
operation, its calls and its latency in ms, as windows with ids.\n\
- {{\"tool\":\"hypothesise\",\"service\":\"svc-NN\",\"claim\":\"…\",\"quotes\":[{{\"id\":\"…\",\
\"field\":\"max\",\"value\":\"0.93\"}}],\"grounds\":[\"…\"]}}: record a hypothesis that a \
service is the root cause, with the numbers it rests on.\n\
- {{\"tool\":\"answer\",\"ranking\":[\"svc-NN\",\"svc-MM\"],\"quotes\":[…]}}: your final \
answer: up to five services, most likely root cause first, and the numbers that support it. \
This ends the investigation.\n\
\n\
Rules. Every number you state is a quote: the id of the summary it comes from, the field \
(count, min, max, sum, mean, first, last, q[0], q[1], q[2]), and the value exactly as the tool \
printed it. Quote only ids a tool returned. You have {steps} tool calls; answer before they run \
out."
    )
}

/// What came back from one call to the model.
pub struct Reply {
    pub text: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// The model, or a script standing in for it.
pub trait Agent {
    fn reply(&mut self, req: &Request) -> Result<Reply, String>;
}

/// A provider: Ollama, or a hosted model.
pub struct ModelAgent {
    pub provider: Box<dyn Provider>,
}

impl Agent for ModelAgent {
    fn reply(&mut self, req: &Request) -> Result<Reply, String> {
        let c = self.provider.complete(req).map_err(|e| e.to_string())?;
        Ok(Reply {
            text: c.text,
            input_tokens: c.usage.input_tokens,
            output_tokens: c.usage.output_tokens,
        })
    }
}

/// One scripted step: the reply, given the transcript so far.
#[cfg_attr(not(test), allow(dead_code))]
pub type Step = Box<dyn Fn(&[Message]) -> String>;

/// A script: each step a function of the transcript so far.
#[cfg_attr(not(test), allow(dead_code))]
pub struct Scripted {
    pub steps: Vec<Step>,
    pub next: usize,
    /// The schema of every request, in order.
    pub schemas: Vec<Option<String>>,
}

impl Agent for Scripted {
    fn reply(&mut self, req: &Request) -> Result<Reply, String> {
        self.schemas.push(req.schema.clone());
        let f = self.steps.get(self.next).ok_or("the script ran out")?;
        self.next += 1;
        Ok(Reply {
            text: f(&req.messages),
            input_tokens: 0,
            output_tokens: 0,
        })
    }
}

/// The final answer.
#[derive(Debug, Clone, Default)]
pub struct Answer {
    pub ranking: Vec<String>,
    pub quotes: Vec<Quote>,
}

/// One run of one arm on one case.
#[derive(Debug, Default)]
pub struct Run {
    pub steps: usize,
    /// Why the run ended without an answer: `parse`, `tool`, `context`, `budget`, `provider`.
    pub protocol: Option<String>,
    pub answer: Option<Answer>,
    pub transcript: Vec<Message>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub secs: f64,
    pub calls: BTreeMap<String, usize>,
    /// Tool calls with a bad or missing argument, answered with an error.
    pub arg_errors: usize,
    /// Tool results cut at the cap.
    pub capped: usize,
    /// The answer came from the forced step after the budget (A4-3).
    pub forced: bool,
}

/// The message closing a spent budget, before the forced step.
pub const FORCED: &str = "No tool calls left: answer now, with what you have.";

fn quotes_of(v: &serde_json::Value) -> Vec<Quote> {
    v.get("quotes")
        .and_then(|q| q.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|q| {
                    let s = |k: &str| q.get(k).and_then(|x| x.as_str()).map(str::to_string);
                    Some(Quote {
                        id: s("id")?,
                        field: s("field")?,
                        value: s("value").or_else(|| q.get("value").map(|x| x.to_string()))?,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn strings(v: &serde_json::Value, k: &str) -> Vec<String> {
    v.get(k)
        .and_then(|q| q.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Run one tool call; `Ok(None)` is `answer`.
fn dispatch(world: &mut World, v: &serde_json::Value) -> Result<Option<String>, String> {
    let s = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .ok_or(format!("`{k}` is missing"))
    };
    let n = |k: &str| {
        v.get(k)
            .and_then(|x| x.as_u64().or_else(|| x.as_f64().map(|f| f.max(0.0) as u64)))
            .ok_or(format!("`{k}` is missing or not a number"))
    };
    let tool = v.get("tool").and_then(|x| x.as_str()).unwrap_or("");
    Ok(Some(match tool {
        "services" => world.tool_services(),
        "metrics" => world.tool_series(&s("service")?)?,
        "window" => world.tool_window(&s("series")?, n("from")?, n("to")?)?,
        "compare" => world.tool_compare(&s("series")?, n("at")?)?,
        "logs" => world.tool_logs(&s("service")?, n("from")?, n("to")?)?,
        "traces" => world.tool_traces(&s("service")?, n("from")?, n("to")?)?,
        "hypothesise" => world.tool_hypothesise(
            &s("service")?,
            &s("claim").unwrap_or_default(),
            &quotes_of(v),
            &strings(v, "grounds"),
        ),
        "answer" => return Ok(None),
        other => return Err(format!("unknown tool `{other}`")),
    }))
}

/// The run's settings.
pub struct Loop {
    pub model: String,
    pub steps: usize,
    pub context_window: usize,
    pub max_output: usize,
}

/// Drive `agent` on `world` from `first` (the alert, and in (d) the ranking).
pub fn run(world: &mut World, agent: &mut dyn Agent, cfg: &Loop, first: String) -> Run {
    let start = Instant::now();
    let mut r = Run::default();
    let system = system_prompt(cfg.steps);
    let mut messages = vec![Message::user(first)];
    let mut seen = std::collections::BTreeSet::new();
    while r.steps < cfg.steps {
        let v = match call(agent, cfg, &system, &mut messages, schema(), &mut r) {
            Ok(v) => v,
            Err(p) => {
                r.protocol = Some(p);
                break;
            }
        };
        let tool = v
            .get("tool")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if !TOOLS.contains(&tool.as_str()) {
            r.protocol = Some("tool".into());
            break;
        }
        *r.calls.entry(tool.clone()).or_default() += 1;
        let mut call = v.clone();
        if let Some(o) = call.as_object_mut() {
            o.remove("thought");
        }
        let call = call.to_string();
        let repeat = seen.contains(&call);
        seen.insert(call);
        let mut result = match if repeat {
            Err(
                "this exact call was made before; its result is above: call something else"
                    .to_string(),
            )
        } else {
            dispatch(world, &v)
        } {
            Ok(None) => {
                r.answer = Some(Answer {
                    ranking: strings(&v, "ranking"),
                    quotes: quotes_of(&v),
                });
                break;
            }
            Ok(Some(text)) => text,
            Err(e) => {
                r.arg_errors += 1;
                match usage(&tool) {
                    "" => format!("error: {e}"),
                    u => format!("error: {e}; the call is {u}"),
                }
            }
        };
        if result.len() > CAP {
            let mut cut = CAP;
            while !result.is_char_boundary(cut) {
                cut -= 1;
            }
            result.truncate(cut);
            result.push_str("\n… (cut at the result cap)");
            r.capped += 1;
        }
        let left = cfg.steps - r.steps;
        if left <= 3 {
            result.push_str(&format!(
                "\n({left} tool call{} left: answer before they run out)",
                if left == 1 { "" } else { "s" }
            ));
        }
        messages.push(Message::user(result));
    }
    // The budget is spent with no answer: one call more, whose only branch is `answer` (A4-3).
    if r.answer.is_none() && r.protocol.is_none() {
        if let Some(m) = messages.last_mut() {
            m.content.push_str(&format!("\n{FORCED}"));
        }
        r.forced = true;
        match call(agent, cfg, &system, &mut messages, answer_schema(), &mut r) {
            Ok(v) if v.get("tool").and_then(|x| x.as_str()) == Some("answer") => {
                *r.calls.entry("answer".into()).or_default() += 1;
                r.answer = Some(Answer {
                    ranking: strings(&v, "ranking"),
                    quotes: quotes_of(&v),
                });
            }
            Ok(_) => r.protocol = Some("budget".into()),
            Err(p) => r.protocol = Some(p),
        }
    }
    r.transcript = messages;
    r.secs = start.elapsed().as_secs_f64();
    r
}

/// One step: the model called on the transcript with `schema`, its reply appended and parsed.
/// `Err` is the protocol failure that ends the run.
fn call(
    agent: &mut dyn Agent,
    cfg: &Loop,
    system: &str,
    messages: &mut Vec<Message>,
    schema: String,
    r: &mut Run,
) -> Result<serde_json::Value, String> {
    r.steps += 1;
    let mut req = Request::new(cfg.model.clone(), String::new())
        .with_system(system.to_string())
        .with_max_output(cfg.max_output)
        .with_schema(StructuredMode::JsonSchema, schema);
    req.messages = messages.clone();
    // A transcript the window cannot hold is a protocol failure, not a silent truncation.
    let tokens = smysl::tokens(system) as usize
        + messages
            .iter()
            .map(|m| smysl::tokens(&m.content) as usize)
            .sum::<usize>();
    if tokens + cfg.max_output > cfg.context_window {
        return Err("context".into());
    }
    // `GE_M7_DUMP=DIR`: each step's system prompt, schema and messages, to replay against the
    // model by hand.
    if let Ok(d) = std::env::var("GE_M7_DUMP") {
        let body = serde_json::json!({
            "system": system,
            "schema": req.schema,
            "messages": messages
                .iter()
                .map(|m| serde_json::json!({"role": m.role.as_str(), "content": m.content}))
                .collect::<Vec<_>>(),
        });
        std::fs::write(format!("{d}/step{}.json", r.steps), body.to_string()).ok();
    }
    let reply = agent.reply(&req).map_err(|e| format!("provider: {e}"))?;
    r.input_tokens += reply.input_tokens;
    r.output_tokens += reply.output_tokens;
    messages.push(Message::assistant(reply.text.clone()));
    serde_json::from_str(reply.text.trim()).map_err(|_| "parse".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_answer_schema_is_answers_branch_alone() {
        let v: serde_json::Value = serde_json::from_str(&answer_schema()).expect("JSON");
        let b = v["anyOf"].as_array().expect("anyOf");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0]["properties"]["tool"]["const"], "answer");
    }

    #[test]
    fn the_schema_parses_and_keeps_thought_first_in_every_branch() {
        let text = schema();
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(v["anyOf"].as_array().map(Vec::len), Some(TOOLS.len()));
        for t in TOOLS {
            let at = text.find(&format!(r#""const":"{t}""#)).expect("a branch");
            let open = text[..at]
                .rfind(r#""properties":{"#)
                .expect("its properties");
            assert!(
                text[open..at].starts_with(r#""properties":{"thought""#),
                "{t}"
            );
        }
    }
}
