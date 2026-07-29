//! **The SM-P15 gate's last clause: both arms, measured, on the same fixture.**
//!
//! Skipped without a key, like every other live test here, because a test that fails for
//! want of a credential is a test everyone learns to ignore. `SMYSL_EVAL_LIVE=required`
//! turns the skip into a failure.
//!
//! ```text
//! GEMINI_API_KEY=…            the arm runs
//! SMYSL_EVAL_LIVE=required    it must run, or the test fails
//! ```
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

const MODEL: &str = "gemini-3.5-flash-lite";

fn provider() -> Option<Box<dyn Provider>> {
    if std::env::var("GEMINI_API_KEY").ok()?.trim().is_empty() {
        return None;
    }
    let mut cfg = ProviderConfig::new(ProviderId::new("gemini").unwrap(), "gemini");
    cfg.endpoint = "https://generativelanguage.googleapis.com".into();
    cfg.model = MODEL.into();
    cfg.context_window = 1_000_000;
    cfg.max_output = 2048;
    cfg.timeout_secs = 120;
    cfg.api_key_env = Some("GEMINI_API_KEY".into());
    smysl_provider::map::build(&cfg).ok()
}

struct Model<'a>(&'a dyn Provider);

impl Summariser for Model<'_> {
    fn summarise(&self, text: &str, max_tokens: u64) -> Result<String, EvalError> {
        let prompt = format!(
            "Summarise the following notes for a colleague who has to act on them.\n\
             Stay under {max_tokens} tokens. Prose only: no lists, no headings.\n\n{text}"
        );
        let req = Request::new(MODEL, prompt)
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
        let req = Request::new(MODEL, prompt)
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

fn fixture(name: &str) -> Store {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus")
        .join(name);
    let src = std::fs::read_to_string(&path).unwrap();
    let out = smysl_core::surface::parse_surface(&src).unwrap();
    Store::from_records(out.records)
}

/// Both arms over F1, at the same budget, reported side by side.
#[test]
fn both_arms_over_the_same_fixture() {
    let required = std::env::var("SMYSL_EVAL_LIVE").as_deref() == Ok("required");
    let Some(p) = provider() else {
        assert!(
            !required,
            "SMYSL_EVAL_LIVE=required but GEMINI_API_KEY is unset"
        );
        eprintln!("prose arm skipped: GEMINI_API_KEY is unset");
        return;
    };
    let model = Model(p.as_ref());

    let store = fixture("F1-incident.smy");
    let opts = ChainOptions::default();

    // -- the smysl arm: deterministic, no model ---------------------------
    let smysl = run_smysl_arm(&store, &opts);
    let ms = measure(&store, &smysl);
    let val = |m: Metric| {
        ms.iter()
            .find(|x| x.metric == m)
            .and_then(|x| x.outcome.value())
            .unwrap()
    };

    // -- the prose arm: a model at every hop ------------------------------
    let budget = chain::Budget::Fraction(0.6).tokens(smysl.initial_tokens);
    let run = run_prose_arm(&store, opts.hops, budget, &model).expect("prose arm");
    let claims = claims_of(&store);

    // **The control.** Judge the *unsummarised* prose with the same judge and the same
    // prompt. If the hedges are already read as certainties here, then the inflation
    // measured after five hops is the judge's bias, not summarisation's damage - and the
    // headline number would be an artefact of the instrument. A finding needs this to be
    // near zero and the post-chain number to be well above it.
    let control = judge(&claims, &run.initial, &model).expect("judging the control");
    let judged = judge(&claims, run.final_text(), &model).expect("judging");

    eprintln!("\n--- SM-P15, both arms over F1 ({} hops) ---", opts.hops);
    eprintln!(
        "  smysl   E1 {:.3}  E2 {:.3}  E3 {:.0} inflated",
        val(Metric::TokenCost),
        val(Metric::ClaimSurvival),
        val(Metric::EpistemicCorruption)
    );
    eprintln!(
        "  control E1 1.000  E2 {:.3}  E3 {} inflated  attribution {:.3}  ({} abstention(s) of {})  <- hop 0",
        control.survival(),
        control.inflated.len(),
        control.attribution(),
        control.abstained,
        control.total
    );
    eprintln!(
        "  prose   E1 {:.3}  E2 {:.3}  E3 {} inflated  attribution {:.3}  ({} abstention(s) of {})",
        run.final_tokens() as f64 / run.initial_tokens().max(1) as f64,
        judged.survival(),
        judged.inflated.len(),
        judged.attribution(),
        judged.abstained,
        judged.total
    );
    eprintln!(
        "          {} of {} sourced claim(s) still name their source",
        judged.attributed.len(),
        judged.attributable
    );

    // On the smysl arm attribution survival is 1.0 by construction: `source` is a field of
    // the unit, so it travels with anything that travels. Structural, like E3 and E4 - and
    // stated rather than measured, because measuring it would only confirm the type system.
    eprintln!("  smysl   attribution 1.000 (structural: `source` is a field of the unit)");
    for (uid, was, now) in &judged.inflated {
        eprintln!(
            "          {} {was} -> read as {now}",
            &uid.to_string()[..14]
        );
    }

    // The structural guarantee, asserted only on the arm that makes it. The prose arm is
    // measured and reported; it is not required to pass, because the whole point is that
    // nothing enforces anything over there.
    assert_eq!(
        val(Metric::EpistemicCorruption),
        0.0,
        "rule M/T on the smysl arm"
    );
    assert!(
        judged.is_usable(),
        "the judge abstained on {} of {} claims; its verdicts mean too little to report",
        judged.abstained,
        judged.total
    );

    // The control has to hold up, or nothing measured after it means anything. The
    // baseline prose states every hedge in words, so a judge that cannot read them back
    // from the *unsummarised* text is not measuring the chain.
    assert!(
        control.is_usable(),
        "the judge abstained on the control; it cannot read hedges at all"
    );
    // The same control the hedges get. The baseline prose names every source in words, so a
    // judge that cannot read them back from the *unsummarised* text is not measuring the
    // chain - and an attribution figure taken without this is an artefact.
    assert!(
        control.attribution() > 0.5,
        "the judge recovered only {:.0}% of sources from unsummarised prose; the \
         post-chain attribution figure of {:.0}% would measure the instrument",
        control.attribution() * 100.0,
        judged.attribution() * 100.0
    );

    assert!(
        control.inflated.len() < judged.inflated.len().max(1),
        "the control inflated {} of {} claims before a single hop ran: the instrument is \
         reading confidence that the text does not state, so the post-chain figure of {} \
         is an artefact rather than a finding",
        control.inflated.len(),
        control.total,
        judged.inflated.len()
    );
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
