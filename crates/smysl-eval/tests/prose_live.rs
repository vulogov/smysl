//! **The SM-P15 gate's last clause: both arms, measured, on the same fixture.**
//!
//! **Opt-in, not key-triggered.** A credential in the environment is not consent to spend
//! it. This arm runs a model at five hops over five fixtures for every endpoint that has a
//! key, and then again to judge the result - minutes of wall clock and real money - so
//! having exported a key while doing something else must not be enough to start it.
//!
//! It used to be. `make test-matrix` therefore charged anyone with a key on their shell and
//! skipped silently in CI, which is the worst distribution of that cost: invisible where it
//! is watched, and automatic where it is not.
//!
//! ```text
//! (unset)                     skipped, whatever keys exist
//! SMYSL_EVAL_LIVE=1           runs for every endpoint that has a key
//! SMYSL_EVAL_LIVE=required    must run, or the test fails
//! ```
//!
//! A skip is still a skip rather than a failure, because a test that fails for want of a
//! credential is a test everyone learns to ignore.
//!
//! The judge is deliberately not told what the original claimed. It is shown a claim and a
//! passage and asked what the *passage* supports, so E3 measures the prose rather than the
//! judge's memory of the graph.

use smysl_eval::prose::{
    claims_of, judge, run_prose_arm, Claim, EvalError, Judge, Summariser, Verdict,
};
use smysl_eval::{chain, measure, run_smysl_arm, ChainOptions, Metric};
use smysl_graph::Store;
use smysl_provider::{config::ProviderConfig, Provider, ProviderId, Request};

/// The endpoints this runs against, and what each needs to be reachable.
///
/// Two models rather than one, because a single model's habits are indistinguishable from
/// the format's properties when there is nothing to compare them against. Two is not many,
/// and the report says so.
struct Endpoint {
    kind: &'static str,
    model: &'static str,
    base: &'static str,
    key: &'static str,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        kind: "gemini",
        model: "gemini-3.5-flash-lite",
        base: "https://generativelanguage.googleapis.com",
        key: "GEMINI_API_KEY",
    },
    Endpoint {
        kind: "deepseek",
        model: "deepseek-chat",
        base: "https://api.deepseek.com",
        key: "DEEPSEEK_API_KEY",
    },
];

/// The fixtures the baseline runs over.
///
/// F1-F5, which is what §28's chain names. F6 is adversarial and F7/F8 are merge fixtures,
/// so none of the three is a document a pipeline would hand on.
const FIXTURES: &[&str] = &[
    "F1-incident.smy",
    "F2-research.smy",
    "F3-narrative.smy",
    "F4-qa.smy",
    "F5-dataset.smy",
];

fn provider_for(e: &Endpoint) -> Option<Box<dyn Provider>> {
    if std::env::var(e.key).ok()?.trim().is_empty() {
        return None;
    }
    let mut cfg = ProviderConfig::new(ProviderId::new(e.kind).unwrap(), e.kind);
    cfg.endpoint = e.base.into();
    cfg.model = e.model.into();
    cfg.context_window = 65_536;
    cfg.max_output = 2048;
    cfg.timeout_secs = 120;
    cfg.api_key_env = Some(e.key.to_string());
    smysl_provider::map::build(&cfg).ok()
}

struct Model<'a>(&'a dyn Provider, &'static str);

impl Summariser for Model<'_> {
    fn summarise(&self, text: &str, max_tokens: u64) -> Result<String, EvalError> {
        let prompt = format!(
            "Summarise the following notes for a colleague who has to act on them.\n\
             Stay under {max_tokens} tokens. Prose only: no lists, no headings.\n\n{text}"
        );
        let req = Request::new(self.1, prompt)
            .with_system("You summarise technical notes faithfully and briefly.")
            .with_max_output((max_tokens.max(256) + 512) as usize);
        self.0
            .complete(&req)
            .map(|c| c.text)
            .map_err(|e| EvalError(e.to_string()))
    }
}

impl Judge for Model<'_> {
    fn verdict(&self, claim: &Claim, text: &str) -> Result<Verdict, EvalError> {
        // The claim's own status is *not* in the prompt. Telling the judge what the graph
        // said would let it answer from that instead of from the passage, and E3 would
        // measure the leak.
        let prompt = format!(
            "Does the PASSAGE state the CLAIM, and how confidently?\n\n\
             CLAIM: {}\n\nPASSAGE:\n{text}\n\n\
             Reply with exactly one line and nothing else:\n\
             ABSENT\n\
             or\n\
             PRESENT <level> | <source>\n\
             where <source> is what the PASSAGE says the claim rests on - a metric name, a \
             document, a file - copied as the passage names it, or NONE if the passage \
             gives none.\n\
             <level> is the strongest reading the PASSAGE supports:\n\
             speculative - a possibility, guess or hypothesis\n\
             inferred    - reasoned from other statements\n\
             derived     - follows from evidence stated in the passage\n\
             cited       - attributed to a named source\n\
             measured    - stated flatly as observation or measurement, unhedged",
            claim.gist
        );
        let req = Request::new(self.1, prompt)
            .with_system("You are a precise reader. You answer in the exact format asked.")
            .with_max_output(1024usize);

        let answer = self
            .0
            .complete(&req)
            .map(|c| c.text)
            .map_err(|e| EvalError(e.to_string()))?;
        Ok(parse_verdict(&answer))
    }
}

/// Lenient on shape, strict on meaning. An unparseable answer abstains rather than being
/// guessed at, and abstentions are counted so a bad judge is visible as a bad judge.
fn parse_verdict(answer: &str) -> Verdict {
    let line = answer
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_ascii_lowercase();

    if line.starts_with("absent") {
        return Verdict::absent();
    }
    if !line.starts_with("present") {
        return Verdict::absent();
    }
    // `PRESENT <level> | <source>`. The level is looked for in the part before the bar so a
    // source that happens to contain the word "measured" cannot be read as the level.
    let (head, tail) = match line.split_once('|') {
        Some((h, t)) => (h, Some(t.trim())),
        None => (line.as_str(), None),
    };
    let status = ["measured", "cited", "derived", "inferred", "speculative"]
        .iter()
        .find(|s| head.contains(**s))
        .and_then(|s| smysl_core::Status::parse(s));

    let mut v = Verdict::absent();
    v.present = true;
    v.as_stated = status;
    v.attributed_to = tail
        .filter(|t| !t.is_empty() && *t != "none")
        .map(str::to_string);
    v
}

fn fixture_store(name: &str) -> Store {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus")
        .join(name);
    let src = std::fs::read_to_string(&path).unwrap();
    let out = smysl_core::surface::parse_surface(&src).unwrap();
    Store::from_records(out.records)
}

/// One row of the matrix.
struct Row {
    fixture: &'static str,
    model: &'static str,
    e1: f64,
    e2: f64,
    inflated: usize,
    attribution: f64,
    control_inflated: usize,
    control_attribution: f64,
    attributable: usize,
    abstained: usize,
    units: usize,
    /// The smysl arm's token cost on the same fixture and budget, so the two are
    /// comparable in one row rather than in two tables.
    smysl_e1: f64,
}

/// **Both arms, every fixture the chain names, every endpoint reachable.**
///
/// The smysl arm already ran over the whole corpus; the baseline ran over one fixture with
/// one model, and every number published about this comparison rested on that. One model's
/// habits are indistinguishable from the format's properties when there is nothing to
/// compare them against.
#[test]
fn both_arms_over_every_fixture_and_endpoint() {
    let gate = std::env::var("SMYSL_EVAL_LIVE").unwrap_or_default();
    let required = gate == "required";
    if gate.trim().is_empty() {
        eprintln!("prose arm skipped: set SMYSL_EVAL_LIVE=1 to spend tokens on it");
        return;
    }

    let live: Vec<(&Endpoint, Box<dyn Provider>)> = ENDPOINTS
        .iter()
        .filter_map(|e| provider_for(e).map(|p| (e, p)))
        .collect();
    if live.is_empty() {
        assert!(
            !required,
            "SMYSL_EVAL_LIVE=required but no endpoint has a key"
        );
        eprintln!("prose arm skipped: no endpoint has a key");
        return;
    }

    let mut rows = Vec::new();
    let mut failures = Vec::new();

    for (endpoint, provider) in &live {
        let model = Model(provider.as_ref(), endpoint.model);
        for fixture in FIXTURES {
            let store = fixture_store(fixture);
            let opts = ChainOptions::default();

            // The smysl arm: deterministic, no model, no cost.
            let smysl = run_smysl_arm(&store, &opts);
            let ms = measure(&store, &smysl);
            let val = |m: Metric| {
                ms.iter()
                    .find(|x| x.metric == m)
                    .and_then(|x| x.outcome.value())
                    .unwrap_or(f64::NAN)
            };
            assert_eq!(
                val(Metric::EpistemicCorruption),
                0.0,
                "{fixture}: rule M/T on the smysl arm"
            );

            // The prose arm, at the same budget so E1 compares like with like.
            let budget = chain::Budget::Fraction(0.6).tokens(smysl.initial_tokens);
            let claims = claims_of(&store);
            let run = match run_prose_arm(&store, opts.hops, budget, &model) {
                Ok(r) => r,
                Err(e) => {
                    // A provider failure is not a result. Recorded and skipped, never
                    // folded into the numbers as if the chain had simply lost things.
                    failures.push(format!("{} on {}: {e}", endpoint.model, fixture));
                    continue;
                }
            };
            let (control, judged) = match (
                judge(&claims, &run.initial, &model),
                judge(&claims, run.final_text(), &model),
            ) {
                (Ok(c), Ok(j)) => (c, j),
                _ => {
                    failures.push(format!("{} on {}: judging failed", endpoint.model, fixture));
                    continue;
                }
            };

            rows.push(Row {
                fixture,
                model: endpoint.model,
                e1: run.final_tokens() as f64 / run.initial_tokens().max(1) as f64,
                e2: judged.survival(),
                inflated: judged.inflated.len(),
                attribution: judged.attribution(),
                control_inflated: control.inflated.len(),
                control_attribution: control.attribution(),
                attributable: judged.attributable,
                abstained: judged.abstained,
                units: claims.len(),
                smysl_e1: val(Metric::TokenCost),
            });
        }
    }

    report(&rows, &failures);

    assert!(!rows.is_empty(), "no row completed");
    // Every row's control must clear, or that row's numbers describe the judge.
    for r in &rows {
        assert_eq!(
            r.control_inflated, 0,
            "{} on {}: the judge read {} hedge(s) as inflated before any summarisation",
            r.model, r.fixture, r.control_inflated
        );
        assert!(
            r.attributable == 0 || r.control_attribution > 0.5,
            "{} on {}: the judge recovered only {:.0}% of sources from unsummarised prose",
            r.model,
            r.fixture,
            r.control_attribution * 100.0
        );
    }
}

fn report(rows: &[Row], failures: &[String]) {
    eprintln!("\n--- both arms, {} row(s) ---", rows.len());
    eprintln!(
        "  {:<16} {:<22} {:>6} {:>6} {:>6} {:>9} {:>12}",
        "fixture", "model", "E1", "smysl", "E2", "hedges", "sources"
    );
    for r in rows {
        eprintln!(
            "  {:<16} {:<22} {:>6.3} {:>6.3} {:>6.3} {:>4}/{:<4} {:>6}/{:<5}",
            r.fixture.trim_end_matches(".smy"),
            r.model,
            r.e1,
            r.smysl_e1,
            r.e2,
            r.inflated,
            r.units,
            (r.attribution * r.attributable as f64).round() as usize,
            r.attributable
        );
    }

    // Totals, which is the only place a claim about "the prose baseline" can honestly be
    // read off: a mean over five fixtures says more than any single one of them.
    let n = rows.len() as f64;
    let hedges: usize = rows.iter().map(|r| r.inflated).sum();
    let units: usize = rows.iter().map(|r| r.units).sum();
    let kept: usize = rows
        .iter()
        .map(|r| (r.attribution * r.attributable as f64).round() as usize)
        .sum();
    let sourced: usize = rows.iter().map(|r| r.attributable).sum();
    eprintln!(
        "\n  mean E1 {:.3}   mean E2 {:.3}   hedges lost {hedges}/{units}   sources kept {kept}/{sourced}",
        rows.iter().map(|r| r.e1).sum::<f64>() / n,
        rows.iter().map(|r| r.e2).sum::<f64>() / n,
    );
    let abstained: usize = rows.iter().map(|r| r.abstained).sum();
    eprintln!("  judge abstained on {abstained} of {units} claim(s)");
    eprintln!("  smysl arm: hedges lost 0/{units}, sources kept {sourced}/{sourced} (structural)");
    for f in failures {
        eprintln!("  FAILED  {f}");
    }
}

/// The ports are ports: this crate must not have grown a provider dependency.
#[test]
fn the_eval_crate_itself_links_no_provider() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .unwrap();
    let deps = manifest
        .split("[dependencies]")
        .nth(1)
        .expect("a dependencies section");
    assert!(
        !deps.contains("smysl-provider"),
        "smysl-provider must stay a dev-dependency, or `Summariser` stops being a port"
    );
}
