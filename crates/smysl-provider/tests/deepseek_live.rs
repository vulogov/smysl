//! DeepSeek against the live endpoint.
//!
//! Ollama is the CI conformance reference because it needs no key, but there are three
//! things it cannot exercise at all: TLS, a real `Unauthorized`, and a credential resolved
//! from the environment. Those are what this file is for.
//!
//! **Skipped unless `DEEPSEEK_API_KEY` is set.** Set `SMYSL_DEEPSEEK=required` to turn the
//! skip into a failure. Every test here costs money, so the prompts are two words and the
//! output limits are tiny.

#![cfg(feature = "deepseek")]

use smysl_provider::config::ProviderConfig;
use smysl_provider::map::deepseek::DeepSeek;
use smysl_provider::{
    Provider, ProviderError, ProviderId, Registry, Request, StructuredMode, Task,
};

const KEY_VAR: &str = "DEEPSEEK_API_KEY";

fn cfg() -> ProviderConfig {
    let mut c = ProviderConfig::new(ProviderId::new("deepseek").unwrap(), "deepseek");
    c.endpoint = "https://api.deepseek.com".into();
    c.model = std::env::var("SMYSL_DEEPSEEK_MODEL").unwrap_or_else(|_| "deepseek-chat".into());
    c.context_window = 65536;
    c.max_output = 32;
    c.structured = StructuredMode::JsonMode;
    c.api_key_env = Some(KEY_VAR.into());
    c.timeout_secs = 90;
    c
}

/// Opt-in, not key-triggered: a credential in the environment is not consent to spend it.
///
/// `SMYSL_DEEPSEEK` unset means skip whatever keys exist, `=1` means run where a key is
/// available, `=required` means a missing key is a failure. A key alone used to be enough,
/// which quietly charged anyone running `make test-matrix` with one exported.
fn available(what: &str) -> bool {
    let gate = std::env::var("SMYSL_DEEPSEEK").unwrap_or_default();
    if gate.trim().is_empty() {
        eprintln!("skipping {what}: set SMYSL_DEEPSEEK=1 to call the real endpoint");
        return false;
    }
    let required = gate == "required";
    let has_key = std::env::var(KEY_VAR).is_ok_and(|v| !v.trim().is_empty());
    if has_key {
        return true;
    }
    assert!(!required, "SMYSL_DEEPSEEK=required but ${KEY_VAR} is unset");
    eprintln!("skipping {what}: ${KEY_VAR} is unset");
    false
}

/// TLS, auth from the environment, request, response, and usage - the whole path a hosted
/// provider takes, against the real endpoint.
#[test]
fn a_completion_round_trips_over_tls() {
    if !available("completion") {
        return;
    }
    let req = Request::new("", "Reply with the single word: ok").with_max_output(8);
    let c = DeepSeek::new(cfg())
        .complete(&req)
        .expect("the live endpoint answers");

    assert!(!c.text.trim().is_empty());
    assert!(!c.usage.estimated, "the endpoint reports counts");
    assert!(c.usage.input_tokens > 0 && c.usage.output_tokens > 0);
    // The model that answered is not the model asked for: `deepseek-chat` resolves to a
    // dated build, and the ledger should record what ran.
    assert!(!c.model.is_empty());
}

/// **The case Ollama cannot exercise at all.** A rejected credential is `Unauthorized`,
/// which is not fallback-eligible: falling back would hide a wrong key behind another
/// model's answer.
#[test]
fn a_rejected_key_is_unauthorized_and_never_falls_back() {
    if !available("auth rejection") {
        return;
    }
    let var = "SMYSL_TEST_DEEPSEEK_BAD_KEY";
    std::env::set_var(var, "sk-definitely-not-a-valid-key");

    let mut bad = cfg();
    bad.id = ProviderId::new("bad").unwrap();
    bad.api_key_env = Some(var.into());

    let e = DeepSeek::new(bad.clone())
        .complete(&Request::new("", "hi").with_max_output(4))
        .expect_err("a bad key is rejected");
    assert_eq!(e, ProviderError::Unauthorized);
    assert!(!e.is_fallback_eligible());

    // And the registry honours that: the spare is never reached.
    let r = Registry::new()
        .with_provider(Box::new(DeepSeek::new(bad)))
        .with_provider(Box::new(DeepSeek::new(cfg())))
        .route(Task::Attest, ProviderId::new("bad").unwrap())
        .with_fallback([ProviderId::new("deepseek").unwrap()]);

    assert_eq!(
        r.complete(Task::Attest, &Request::new("", "hi").with_max_output(4))
            .unwrap_err(),
        ProviderError::Unauthorized
    );
    std::env::remove_var(var);
}

/// A key rejected at probe time is reported, not raised: `providers --probe` exists to say
/// what is wrong.
#[test]
fn a_probe_with_a_bad_key_reports_rather_than_fails() {
    if !available("probe") {
        return;
    }
    let var = "SMYSL_TEST_DEEPSEEK_BAD_PROBE";
    std::env::set_var(var, "sk-invalid");
    let mut bad = cfg();
    bad.api_key_env = Some(var.into());

    let probe = DeepSeek::new(bad).probe().expect("a probe reports");
    assert!(!probe.reachable);
    assert!(probe.detail.contains("credentials"), "{}", probe.detail);
    std::env::remove_var(var);
}

#[test]
fn a_probe_lists_the_live_models_and_reports_capabilities() {
    if !available("probe") {
        return;
    }
    let probe = DeepSeek::new(cfg()).probe().expect("a reachable endpoint");
    assert!(probe.reachable);
    assert!(!probe.models.is_empty(), "{probe:?}");

    let caps = probe.caps.expect("capabilities");
    assert!(!caps.offline, "a hosted provider is never offline-capable");
    // D-9 rests on this being reported honestly.
    assert_eq!(caps.structured, StructuredMode::JsonMode);
    assert!(!caps.structured.is_enforced());
}

/// `json_object` produces parseable JSON - and nothing more. That gap is exactly what E9
/// measures and what D-9 leaves open, so the test asserts the parse and *not* the shape.
#[test]
fn json_mode_yields_parseable_json_without_promising_a_shape() {
    if !available("json mode") {
        return;
    }
    let mut req = Request::new(
        "",
        "What colour is a clear midday sky? Reply as {\"answer\": string} in JSON.",
    )
    .with_system("Reply in JSON.")
    .with_max_output(48);
    req.structured = StructuredMode::JsonMode;

    let c = DeepSeek::new(cfg())
        .complete(&req)
        .expect("the live endpoint answers");

    assert!(
        !c.structured,
        "json_object is not enforcement, and must never be reported as such"
    );
    serde_json::from_str::<serde_json::Value>(c.text.trim())
        .unwrap_or_else(|e| panic!("json_object did not produce JSON: {e}\n{}", c.text));
}

/// `--offline` refuses a hosted provider before any I/O, against a provider that really is
/// hosted rather than a stand-in for one.
#[test]
fn offline_refuses_the_live_hosted_provider_without_a_call() {
    let r = Registry::new()
        .with_provider(Box::new(DeepSeek::new(cfg())))
        .route(Task::ContentIngest, ProviderId::new("deepseek").unwrap())
        .offline(true);

    assert_eq!(
        r.for_task(Task::ContentIngest).map(|_| ()).unwrap_err(),
        ProviderError::OfflineViolation
    );
    assert_eq!(
        ProviderError::OfflineViolation.exit_code(),
        smysl_core::ExitCode::Offline
    );

    let row = r
        .egress_report()
        .into_iter()
        .find(|e| e.task == Task::ContentIngest)
        .unwrap();
    assert!(row.leaves_machine, "a hosted provider egresses");
}
