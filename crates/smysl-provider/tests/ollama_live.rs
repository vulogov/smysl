//! The SM-P13 gate's live half: `providers --probe` against a running local model.
//!
//! Every other test in this crate is deliberately network-free - routing, fallback and
//! offline are decided from configuration, so testing them needs no server. This file is
//! the exception, and it exists because the RFC's implementation note is explicit that
//! endpoint paths and field names must be *verified against a running server*, not
//! remembered.
//!
//! **Skipped when no server is listening.** A test that failed on a machine without Ollama
//! would train everyone to ignore it, which is worse than not having it. Set
//! `SMYSL_OLLAMA=required` to turn the skip into a failure - which is what CI should do.
//!
//! Nothing here sends unit content. The probe asks what the provider *is*; the one
//! completion test sends a fixed two-word prompt of its own.

#![cfg(feature = "ollama")]

use std::time::Duration;

use smysl_provider::config::ProviderConfig;
use smysl_provider::map::ollama::Ollama;
use smysl_provider::{
    Provider, ProviderError, ProviderId, Registry, Request, StructuredMode, Task,
};

const ENDPOINT: &str = "http://127.0.0.1:11434";

fn cfg() -> ProviderConfig {
    let mut c = ProviderConfig::new(ProviderId::new("ollama").unwrap(), "ollama");
    c.endpoint = std::env::var("SMYSL_OLLAMA_ENDPOINT").unwrap_or_else(|_| ENDPOINT.into());
    c.model = std::env::var("SMYSL_OLLAMA_MODEL").unwrap_or_else(|_| "llama3.2".into());
    c.context_window = 8192;
    c.max_output = 32;
    c.structured = StructuredMode::JsonSchema;
    c.timeout_secs = 120;
    c
}

/// Whether to run. Returns false to skip, panics if a server was required and is absent.
fn available(what: &str) -> bool {
    let required = std::env::var("SMYSL_OLLAMA").as_deref() == Ok("required");
    let p = Ollama::new(cfg());
    match p.probe() {
        Ok(probe) if probe.reachable => true,
        other => {
            let why = match other {
                Ok(_) => "no server listening".to_string(),
                Err(e) => e.to_string(),
            };
            assert!(
                !required,
                "SMYSL_OLLAMA=required but {what} cannot run: {why}"
            );
            eprintln!("skipping {what}: {why} (set SMYSL_OLLAMA=required to fail instead)");
            false
        }
    }
}

/// **The gate.** `providers --probe` reports correct capabilities against a live model.
///
/// "Correct" means measured rather than echoed: the context window comes from the server's
/// own `model_info`, so a probe that merely repeated the configured 8192 would fail here.
#[test]
fn probe_reports_capabilities_from_the_live_server() {
    if !available("probe") {
        return;
    }
    let p = Ollama::new(cfg());
    let probe = p.probe().expect("a reachable server probes");

    assert!(probe.reachable);
    assert!(
        !probe.models.is_empty(),
        "a server with no models: {probe:?}"
    );

    let caps = probe.caps.expect("capabilities were read from the server");
    assert!(caps.offline, "loopback is offline-capable");
    assert!(caps.usage_reporting);
    assert!(
        caps.context_window >= 2048,
        "implausible context window: {}",
        caps.context_window
    );
    assert_ne!(
        caps.context_window,
        cfg().context_window,
        "the probe echoed the configuration instead of measuring the model"
    );
    assert_eq!(
        caps.max_output,
        cfg().max_output,
        "output is ours to choose"
    );
}

#[test]
fn probe_names_the_installed_models_and_says_whether_ours_is_among_them() {
    if !available("model listing") {
        return;
    }
    let probe = Ollama::new(cfg()).probe().unwrap();
    let model = cfg().model;
    assert!(
        probe.models.iter().any(|m| m.starts_with(&model)),
        "{model} is not installed; {:?}",
        probe.models
    );
    assert!(probe.detail.contains(&model), "{}", probe.detail);
}

/// A probe against a port nothing is listening on is `reachable: false` rather than an
/// error: `providers --probe` reports what it found, and "nothing there" is a finding.
#[test]
fn an_absent_server_probes_as_unreachable_rather_than_erroring() {
    let mut c = cfg();
    // Port 1 is reserved and never a model server.
    c.endpoint = "http://127.0.0.1:1".into();
    c.timeout_secs = 2;
    let probe = Ollama::new(c)
        .probe()
        .expect("a probe reports, it does not fail");
    assert!(!probe.reachable);
    assert!(probe.models.is_empty());
    assert!(probe.caps.is_none());
}

/// The whole round trip against a real model: request body, HTTP, response parsing, usage
/// extraction. This is what proves the mapper matches the endpoint rather than my memory
/// of it.
#[test]
fn a_completion_round_trips_through_the_live_server() {
    if !available("completion") {
        return;
    }
    let p = Ollama::new(cfg());
    let req = Request::new(cfg().model, "Reply with the single word: ok")
        .with_system("Answer with one word and no punctuation.")
        .with_max_output(8);

    let c = p.complete(&req).expect("a reachable server completes");
    assert!(!c.text.trim().is_empty(), "empty completion");
    assert!(c.model.starts_with("llama") || !c.model.is_empty());

    // Usage reporting on a completed response is real, not estimated.
    assert!(!c.usage.estimated, "a completed response reports counts");
    assert!(c.usage.input_tokens > 0, "the prompt was counted");
    assert!(c.usage.output_tokens > 0, "the output was counted");
    assert_eq!(c.usage.retries, 0);
}

/// Structured output, which is the mechanism ingest will rest on at SM-P14.
#[test]
fn a_json_schema_is_enforced_by_the_live_server() {
    if !available("structured output") {
        return;
    }
    let schema = r#"{"type":"object","properties":{"answer":{"type":"string"}},
                     "required":["answer"]}"#;
    let req = Request::new(
        cfg().model,
        "What colour is a clear midday sky? Answer briefly.",
    )
    .with_schema(StructuredMode::JsonSchema, schema)
    .with_max_output(64);

    let c = Ollama::new(cfg())
        .complete(&req)
        .expect("a reachable server completes");
    assert!(c.structured, "the configuration enforces a schema");

    let v: serde_json::Value = serde_json::from_str(c.text.trim())
        .unwrap_or_else(|e| panic!("not JSON despite the schema: {e}\n{}", c.text));
    assert!(v.get("answer").is_some(), "schema not honoured: {}", c.text);
}

/// Streaming over the synchronous channel (§21.5), against a real server.
#[test]
fn streaming_delivers_tokens_over_the_channel() {
    if !available("streaming") {
        return;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let req = Request::new(cfg().model, "Count from one to five, words only.").with_max_output(48);

    let usage = Ollama::new(cfg())
        .stream(&req, tx)
        .expect("a reachable server streams");

    let mut stream = smysl_provider::stream::Stream::new(rx);
    let text = stream.drain();
    assert!(!text.trim().is_empty(), "no tokens arrived");
    assert!(stream.is_finished());
    assert!(usage.output_tokens > 0);
}

/// **The gate, offline half, against a live server**: the local provider is permitted and
/// a hosted one is refused, with no call made either way.
#[test]
fn offline_admits_the_live_local_provider_and_refuses_a_hosted_one() {
    let local = ProviderId::new("ollama").unwrap();
    let hosted_id = ProviderId::new("hosted").unwrap();
    let mut hosted = cfg();
    hosted.id = hosted_id.clone();
    hosted.endpoint = "https://ollama.example.com".into();

    let r = Registry::new()
        .with_provider(Box::new(Ollama::new(cfg())))
        .with_provider(Box::new(Ollama::new(hosted)))
        .route(Task::ContentIngest, local.clone())
        .route(Task::Attest, hosted_id)
        .offline(true);

    assert!(
        r.for_task(Task::ContentIngest).is_ok(),
        "loopback is allowed"
    );
    assert_eq!(
        r.for_task(Task::Attest).map(|_| ()).unwrap_err(),
        ProviderError::OfflineViolation
    );

    let rows = r.egress_report();
    let ingest = rows.iter().find(|e| e.task == Task::ContentIngest).unwrap();
    assert!(!ingest.leaves_machine, "a local model egresses nothing");
    let attest = rows.iter().find(|e| e.task == Task::Attest).unwrap();
    assert!(attest.leaves_machine);
}

/// A completion against a model that is not installed must not fall back: it is a
/// configuration error, and a fallback would paper over it with a different model's answer.
#[test]
fn a_missing_model_surfaces_rather_than_falling_back() {
    if !available("missing-model handling") {
        return;
    }
    let mut missing = cfg();
    missing.id = ProviderId::new("primary").unwrap();
    missing.model = "no-such-model-93f2a".into();
    let mut spare = cfg();
    spare.id = ProviderId::new("spare").unwrap();

    let r = Registry::new()
        .with_provider(Box::new(Ollama::new(missing.clone())))
        .with_provider(Box::new(Ollama::new(spare)))
        .route(Task::ContentIngest, missing.id.clone())
        .with_fallback([ProviderId::new("spare").unwrap()]);

    let e = r
        .complete(
            Task::ContentIngest,
            &Request::new(&missing.model, "hi").with_max_output(4),
        )
        .expect_err("a missing model is an error, not a fallback");
    assert!(
        matches!(e, ProviderError::Malformed(_)),
        "expected a configuration error, got {e}"
    );
}

/// A provider pointed at nothing is `Unreachable`, which *is* the case a fallback exists
/// for - and the live server is what proves the fallback actually completes.
#[test]
fn fallback_from_a_dead_endpoint_reaches_the_live_server() {
    if !available("fallback") {
        return;
    }
    let mut dead = cfg();
    dead.id = ProviderId::new("dead").unwrap();
    dead.endpoint = "http://127.0.0.1:1".into();
    dead.timeout_secs = 2;

    let r = Registry::new()
        .with_provider(Box::new(Ollama::new(dead)))
        .with_provider(Box::new(Ollama::new(cfg())))
        .route(Task::ContentIngest, ProviderId::new("dead").unwrap())
        .with_fallback([ProviderId::new("ollama").unwrap()]);

    let out = r
        .complete(
            Task::ContentIngest,
            &Request::new(cfg().model, "Say ok.").with_max_output(8),
        )
        .expect("the fallback answers");
    assert_eq!(out.provider, ProviderId::new("ollama").unwrap());
    assert_eq!(out.skipped, vec![ProviderId::new("dead").unwrap()]);
    assert!(!out.completion.text.trim().is_empty());
}

/// The timeout is honoured, so a hung server cannot wedge a pipeline for ever.
#[test]
fn a_dead_endpoint_fails_within_its_timeout() {
    let mut c = cfg();
    c.endpoint = "http://127.0.0.1:1".into();
    c.timeout_secs = 2;

    let start = std::time::Instant::now();
    let e = Ollama::new(c)
        .complete(&Request::new("m", "hi"))
        .expect_err("nothing is listening");
    assert_eq!(e, ProviderError::Unreachable);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "took {:?}",
        start.elapsed()
    );
}
