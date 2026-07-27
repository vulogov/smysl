//! The SM-P14 gate's first clause: **the same ingest fixture yields conformant units on all
//! five providers.**
//!
//! One fixture, one assertion, run against whichever providers this machine can reach. A
//! provider with no key or no server is skipped rather than failed - a test that failed for
//! want of a credential would be a test everyone learns to ignore.
//!
//! ```text
//! SMYSL_OLLAMA=required   ollama must be reachable
//! DEEPSEEK_API_KEY=…      deepseek runs
//! ANTHROPIC_API_KEY=…     anthropic runs
//! OPENAI_API_KEY=…        openai runs
//! GEMINI_API_KEY=…        gemini runs
//! SMYSL_INGEST_LIVE=all   every provider above must run, or the test fails
//! ```
//!
//! **Conformant** is the claim being tested, and it is deliberately not "the units are
//! good". It is: every unit passes `check`, no unit exceeds its rung's ceiling, and the run
//! produced something. What a model chooses to say is its business; what the boundary
//! guarantees is that whatever it said arrives shaped correctly or not at all.

use smysl_core::{Rung, Severity, Status};
use smysl_graph::Store;
use smysl_ingest::{IngestOptions, Ingestor};
use smysl_provider::{
    config::ProviderConfig, Provider, ProviderId, Registry, StructuredMode, Task,
};

/// The fixture. Short on purpose - every hosted call costs money, and the claim under test
/// is about shape rather than about depth.
const FIXTURE: &str = "\
On Thursday the eu-west shard slowed: p95 request latency rose from 180ms to 410ms.

Connection pool wait time rose alongside it, which suggests pool saturation as the cause.

The canary shard stayed clean throughout, which the pool theory does not explain.";

struct Candidate {
    id: &'static str,
    kind: &'static str,
    endpoint: &'static str,
    model: &'static str,
    key_var: Option<&'static str>,
    structured: StructuredMode,
    context_window: usize,
}

/// Every provider the workspace can drive. Availability is decided at run time.
fn candidates() -> Vec<Candidate> {
    vec![
        Candidate {
            id: "ollama",
            kind: "ollama",
            endpoint: "http://127.0.0.1:11434",
            model: "llama3.2",
            key_var: None,
            structured: StructuredMode::JsonSchema,
            context_window: 8192,
        },
        Candidate {
            id: "deepseek",
            kind: "deepseek",
            endpoint: "https://api.deepseek.com",
            model: "deepseek-chat",
            key_var: Some("DEEPSEEK_API_KEY"),
            structured: StructuredMode::JsonMode,
            context_window: 65536,
        },
        Candidate {
            id: "anthropic",
            kind: "anthropic",
            endpoint: "https://api.anthropic.com",
            model: "claude-sonnet-4-5",
            key_var: Some("ANTHROPIC_API_KEY"),
            structured: StructuredMode::ToolForce,
            context_window: 200_000,
        },
        Candidate {
            id: "openai",
            kind: "openai",
            endpoint: "https://api.openai.com",
            model: "gpt-4.1-mini",
            key_var: Some("OPENAI_API_KEY"),
            structured: StructuredMode::JsonSchema,
            context_window: 128_000,
        },
        Candidate {
            id: "gemini",
            kind: "gemini",
            endpoint: "https://generativelanguage.googleapis.com",
            model: "gemini-2.5-flash",
            key_var: Some("GEMINI_API_KEY"),
            structured: StructuredMode::JsonSchema,
            context_window: 1_000_000,
        },
    ]
}

fn config(c: &Candidate) -> ProviderConfig {
    let mut cfg = ProviderConfig::new(ProviderId::new(c.id).unwrap(), c.kind);
    cfg.endpoint = c.endpoint.into();
    cfg.model = c.model.into();
    cfg.structured = c.structured;
    cfg.context_window = c.context_window;
    cfg.max_output = 800;
    cfg.timeout_secs = 120;
    cfg.api_key_env = c.key_var.map(str::to_string);
    cfg
}

/// Whether this candidate can run here: compiled in, credential present, endpoint up.
fn usable(c: &Candidate) -> Result<Box<dyn Provider>, String> {
    let cfg = config(c);
    let provider = smysl_provider::map::build(&cfg).map_err(|e| e.to_string())?;

    if let Some(var) = c.key_var {
        let set = std::env::var(var).is_ok_and(|v| !v.trim().is_empty());
        if !set {
            return Err(format!("${var} is unset"));
        }
    }
    match provider.probe() {
        Ok(p) if p.reachable => Ok(provider),
        Ok(p) => Err(p.detail),
        Err(e) => Err(e.to_string()),
    }
}

/// **The gate.** The same fixture, every reachable provider, one assertion.
#[test]
fn the_same_fixture_yields_conformant_units_on_every_reachable_provider() {
    let require_all = std::env::var("SMYSL_INGEST_LIVE").as_deref() == Ok("all");
    let mut ran = Vec::new();
    let mut skipped = Vec::new();

    for c in candidates() {
        let provider = match usable(&c) {
            Ok(p) => p,
            Err(why) => {
                assert!(
                    !require_all,
                    "SMYSL_INGEST_LIVE=all but {} cannot run: {why}",
                    c.id
                );
                skipped.push(format!("{} ({why})", c.id));
                continue;
            }
        };

        let id = provider.id();
        let registry = Registry::new()
            .with_provider(provider)
            .route(Task::ContentIngest, id.clone());

        let opts = IngestOptions::at_rung(Rung::Document)
            .with_model(c.model)
            .with_max_output(800);

        let (staged, report) = Ingestor::new(&registry, opts)
            .ingest(&Store::new(), FIXTURE)
            .unwrap_or_else(|e| panic!("{}: ingest failed: {e}", c.id));

        // Rule I: something is always produced, whatever the model did.
        assert!(!staged.is_empty(), "{}: produced nothing at all", c.id);

        // Conformant: every staged unit passes `check` at error severity.
        assert!(
            staged.report.fail_on(Severity::Error).is_ok(),
            "{}: staged units do not check:\n{}",
            c.id,
            staged
                .report
                .iter()
                .filter(|d| d.severity == Severity::Error)
                .map(|d| format!("  {d}"))
                .collect::<Vec<_>>()
                .join("\n")
        );

        // Rule T: nothing above the ceiling, whatever the model claimed.
        for u in &staged.units {
            assert!(
                u.status <= Status::Cited,
                "{}: `{}` claims {} above the document ceiling",
                c.id,
                u.gist,
                u.status
            );
            assert_ne!(
                u.status,
                Status::Measured,
                "{}: ingest assigned measured",
                c.id
            );
        }

        // And the batch is readable surface text, which is what a human is asked to approve.
        let surface = staged.to_surface();
        assert!(surface.starts_with('@'), "{}: not surface text", c.id);

        ran.push(format!(
            "{}: {} unit(s), {} call(s), {} degraded, {} token(s){}",
            c.id,
            staged.len(),
            report.calls,
            report.degraded,
            report.usage.total(),
            if report.degraded > 0 { " [rule I]" } else { "" }
        ));
    }

    // Printed rather than asserted: which providers a machine can reach is a fact about the
    // machine, and the record of what ran is the useful output when some were skipped.
    eprintln!("--- SM-P14 gate, clause 1 ---");
    for line in &ran {
        eprintln!("  ran     {line}");
    }
    for line in &skipped {
        eprintln!("  skipped {line}");
    }

    // Skipping is the normal outcome on a machine with no keys and no local server, and a
    // test that failed for want of a credential is a test everyone learns to ignore.
    // `SMYSL_INGEST_LIVE=all` is how CI says otherwise.
    assert!(
        !require_all || !ran.is_empty(),
        "SMYSL_INGEST_LIVE=all but no provider ran"
    );
}

/// E9's raw material: which path each provider took, and whether its structure was
/// enforced. D-9 leaves DeepSeek's default path to this measurement rather than assuming.
#[test]
fn each_provider_reports_its_own_structural_guarantee() {
    for c in candidates() {
        let Ok(provider) = usable(&c) else { continue };
        let caps = provider.caps();

        // The mapper reports what the endpoint enforces, not what the config asked for.
        match c.kind {
            "deepseek" => assert!(
                !caps.structured.is_enforced(),
                "deepseek claims enforcement it does not have"
            ),
            "anthropic" => assert_eq!(caps.structured, StructuredMode::ToolForce),
            _ => {}
        }

        let choice = smysl_ingest::path::choose(&caps, Task::ContentIngest, FIXTURE.len(), None);
        eprintln!(
            "  {:<10} structured {:<11} path {:<9} ({})",
            c.id,
            caps.structured,
            choice.path,
            choice.reason.as_str()
        );
    }
}
