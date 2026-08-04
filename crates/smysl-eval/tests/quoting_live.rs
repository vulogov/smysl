//! Does requiring a quote coarsen what a model extracts?
//!
//! ```text
//! SMYSL_EVAL_LIVE=1 cargo test -p smysl-eval --test quoting_live -- --ignored --nocapture
//! ```
//!
//! The question, and why it needed a design before a model
//! ------------------------------------------------------
//!
//! `ingest.content.json` tells the model to give each unit a `quote`: the span it came from,
//! copied exactly, checked against the document afterwards. The suspicion — carried since 0.7
//! and never tested — is that this **coarsens** the result: a model told to anchor every unit
//! to a verbatim span may produce fewer and blunter units than one free to state a claim in
//! its own words, because a fine-grained claim often has no single span to point at.
//!
//! Nothing could be measured because there were no two arms to compare. The quote requirement
//! is not a flag; it is a paragraph in a prompt. So the experiment's first cost was deciding
//! what the other arm *is*, and the answer taken here is the narrowest one available: the
//! shipped prompt with that paragraph removed and nothing else changed. Anything wider —
//! rewriting the instruction to be gentler, say — would measure the rewrite.
//!
//! What "coarsening" is taken to mean
//! ----------------------------------
//!
//! Four numbers, none of which is quality:
//!
//! * **units per document** — the direct reading of "fewer".
//! * **mean gist length** — "blunter" would show as longer, more hedged gists.
//! * **share carrying a body** — a unit with only a gist has no detail to lose later.
//! * **share carrying grounds or a relation** — structure is the thing the format is for, and
//!   a model spending its attention on span-matching may produce less of it.
//!
//! Deliberately not a quality judgement. Judging would need a judge, the judge would be a
//! model, and 0.8 established that an uncontrolled judge measures its own bias. These four are
//! counts, and counts are what a first pass can honestly produce.
//!
//! # Closed, negative
//!
//! **No detectable effect**, at n=6, on three fixtures, on two models. The suspicion carried
//! since 0.7 — that requiring a quote coarsens what a model extracts — has no support. Unit
//! counts overlap everywhere; so does everything else, in five of six fixture-model pairs.
//!
//! The single separation found (`struct%`, DeepSeek on F2) is what chance produces: six pairs
//! times four metrics is twenty-four comparisons a run, and it appeared in the second run
//! having been absent from the first.
//!
//! Two runs at n=6 are what settled it, and comparing them is the whole lesson. The `body%`
//! column looked like a large, consistent, cross-model effect in run one — 83 against 0, 100
//! against 0, 87 against 3 — and in run two the same fixtures gave 100/17, 83/83 and 53/0. The
//! gaps move more between runs than between arms. Within-run variance swamps the difference,
//! which is precisely what the range test exists to notice and precisely what reading a
//! column of means cannot.
//!
//! The harness is kept rather than deleted: it is how the question was answered, and the same
//! two arms would be needed to ask it again with more power or different metrics.
//!
//! Sample size is the whole difficulty. The 0.10 pilot ran each arm twice, and the between-arm
//! difference landed inside one arm's own run-to-run spread on every fixture of every model —
//! which is not evidence of no effect, it is an absence of power. `SMYSL_QUOTING_RUNS`
//! controls it, defaulting to six.
//!
//! The verdict is computed rather than eyeballed: a metric reports a difference only when the
//! two arms' ranges do not overlap. That is blunt, and blunt is right here — with samples this
//! small, anything finer would be arithmetic dressed as inference.
//!
//! **All four metrics, not just the unit count.** The first version tested units alone and
//! printed "no effect visible", which was true of units and false of the run: `body%` stood at
//! 83 against 0 on the same fixtures and the verdict said nothing, because it was not looking
//! there.

use std::collections::BTreeSet;

use smysl_eval::prose::to_prose;
use smysl_graph::Store;
use smysl_ingest::json_ast;
use smysl_ingest::prompt::{content_ingest_json, Template};
use smysl_provider::config::ProviderConfig;
use smysl_provider::{Provider, ProviderId, Request, StructuredMode};

struct Endpoint {
    kind: &'static str,
    model: &'static str,
    base: &'static str,
    key: &'static str,
    /// Per-endpoint, because the two differ. Gemini takes a schema; DeepSeek takes only
    /// "reply in JSON" and is documented that way in `READINESS.md`.
    mode: StructuredMode,
    /// Gemini's reasoning tokens are charged against this cap and are not in the answer, so a
    /// cap sized for the answer alone never finishes — the mapper's own module doc records a
    /// run that spent 1468 thought tokens on a 234-token reply. Sized for both halves here.
    max_output: usize,
}

const ENDPOINTS: &[Endpoint] = &[
    Endpoint {
        kind: "gemini",
        model: "gemini-3.5-flash-lite",
        base: "https://generativelanguage.googleapis.com",
        key: "GEMINI_API_KEY",
        mode: StructuredMode::JsonSchema,
        max_output: 32_768,
    },
    Endpoint {
        kind: "deepseek",
        model: "deepseek-chat",
        base: "https://api.deepseek.com",
        key: "DEEPSEEK_API_KEY",
        mode: StructuredMode::JsonMode,
        max_output: 8_192,
    },
];

const FIXTURES: &[&str] = &["F1-incident.smy", "F2-research.smy", "F4-qa.smy"];

/// The other arm: the shipped prompt with the quote paragraph removed, and nothing else.
///
/// Cut by locating the sentence rather than by rewriting the prompt, so this cannot drift into
/// measuring a different instruction if the shipped one is edited. If the sentence is not
/// found the test fails rather than silently comparing an arm against itself — which is the
/// shape of vacuity this project keeps finding.
fn without_quote_requirement() -> Template {
    let base = content_ingest_json();
    let start = base
        .system
        .find("Give each unit a `quote`")
        .expect("the quote instruction moved; this arm would otherwise equal the other");
    let rest = &base.system[start..];
    let end = rest
        .find("Where two units stand in a relation")
        .expect("the paragraph after the quote instruction moved");
    let mut system = String::with_capacity(base.system.len());
    system.push_str(&base.system[..start]);
    system.push_str(&rest[end..]);
    // Built from the shipped template and adjusted, not written out as a literal: `Template`
    // is `#[non_exhaustive]`, and more to the point a literal would drift from the thing under
    // test the moment a field is added to it.
    let mut t = base;
    t.id = "eval.ingest.content.json.noquote";
    t.system = system;
    t
}

#[derive(Default, Debug, Clone, Copy)]
struct Shape {
    units: usize,
    gist_chars: usize,
    with_body: usize,
    with_structure: usize,
}

impl Shape {
    fn mean_gist(&self) -> f64 {
        if self.units == 0 {
            0.0
        } else {
            self.gist_chars as f64 / self.units as f64
        }
    }
    fn pct(n: usize, d: usize) -> f64 {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    }
}

fn shape_of(raw: &str) -> Shape {
    let converted = json_ast::convert(raw);
    let mut s = Shape::default();
    let mut related: BTreeSet<String> = BTreeSet::new();
    for rel in &converted.relations {
        related.insert(rel.from.to_string());
        related.insert(rel.to.to_string());
    }
    for core in &converted.units {
        s.units += 1;
        s.gist_chars += core.gist.chars().count();
        if core.body.is_some() {
            s.with_body += 1;
        }
        let grounded = !core.grounds.is_empty() || !core.deps.is_empty();
        if grounded || related.contains(&smysl_core::canonical_uid(core).to_string()) {
            s.with_structure += 1;
        }
    }
    s
}

fn provider_for(e: &Endpoint) -> Option<Box<dyn Provider>> {
    if std::env::var(e.key).ok()?.trim().is_empty() {
        return None;
    }
    let mut cfg = ProviderConfig::new(ProviderId::new(e.kind).unwrap(), e.kind);
    cfg.endpoint = e.base.into();
    cfg.model = e.model.into();
    cfg.context_window = 65_536;
    cfg.max_output = e.max_output;
    cfg.timeout_secs = 120;
    cfg.api_key_env = Some(e.key.into());
    cfg.structured = e.mode;
    smysl_provider::map::build(&cfg).ok()
}

/// A named way of reading one number off a `Shape`.
type Metric = (&'static str, fn(&Shape) -> f64);

/// Runs per arm. Six by default; `SMYSL_QUOTING_RUNS` overrides it for a longer look.
fn runs() -> usize {
    std::env::var("SMYSL_QUOTING_RUNS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6)
}

fn corpus_prose(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus")
        .join(name);
    let text = std::fs::read_to_string(&path).expect("fixture");
    let parsed = smysl_core::surface::parse_surface(&text).expect("the fixture parses");
    let store = Store::from_records(parsed.records);
    to_prose(&store)
}

fn run_arm(
    p: &dyn Provider,
    t: &Template,
    prose: &str,
    mode: StructuredMode,
    cap: usize,
) -> Option<Shape> {
    // `with_max_output` and not just the config: the mapper sends *this* number as the
    // provider's cap, and the config's is only what the error used to quote — which is how
    // three runs were spent chasing "context window exceeded: 1008 > 32768".
    //
    // The schema goes wherever the provider will read it, exactly as `Ingestor::request` does.
    // This harness built its own request and so did not inherit that fix, and it showed:
    // DeepSeek returned zero units on every fixture of every arm, twice, which read as a
    // useless provider rather than a prompt naming a schema nobody had sent. An experiment
    // whose control arm is broken measures the breakage.
    let mut req = Request::new("", t.render(prose))
        .with_max_output(cap)
        .with_system(&t.system);
    if mode.is_enforced() {
        req = req.with_schema(mode, smysl_ingest::schema::batch_schema());
    } else {
        req = req.with_system(format!(
            "{}\n\nThe schema your object must match:\n{}",
            t.system,
            smysl_ingest::schema::batch_schema()
        ));
    }
    match p.complete(&req) {
        Ok(c) => Some(shape_of(&c.text)),
        Err(e) => {
            println!("    (call failed: {e})");
            None
        }
    }
}

#[test]
#[ignore = "live: needs a provider key and SMYSL_EVAL_LIVE=1"]
fn quote_requirement_versus_none() {
    if std::env::var("SMYSL_EVAL_LIVE")
        .unwrap_or_default()
        .is_empty()
    {
        println!("SMYSL_EVAL_LIVE is not set; not calling anybody");
        return;
    }
    let with = content_ingest_json();
    let without = without_quote_requirement();
    assert_ne!(
        with.system, without.system,
        "the two arms are identical, so this measures nothing"
    );
    assert!(
        !without.system.contains("quote"),
        "the no-quote arm still mentions quoting"
    );

    for e in ENDPOINTS {
        let Some(p) = provider_for(e) else {
            println!("\n{}: no key, skipped", e.kind);
            continue;
        };
        println!("\n=== {} / {} ===", e.kind, e.model);
        println!(
            "{:<18} {:>7} {:>7}  {:>8} {:>8}  {:>7} {:>7}  {:>7} {:>7}",
            "fixture",
            "n(q)",
            "n(-)",
            "gist(q)",
            "gist(-)",
            "body(q)",
            "body(-)",
            "str(q)",
            "str(-)"
        );
        for f in FIXTURES {
            let prose = corpus_prose(f);
            // `RUNS` per arm. Two was a pilot and could not see past the noise: the
            // between-arm difference landed inside one arm's own spread every time, which is
            // not "no effect" but "no power". Six is enough to separate an effect the size of
            // the spread from the spread itself, and cheap enough to actually run.
            let a: Vec<Shape> = (0..runs())
                .filter_map(|_| run_arm(&*p, &with, &prose, e.mode, e.max_output))
                .collect();
            let b: Vec<Shape> = (0..runs())
                .filter_map(|_| run_arm(&*p, &without, &prose, e.mode, e.max_output))
                .collect();
            if a.is_empty() || b.is_empty() {
                println!("{f:<18} (no usable runs)");
                continue;
            }
            let m = |v: &Vec<Shape>, f: fn(&Shape) -> f64| {
                v.iter().map(f).sum::<f64>() / v.len() as f64
            };
            println!(
                "{:<18} {:>7.1} {:>7.1}  {:>8.1} {:>8.1}  {:>6.0}% {:>6.0}%  {:>6.0}% {:>6.0}%",
                f,
                m(&a, |s| s.units as f64),
                m(&b, |s| s.units as f64),
                m(&a, |s| s.mean_gist()),
                m(&b, |s| s.mean_gist()),
                m(&a, |s| Shape::pct(s.with_body, s.units)),
                m(&b, |s| Shape::pct(s.with_body, s.units)),
                m(&a, |s| Shape::pct(s.with_structure, s.units)),
                m(&b, |s| Shape::pct(s.with_structure, s.units)),
            );
            // The verdict, stated per fixture rather than left to the reader's eye.
            //
            // A difference is reported only when the two arms' unit-count ranges do not
            // overlap at all. That is a blunt test and deliberately so: with samples this
            // small anything finer would be arithmetic dressed as inference, and the honest
            // question is whether the arms are even separable.
            // All four metrics, not just the unit count.
            //
            // The first version of this tested `units` alone and printed "no effect visible",
            // which was true of units and false of the run: the body column showed 83% against
            // 0% on the same fixtures, and the verdict line said nothing because it was not
            // looking there. A check answering a narrower question than its wording implies is
            // the defect this project keeps finding, and this one was mine.
            let lo = |u: &[f64]| u.iter().cloned().fold(f64::MAX, f64::min);
            let hi = |u: &[f64]| u.iter().cloned().fold(f64::MIN, f64::max);
            let metrics: [Metric; 4] = [
                ("units", |s| s.units as f64),
                ("gist", |s| s.mean_gist()),
                ("body%", |s| Shape::pct(s.with_body, s.units)),
                ("struct%", |s| Shape::pct(s.with_structure, s.units)),
            ];
            let mut verdicts: Vec<String> = Vec::new();
            for (name, get) in metrics {
                let ua: Vec<f64> = a.iter().map(get).collect();
                let ub: Vec<f64> = b.iter().map(get).collect();
                if hi(&ua) < lo(&ub) || hi(&ub) < lo(&ua) {
                    verdicts.push(format!(
                        "{name} SEPARATED (q {:.0}..{:.0} vs - {:.0}..{:.0})",
                        lo(&ua),
                        hi(&ua),
                        lo(&ub),
                        hi(&ub)
                    ));
                }
            }
            println!(
                "{:<18} n={}/{}  {}",
                "",
                a.len(),
                b.len(),
                if verdicts.is_empty() {
                    "no metric separates at this sample size".to_string()
                } else {
                    verdicts.join("; ")
                }
            );
        }
        println!("\n(q) = quote required, (-) = quote paragraph removed.");
        println!("A fixture reports a difference only if the two arms' ranges do not overlap.");
    }
}

/// Runs without a key, because the arm construction is the part that can silently break.
#[test]
fn the_two_arms_differ_only_in_the_quote_paragraph() {
    let with = content_ingest_json();
    let without = without_quote_requirement();

    assert!(with.system.contains("Give each unit a `quote`"));
    assert!(!without.system.contains("`quote`"));

    // Everything either side of the removed paragraph must be untouched, or the experiment is
    // comparing two prompts that differ in more than the thing under test.
    let head = &with.system[..with.system.find("Give each unit a `quote`").unwrap()];
    let tail = &with.system[with
        .system
        .find("Where two units stand in a relation")
        .unwrap()..];
    assert!(without.system.starts_with(head));
    assert!(without.system.ends_with(tail));
    assert_eq!(without.system.len(), head.len() + tail.len());
}
