//! The SM-P14 gate's first clause: **the same ingest fixture yields conformant units on all
//! five providers.**
//!
//! One fixture, one assertion, run against whichever providers this machine can reach. A
//! provider with no key or no server is skipped rather than failed - a test that failed for
//! want of a credential would be a test everyone learns to ignore.
//!
//! ```text
//! SMYSL_OLLAMA=required   ollama must be reachable
//! (unset)                 skipped, whatever keys exist
//! SMYSL_INGEST_LIVE=1     every provider with a key runs
//! SMYSL_INGEST_LIVE=all   every provider above must run, or the test fails
//! SMYSL_ANTHROPIC_MODEL   the Anthropic model the gate runs (default claude-haiku-5-5)
//! SMYSL_ANTHROPIC_MATRIX=1  the fixture on every current Anthropic model, in turn
//! SMYSL_ANTHROPIC_EFFORT   low, medium or high, for the Anthropic candidate (default: the mapper's)
//! SMYSL_ANTHROPIC_HEADROOM=1 every model at every effort over four passages; the thinking share
//! ```
//!
//! The Anthropic default is the cheap model, so the gate is affordable to run routinely; the
//! matrix is for the per-model differences a single model hides — the forced tool choice
//! that one generation refused and another took, for instance.
//!
//! Opt-in rather than key-triggered: a credential in the environment is not consent to
//! spend it. A key alone used to be enough, which charged anyone running the test suite
//! with one exported and skipped silently in CI, where nobody could see what was missing.
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
    model: String,
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
            model: "llama3.2".into(),
            key_var: None,
            structured: StructuredMode::JsonSchema,
            context_window: 8192,
        },
        Candidate {
            id: "deepseek",
            kind: "deepseek",
            endpoint: "https://api.deepseek.com",
            model: "deepseek-chat".into(),
            key_var: Some("DEEPSEEK_API_KEY"),
            structured: StructuredMode::JsonMode,
            context_window: 65536,
        },
        Candidate {
            id: "anthropic",
            kind: "anthropic",
            endpoint: "https://api.anthropic.com",
            model: std::env::var("SMYSL_ANTHROPIC_MODEL")
                .ok()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| "claude-haiku-5-5".to_string()),
            key_var: Some("ANTHROPIC_API_KEY"),
            structured: StructuredMode::ToolForce,
            context_window: 1_000_000,
        },
        Candidate {
            id: "openai",
            kind: "openai",
            endpoint: "https://api.openai.com",
            model: "gpt-4.1-mini".into(),
            key_var: Some("OPENAI_API_KEY"),
            structured: StructuredMode::JsonSchema,
            context_window: 128_000,
        },
        // `gemini-2.5-flash` is listed by the probe and refused by the endpoint - "no
        // longer available to new users". A model list is a catalogue, not an entitlement,
        // which is worth knowing before trusting `providers --probe` to mean can-call.
        Candidate {
            id: "gemini",
            kind: "gemini",
            endpoint: "https://generativelanguage.googleapis.com",
            model: "gemini-3.5-flash-lite".into(),
            key_var: Some("GEMINI_API_KEY"),
            structured: StructuredMode::JsonSchema,
            context_window: 1_000_000,
        },
    ]
}

fn config(c: &Candidate) -> ProviderConfig {
    let mut cfg = ProviderConfig::new(ProviderId::new(c.id).unwrap(), c.kind);
    cfg.endpoint = c.endpoint.into();
    cfg.model = c.model.clone();
    cfg.structured = c.structured;
    cfg.context_window = c.context_window;
    cfg.max_output = 800;
    cfg.timeout_secs = 120;
    cfg.api_key_env = c.key_var.map(str::to_string);
    // `SMYSL_ANTHROPIC_EFFORT=low|medium|high` is how the effort measurement is run: the
    // matrix at each setting, and the record compares them.
    if c.kind == "anthropic" {
        cfg.effort = std::env::var("SMYSL_ANTHROPIC_EFFORT")
            .ok()
            .filter(|e| matches!(e.as_str(), "low" | "medium" | "high"));
    }
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
    let gate = std::env::var("SMYSL_INGEST_LIVE").unwrap_or_default();
    if gate.trim().is_empty() {
        eprintln!("ingest live gate skipped: set SMYSL_INGEST_LIVE=1 to call real providers");
        return;
    }
    let require_all = gate == "all";
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
            .with_model(&c.model)
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

/// Every current Anthropic model the list names, by family, cheapest first.
const ANTHROPIC_MATRIX: &[&str] = &["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"];

/// What one model did with the fixture, for the matrix's record.
fn run_fixture(c: &Candidate) -> Result<String, String> {
    let provider = usable(c)?;
    let id = provider.id();
    let registry = Registry::new()
        .with_provider(provider)
        .route(Task::ContentIngest, id);
    let opts = IngestOptions::at_rung(Rung::Document)
        .with_model(&c.model)
        .with_max_output(800);
    let (staged, report) = Ingestor::new(&registry, opts)
        .ingest(&Store::new(), FIXTURE)
        .map_err(|e| format!("ingest failed: {e}"))?;
    if staged.report.fail_on(Severity::Error).is_err() {
        return Err("staged units do not check".to_string());
    }
    Ok(format!(
        "{} unit(s), {} call(s), {} degraded, {} repaired, {} token(s), {} thinking, model {}",
        staged.len(),
        report.calls,
        report.degraded,
        report.repaired,
        report.usage.total(),
        report.usage.thinking_tokens,
        report.model.as_deref().unwrap_or("?")
    ))
}

/// **The Anthropic matrix.** The same fixture on every current model, one after another;
/// each must stage conformant units, and the record says what each cost. Opt-in with
/// `SMYSL_ANTHROPIC_MATRIX=1`, since it is three paid calls at least.
#[test]
fn every_current_anthropic_model_stages_conformant_units() {
    if std::env::var("SMYSL_ANTHROPIC_MATRIX")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        eprintln!("anthropic matrix skipped: set SMYSL_ANTHROPIC_MATRIX=1");
        return;
    }
    let base = candidates()
        .into_iter()
        .find(|c| c.kind == "anthropic")
        .expect("the anthropic candidate");
    let mut failures = Vec::new();
    eprintln!("--- Anthropic matrix ---");
    for model in ANTHROPIC_MATRIX {
        let c = Candidate {
            model: model.to_string(),
            ..base
        };
        match run_fixture(&c) {
            Ok(line) => eprintln!("  ok      {model}: {line}"),
            Err(why) => {
                eprintln!("  FAILED  {model}: {why}");
                failures.push(format!("{model}: {why}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A longer passage for the headroom measurement: the fixture's incident told in full, so the
/// answer has more to say and the reasoning more to weigh.
const REPORT: &str = "\
On Thursday at 09:12 UTC the eu-west shard slowed. The p95 request latency rose from 180ms to \
410ms over eleven minutes and stayed there until 10:40, when it fell back without anyone \
changing anything.

Connection pool wait time rose alongside the latency, from under a millisecond to 90ms at the \
peak. The pool was at its configured ceiling of 64 connections for the whole window, which \
suggests saturation as the cause: requests queued for a connection before they could run.

The canary shard, which runs the same build against a replica of the same database with a \
pool of the same size, stayed clean throughout. Its latency did not move and its pool never \
filled. The pool theory does not explain that, since the canary took the same traffic shape \
at a tenth of the volume.

A second reading is that the database itself was slow: a vacuum ran on the orders table from \
09:10 to 10:38 on the primary, and the canary reads from a replica that was not vacuuming. \
That would put the pool saturation downstream of the cause rather than at it.

Nobody has yet compared the vacuum's timing against the latency curve minute by minute, and \
until that is done neither reading is more than a suggestion.";

/// Source code, for the headroom measurement: a provider's reading of this is a different
/// shape of answer, and the tokeniser counts it differently.
const CODE: &str = "\
/// Waits for a connection, or gives up after `wait` and reports the pool as saturated.
pub fn acquire(pool: &Pool, wait: Duration) -> Result<Conn, PoolError> {
    let start = Instant::now();
    loop {
        if let Some(conn) = pool.try_take() {
            pool.metrics.record_wait(start.elapsed());
            return Ok(conn);
        }
        if start.elapsed() >= wait {
            pool.metrics.saturated.fetch_add(1, Ordering::Relaxed);
            return Err(PoolError::Saturated { size: pool.size(), waited: wait });
        }
        // The canary never reaches this branch: its pool is the same size but its traffic is
        // a tenth of the shard's, so a connection is always free. The shard reached it 2,140
        // times on Thursday between 09:12 and 10:40.
        std::thread::sleep(Duration::from_millis(5));
    }
}";

/// Cyrillic prose, for the headroom measurement: two-byte letters, which the estimator
/// over-counts and the model answers in its own script.
const RUSSIAN: &str = "\
В четверг сегмент eu-west замедлился: задержка p95 выросла со 180 до 410 миллисекунд за \
одиннадцать минут и держалась до 10:40, после чего вернулась к норме сама.

Время ожидания соединения из пула росло вместе с задержкой, и пул весь этот час был заполнен \
до предела в 64 соединения. Это указывает на насыщение пула как на причину.

Канареечный сегмент, который работает на той же сборке с пулом того же размера, оставался \
чистым всё время. Теория насыщения пула этого не объясняет.

Второе прочтение: медленной была сама база. С 09:10 до 10:38 на основном узле шла очистка \
таблицы заказов, а канарейка читает с реплики, где очистки не было.";

/// The passages the headroom measurement runs, by name.
const HEADROOM_PASSAGES: &[(&str, &str)] = &[
    ("fixture", FIXTURE),
    ("report", REPORT),
    ("code", CODE),
    ("russian", RUSSIAN),
];

/// What one call did with one passage, for the headroom record.
struct HeadroomCell {
    units: usize,
    output: u64,
    thinking: u64,
    degraded: usize,
    unanswered: bool,
    truncated: bool,
}

/// One ingest of `text` on `model` at `effort` with `max_output`, and what the ledger saw.
fn run_passage(
    base: &Candidate,
    model: &str,
    effort: &str,
    max_output: usize,
    text: &str,
) -> Result<HeadroomCell, String> {
    let c = Candidate {
        id: base.id,
        kind: base.kind,
        endpoint: base.endpoint,
        model: model.to_string(),
        key_var: base.key_var,
        structured: base.structured,
        context_window: base.context_window,
    };
    let mut cfg = config(&c);
    cfg.effort = Some(effort.to_string());
    cfg.max_output = max_output;
    cfg.timeout_secs = 300;
    let provider = smysl_provider::map::build(&cfg).map_err(|e| e.to_string())?;
    let id = provider.id();
    let registry = Registry::new()
        .with_provider(provider)
        .route(Task::ContentIngest, id);
    let opts = IngestOptions::at_rung(Rung::Document)
        .with_model(model)
        .with_max_output(max_output);
    let (staged, report) = Ingestor::new(&registry, opts)
        .ingest(&Store::new(), text)
        .map_err(|e| format!("ingest failed: {e}"))?;
    let has = |suffix: &str| {
        report
            .diagnostics
            .iter()
            .any(|d| d.code.as_str().ends_with(suffix))
    };
    Ok(HeadroomCell {
        units: staged.len(),
        output: report.usage.output_tokens,
        thinking: report.usage.thinking_tokens,
        degraded: report.degraded,
        unanswered: has("W455"),
        truncated: has("W304"),
    })
}

/// **The reasoning headroom across the matrix.** [`REASONING_HEADROOM_RATIO`] is a quarter
/// of `max_output` on the strength of one case, Haiku at `high` on the fixture. This runs
/// every current model at every effort over four passages — the fixture, a longer report,
/// source code and Cyrillic prose — at the gate's 800-token limit, and the `high` effort
/// again at 2,048, and prints what each call spent thinking as a share of its limit. The
/// record, not an assertion, is the product; the one assertion is that every call was
/// answered by the endpoint, since a transport failure would be a hole in the table. Opt-in
/// with `SMYSL_ANTHROPIC_HEADROOM=1`: it is 48 paid calls.
///
/// [`REASONING_HEADROOM_RATIO`]: smysl_ingest::chunk::REASONING_HEADROOM_RATIO
#[test]
fn the_reasoning_headroom_is_measured_across_the_matrix() {
    if std::env::var("SMYSL_ANTHROPIC_HEADROOM")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        eprintln!("anthropic headroom skipped: set SMYSL_ANTHROPIC_HEADROOM=1");
        return;
    }
    let base = candidates()
        .into_iter()
        .find(|c| c.kind == "anthropic")
        .expect("the anthropic candidate");
    if let Err(why) = usable(&base) {
        eprintln!("anthropic headroom skipped: {why}");
        return;
    }
    let mut cells: Vec<(&str, usize)> = Vec::new();
    for effort in ["low", "medium", "high"] {
        cells.push((effort, 800));
    }
    cells.push(("high", 2048));
    let mut failures = Vec::new();
    eprintln!("--- Anthropic reasoning headroom (thinking as a share of max_output) ---");
    eprintln!("  model            effort   max passage  units output  think share  note");
    for model in ANTHROPIC_MATRIX {
        for (effort, max_output) in &cells {
            for (name, text) in HEADROOM_PASSAGES {
                match run_passage(&base, model, effort, *max_output, text) {
                    Ok(cell) => {
                        let share = cell.thinking as f64 / *max_output as f64;
                        let note = if cell.unanswered {
                            "W455 unanswered"
                        } else if cell.truncated {
                            "W304 truncated"
                        } else if cell.degraded > 0 {
                            "degraded"
                        } else {
                            ""
                        };
                        eprintln!(
                            "  {model:<16} {effort:<6} {max_output:>5} {name:<8} {:>5} {:>6} {:>6} {share:>5.2}  {note}",
                            cell.units, cell.output, cell.thinking
                        );
                    }
                    Err(why) => {
                        eprintln!(
                            "  {model:<16} {effort:<6} {max_output:>5} {name:<8} FAILED {why}"
                        );
                        failures.push(format!("{model} {effort} {max_output} {name}: {why}"));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// **The estimator against the tokeniser.** `smysl_core::tokens` is `ceil(bytes / 4)` by
/// design (rule D: a chunking is pure, and no tokeniser is available offline); this says how
/// far from the endpoint's own count that is on prose, so the chunk sizing knows its error.
/// The endpoint counts a whole message, so an empty-message baseline is subtracted. Opt-in
/// with `SMYSL_ANTHROPIC_CALIBRATE=1`; the counting endpoint is free, but it sends the text.
///
/// Gated on the feature rather than decided at run time like the others, because it reaches
/// the mapper's own type for the counting call, and a build without the mapper has no type.
#[cfg(feature = "anthropic")]
#[test]
fn the_estimator_is_calibrated_against_the_count_tokens_endpoint() {
    if std::env::var("SMYSL_ANTHROPIC_CALIBRATE")
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        eprintln!("estimator calibration skipped: set SMYSL_ANTHROPIC_CALIBRATE=1");
        return;
    }
    let c = candidates()
        .into_iter()
        .find(|c| c.kind == "anthropic")
        .expect("the anthropic candidate");
    if usable(&c).is_err() {
        eprintln!("estimator calibration skipped: anthropic is not usable here");
        return;
    }
    let anthropic = smysl_provider::map::anthropic::Anthropic::new(config(&c));
    let baseline = anthropic.count_tokens_remote("x").expect("baseline") - 1;
    let samples: &[(&str, &str)] = &[
        ("fixture", FIXTURE),
        (
            "prose",
            "The committee met on the fourth of March to consider the report, which ran to \
             ninety pages and recommended, among other things, that the harbour works be \
             postponed until the survey of the eastern channel was complete. Two members \
             dissented, on the ground that the survey had already been postponed twice.",
        ),
        (
            "code",
            "fn tokens(text: &str) -> u32 {\n    (text.len() as u32).div_ceil(4)\n}\n\n\
             pub fn chunk(input: &str, window: Window) -> Vec<Chunk> {\n    let mut out = \
             Vec::new();\n    for (i, p) in input.split(\"\\n\\n\").enumerate() {\n        \
             out.push(piece(p, i));\n    }\n    out\n}",
        ),
        (
            "cyrillic",
            "Комитет собрался четвёртого марта, чтобы рассмотреть отчёт, который занимал \
             девяносто страниц и рекомендовал отложить работы в гавани до завершения съёмки \
             восточного канала.",
        ),
    ];
    eprintln!(
        "--- estimator calibration (ceil(bytes/4) and the sizing estimate against count_tokens) ---"
    );
    let mut worst = 0.0f64;
    let mut worst_sizing = 0.0f64;
    for (name, text) in samples {
        let counted = anthropic
            .count_tokens_remote(text)
            .expect("count_tokens")
            .saturating_sub(baseline)
            .max(1);
        let estimated = smysl_core::tokens(text) as u64;
        let sizing = smysl_ingest::sizing_tokens(text) as u64;
        let ratio = estimated as f64 / counted as f64;
        let sizing_ratio = sizing as f64 / counted as f64;
        worst = worst.max((ratio - 1.0).abs());
        worst_sizing = worst_sizing.max((sizing_ratio - 1.0).abs());
        eprintln!(
            "  {name:<9} bytes {:>5}  estimated {estimated:>5}  counted {counted:>5}  ratio {ratio:.2}  sizing {sizing:>5} ({sizing_ratio:.2})",
            text.len()
        );
    }
    eprintln!(
        "  worst deviation: format count {:.0}%, sizing estimate {:.0}%",
        worst * 100.0,
        worst_sizing * 100.0
    );
}
