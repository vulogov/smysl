//! **GE-T2, rerun under the consensus policy** (SMYSL-2.4 §5.5, §6 TX-P5 step 5).
//!
//! Draft 1 measured extraction stability as `J_class` — a judge deciding whether two extractions
//! said the same thing — and S0 measured the judges themselves at α 0.600 binary and 0.330 over
//! six labels. A stability figure computed against a gold that unreliable is a figure about the
//! gold. So §1.1 item 2 narrowed the claim to the one relation that needs no judge:
//! **`identical-span` agreement** between the two passes of one window — same tid, same span, same
//! gist — computed, not judged, and reported **per language**.
//!
//! This harness has two arms, and the division is what GE-T2 can and cannot be answered by.
//!
//! # The instrument, offline
//!
//! Whether the figure is *arithmetically* right is a property of this code: that two passes which
//! found the same places report 1.0, that a drifting pass reports less, that units with no span
//! are excluded rather than counted as disagreement, and that a single-pass run reports **no
//! figure** rather than perfect stability. A scripted provider pins each of those exactly, in two
//! languages, because the grouping is per language and a harness that only ever ran English would
//! not have exercised one.
//!
//! # The measurement, which is paid
//!
//! What GE-T2 actually asks — how stable a *model* is over a corpus — cannot be answered by a
//! provider that says what the test told it to say. It needs egress and it needs a corpus.
//!
//! ```text
//! SMYSL_INGEST_LIVE=1        consent to spend a credential (as `providers_live.rs`)
//! SMYSL_GE_T2=<dir>          another corpus: <dir>/<lang>.txt, one file per language
//! SMYSL_GE_T2_MODEL=<name>   another model, in place of the provider's default
//! ```
//!
//! The corpus is `fixtures/stability/corpus/`, and the development-set objection is answered the
//! way GE-T13's chronology and step 3b's probe set answered it — by removing the authored choice
//! rather than defending it. The passages are the S0 spike's, fixed in SMYSL-2.2 before this code
//! existed, and the rule that selects three of its eleven is mechanical: every reference all five
//! tier-1 languages carry. That leaves 26 verses of narrative, 26 of law and 7 of poetry in each
//! language, 59 lines in every file, so the only thing varying between five reported figures is
//! the language. See that directory's README for provenance and hashes.
//!
//! The figure is reported, never thresholded: §5.5 gives the computed form no bar, because the
//! bar it had (0.6 on `J_class`) belongs to the judged form and that waits for GE-T9's human α.
//!
//! # What it measured, and why one run is not an estimate
//!
//! Three runs of all five languages against hosted `deepseek-chat` at temperature 0. Span
//! coverage was **1.000** in every language of every run — every unit extracted got a span — and
//! the `identical-span` rate was:
//!
//! ```text
//! lang     run A   run B   run C   spread
//! de       0.024   0.412   0.078    0.388
//! en       0.452   0.769   0.132    0.637
//! es       0.000   0.333   0.047    0.333
//! fr       0.047   0.024   0.391    0.367
//! ru       0.265   0.481   0.030    0.452
//! ```
//!
//! Re-running one language moves the figure by 0.333 to 0.637. Changing the language inside one
//! run moves it by 0.361 to 0.745. The two are the same size, and the ranking does not survive a
//! rerun — the best language is en, then en, then fr, and the worst is es, then fr, then ru. So
//! **the per-language figure §5.5 asks for is not resolved by one run per language**, and a table
//! of five numbers taken from a single run would have been a table of sampling noise. That is the
//! measurement's own finding about the measurement, and it is why the figures above are reported
//! as a spread over three runs rather than as five numbers.
//!
//! This is consistent with what S0 already had — `J_class` 1.000 local against 0.327 hosted, and
//! MS-8's 145 bit-identical runs, which were the **local** arm. Temperature is not the
//! explanation: `IngestOptions` defaults it to 0, `openai_compat::body` puts `"temperature": 0.0`
//! on the wire, and [`Passes::admits`] refuses anything above it. A hosted model is simply not
//! reproducible at temperature 0, and `admits` cannot see that, because it checks the temperature
//! a caller configured and not whether the provider honours it.
//!
//! # Reading the figure at all
//!
//! `identical-span` includes the **gist** (§3.5), so a pass that found the same bytes and wrote a
//! different sentence about them counts as a disagreement. Over a live corpus that is most of the
//! figure: French run B is 0.024 by `identical-span` and 0.680 by
//! [`smysl_ingest::Agreement::place_rate`], the same comparison with the gist left out — the two
//! passes found the same 17 of 25 places and re-worded nearly all of them. The live arm reports
//! both, because the reported figure alone does not say whether the instability is in the finding
//! or only in the wording, and the two have different remedies. A scripted provider cannot tell
//! them apart: it returns one string by construction, so its passes agree on the wording free.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_core::Rung;
use smysl_ingest::text::{Target, TextIngest, TextReport};
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    config::ProviderConfig, Capabilities, Completion, Probe, Provider, ProviderError, ProviderId,
    Registry, Request, StructuredMode, Task, Usage,
};
use smysl_text::reading::Level;
use smysl_text::{AddSpec, Caps, Input, Library};

/// Two lines of English, each past the window floor so each is its own window.
fn english() -> (String, [String; 2]) {
    let quotes = [
        "the pool wait rose to 1.9 seconds".to_string(),
        "the rollback returned p99 to baseline".to_string(),
    ];
    let mut text = String::new();
    for q in &quotes {
        text.push_str(q);
        // Sized so that a window of this corpus is comfortably under `SMALL_OUTPUT_TOKENS`,
        // in **both** languages: the path is chosen from a window's token count, and a corpus
        // that straddles the bound would take one path here and another there.
        for _ in 0..7 {
            text.push_str(
                " and the operators watched the number for eleven minutes before the page went \
                 out and then watched it again afterwards to be sure of what it had said",
            );
        }
        text.push('\n');
    }
    (text, quotes)
}

/// The same shape in Russian, because the figure is reported per language and a harness that
/// never saw a second script would not have tested the grouping, the normaliser or the estimator
/// on one.
fn russian() -> (String, [String; 2]) {
    let quotes = [
        "время ожидания в пуле выросло до 1,9 секунды".to_string(),
        "откат вернул p99 к исходному уровню".to_string(),
    ];
    let mut text = String::new();
    for q in &quotes {
        text.push_str(q);
        for _ in 0..5 {
            text.push_str(
                " и операторы смотрели на это число одиннадцать минут до того, как пришло \
                 оповещение, а потом смотрели снова, чтобы убедиться",
            );
        }
        text.push('\n');
    }
    (text, quotes)
}

struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answers: Mutex<Vec<String>>,
    calls: Arc<AtomicUsize>,
}

impl Provider for Scripted {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }
    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }
    fn complete(&self, _req: &Request) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        // In order, and the last repeats: the passes of one window are consecutive calls, so an
        // order is how a *drifting* second pass is expressed at all.
        let a = if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers.first().cloned().unwrap_or_default()
        };
        Ok(Completion::new(a, "scripted", Usage::reported(100, 30)))
    }
    fn probe(&self) -> Result<Probe, ProviderError> {
        Ok(Probe::reachable(
            vec!["scripted".into()],
            self.caps.clone(),
            "",
        ))
    }
}

fn driver(answers: Vec<String>) -> (Registry, Arc<AtomicUsize>) {
    let mut caps = Capabilities::default();
    caps.offline = true;
    caps.context_window = 2048;
    caps.structured = StructuredMode::JsonSchema;
    let calls = Arc::new(AtomicUsize::new(0));
    let p = Scripted {
        id: ProviderId::new("scripted").unwrap(),
        caps,
        answers: Mutex::new(answers),
        calls: Arc::clone(&calls),
    };
    let id = p.id();
    (
        Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
    )
}

fn answer(label: &str, gist: &str, quote: &str) -> String {
    format!(
        r#"{{"units":[{{"type":"claim","label":"{label}","gist":"{gist}","status":"cited","quote":"{quote}"}}]}}"#
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("smysl-ge-t2-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn library(tag: &str, text: &str, lang: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("a library");
    lib.add(
        &Input::new(text.as_bytes()),
        &AddSpec::new("txt/1", "corpus", "public-domain")
            .with_lang(smysl_core::ids::LangTag::new(lang).expect("a tag")),
        &Caps::DEFAULT,
    )
    .expect("the text was added");
    (dir, lib)
}

fn run(lib: &mut Library, registry: &Registry, passes: Passes) -> TextReport {
    let mut t = TextIngest::new(
        lib,
        Target::Alias("corpus".to_string()),
        Level::new("line").expect("a level"),
    )
    .with_digest_budget(0);
    let opts = IngestOptions::at_rung(Rung::Document)
        .with_passes(passes)
        .with_model("scripted")
        .with_max_output(256)
        .with_agent(smysl_core::AgentId::new("tool:ge-t2").expect("an agent"));
    Ingestor::new(registry, opts)
        .ingest_text(&mut t)
        .expect("the run reached the provider")
}

/// The two passes of each window saying the same thing, in both languages.
#[test]
fn two_passes_that_found_the_same_places_report_full_stability_in_both_languages() {
    let mut rates: BTreeMap<&str, f64> = BTreeMap::new();
    let mut paths: BTreeMap<&str, String> = BTreeMap::new();
    for (tag, lang, (text, quotes)) in [("en", "en", english()), ("ru", "ru", russian())] {
        let (_dir, mut lib) = library(tag, &text, lang);
        // Window 1 twice, window 2 twice.
        let (registry, _calls) = driver(vec![
            answer("c/one", "the wait rose", &quotes[0]),
            answer("c/one", "the wait rose", &quotes[0]),
            answer("c/two", "the rollback helped", &quotes[1]),
            answer("c/two", "the rollback helped", &quotes[1]),
        ]);
        let report = run(&mut lib, &registry, Passes::TEXT);
        assert_eq!(report.lang, lang, "the report does not carry its language");
        assert_eq!(report.windows, 2, "{report:?}");
        assert_eq!(report.agreement.passes, 2);
        assert_eq!(report.agreement.spanless, 0, "{:?}", report.diagnostics);
        let rate = report.stability().expect("nothing was comparable");
        eprintln!(
            "GE-T2 ({lang}, scripted): {} comparable place(s), {} agreed, stability {rate:.3}",
            report.agreement.comparable, report.agreement.agreed,
        );
        rates.insert(lang, rate);
        paths.insert(
            lang,
            format!("{:?}", report.path.expect("a path was chosen")),
        );
    }
    assert_eq!(rates.get("en"), Some(&1.0));
    assert_eq!(rates.get("ru"), Some(&1.0), "{rates:?}");
    // And both corpora took the **same** path. They did not before this step: the path was
    // chosen from a window's byte length, two bytes per Cyrillic character put the Russian run
    // over a 4 KiB bound, and its answers were then parsed by the surface parser — a figure
    // reported per language that was measuring two pipelines (§3.9.2, `path::choose_tokens`).
    assert_eq!(
        paths.get("en"),
        paths.get("ru"),
        "the two languages took different ingest paths: {paths:?}",
    );
}

/// A second pass that found a *different* place: the figure has to fall, or it is not measuring.
#[test]
fn a_drifting_second_pass_lowers_the_figure_rather_than_being_absorbed() {
    let (text, quotes) = english();
    let (_dir, mut lib) = library("drift", &text, "en");
    // Window 1: the second pass quotes the line the first pass did not. Both spans exist, so
    // both are comparable and neither is agreed.
    let (registry, _calls) = driver(vec![
        answer("c/one", "the wait rose", &quotes[0]),
        answer("c/two", "the rollback helped", &quotes[1]),
    ]);
    let report = run(&mut lib, &registry, Passes::TEXT);
    let rate = report.stability().expect("nothing was comparable");
    eprintln!(
        "GE-T2 (drifting): {} comparable, {} agreed, stability {rate:.3}",
        report.agreement.comparable, report.agreement.agreed,
    );
    assert!(
        rate < 1.0,
        "two passes that found different places reported full agreement: {report:?}",
    );
    // And the units are both staged: nothing is deduplicated during ingest, and the disagreement
    // is a figure rather than a discarded record (§3.4).
    assert!(report.units >= 2, "{report:?}");
}

/// A single-pass run has nothing to compare, and says so rather than reporting 1.0.
#[test]
fn a_single_pass_run_reports_no_figure_at_all() {
    let (text, quotes) = english();
    let (_dir, mut lib) = library("single", &text, "en");
    let (registry, _calls) = driver(vec![answer("c/one", "the wait rose", &quotes[0])]);
    let report = run(&mut lib, &registry, Passes::Single);
    assert_eq!(report.agreement.passes, 1);
    assert_eq!(
        report.stability(),
        None,
        "one pass claimed agreement with itself",
    );
}

/// A unit with no span is not a disagreement: it is excluded, and counted where a reader can see
/// how much of the run was uncomparable.
#[test]
fn a_spanless_unit_is_excluded_from_the_figure_rather_than_counted_against_it() {
    let (text, quotes) = english();
    let (_dir, mut lib) = library("spanless", &text, "en");
    // Both passes of window 1 agree; both passes of window 2 quote something no window holds.
    let ghost = "the shard was replaced on the Tuesday before the incident";
    let (registry, _calls) = driver(vec![
        answer("c/one", "the wait rose", &quotes[0]),
        answer("c/one", "the wait rose", &quotes[0]),
        answer("c/ghost", "the shard was replaced", ghost),
    ]);
    let report = run(&mut lib, &registry, Passes::TEXT);
    // Two counters, two denominators, and the difference is where the quote went. The repair
    // loop refuses an unsupported quote (`SMY-E307`) and after three attempts degrades the unit
    // to opaque prose, which **removes** the quote — so `report.spanless`, which counts units
    // that attributed a quote and got no span, does not see it at all. The agreement does: a
    // unit with no span cannot establish `identical-span` however it lost one.
    assert!(
        report.agreement.spanless > 0,
        "the control attached anyway: {report:?}",
    );
    let rate = report.stability().expect("the first window was comparable");
    eprintln!(
        "GE-T2 (with {} spanless): comparable {}, agreed {}, stability {rate:.3}",
        report.agreement.spanless, report.agreement.comparable, report.agreement.agreed,
    );
    assert_eq!(
        rate, 1.0,
        "a unit nobody could compare was counted as a disagreement",
    );
}

// ---------------------------------------------------------------- the live arm

/// The rerun itself, against a real provider over a real corpus — or a statement of what is
/// missing. See the module doc: the corpus is GE-T4's material and this repository does not
/// hold it.
#[test]
fn the_rerun_over_a_corpus_is_owed_until_a_credential_is_spent() {
    let live = std::env::var("SMYSL_INGEST_LIVE").unwrap_or_default();
    if live.trim().is_empty() {
        eprintln!(
            "GE-T2 rerun OWED: the instrument above is measured; the figure needs \
             SMYSL_INGEST_LIVE=1, because it spends a credential and is never taken by \
             accident. The corpus is no longer owed: `fixtures/stability/corpus/` holds one \
             per tier-1 language, and SMYSL_GE_T2=<dir> replaces it with somebody else's."
        );
        return;
    }
    let dir = corpus_dir();
    let corpus = dir.display().to_string();
    let provider = match reachable() {
        Some(p) => p,
        None => panic!("SMYSL_INGEST_LIVE is set and no configured provider is reachable"),
    };
    let id = provider.id();
    let model = provider.caps().model.clone();
    let registry = Registry::new()
        .with_provider(provider)
        .route(Task::ContentIngest, id.clone());

    let mut ran = 0usize;
    for entry in std::fs::read_dir(&dir).expect("the corpus directory") {
        let path = entry.expect("an entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let lang = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a language in the file name")
            .to_string();
        let text = std::fs::read_to_string(&path).expect("a corpus file");
        let (_dir, mut lib) = library(&format!("live-{lang}"), &text, &lang);
        let opts = IngestOptions::at_rung(Rung::Document)
            .with_passes(Passes::TEXT)
            .with_model(&model)
            .with_max_output(OUTPUT)
            .with_agent(smysl_core::AgentId::new("tool:ge-t2").expect("an agent"));
        let mut t = TextIngest::new(
            &mut lib,
            Target::Alias("corpus".to_string()),
            Level::new("line").expect("a level"),
        );
        let report = Ingestor::new(&registry, opts)
            .ingest_text(&mut t)
            .unwrap_or_else(|e| panic!("{lang}: {e}"));
        eprintln!(
            "GE-T2 ({lang}, {id}): {} window(s), comparable {}, agreed {}, spanless {}, \
             stability {:?}, span coverage {:?}",
            report.windows,
            report.agreement.comparable,
            report.agreement.agreed,
            report.agreement.spanless,
            report.stability(),
            report.span_coverage(),
        );
        // The decomposition, because the reported figure alone cannot be read. `identical-span`
        // includes the gist (§3.5), so a pass that found the same bytes and re-worded its
        // sentence counts as disagreement; `place_rate` is the same question with the gist left
        // out. Reported beside the figure rather than instead of it.
        eprintln!(
            "GE-T2 ({lang}, {id}): places comparable {}, agreed {}, place rate {:?}",
            report.agreement.comparable_places,
            report.agreement.agreed_places,
            report.agreement.place_rate(),
        );
        // Reported, not thresholded (§5.5). What *is* asserted is that the rerun measured
        // something: a run whose every unit came out spanless has no stability figure, and
        // reporting the clause as met on it would be reporting the absence of a measurement.
        assert!(
            report.stability().is_some(),
            "{lang}: nothing was comparable, so GE-T2 measured nothing",
        );
        ran += 1;
    }
    assert!(ran > 0, "SMYSL_GE_T2={corpus} holds no <lang>.txt file");
}

/// The committed corpus, or whichever one the caller named.
fn corpus_dir() -> PathBuf {
    match std::env::var("SMYSL_GE_T2") {
        Ok(d) if !d.trim().is_empty() => PathBuf::from(d),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/stability/corpus"),
    }
}

/// The window geometry every provider is configured with, and the reason it is not the
/// provider's own.
///
/// GE-T2 reports **per language**, and the figure has to be about the language. Two providers
/// with different context windows chunk the same text differently — `Window::for_context` splits
/// one context between the prompt, the input and an output reserve — so a figure taken against
/// whichever provider a machine could reach would be a figure about that machine's chunking.
/// The S0 spike settled this already and for the same reason: SMYSL-2.2 §4 sets
/// `context_window: 32768` and `max_output: 8192` in *both* providers it measured, "so the
/// windows are identical". These are those numbers.
///
/// This is also the defect the live arm shipped with. It set `max_output` to **800** — a value
/// carried over from the scripted instrument above, where an answer is one line of JSON — and
/// never set `context_window` at all, so it kept `ProviderConfig::default()`'s 4096. Against a
/// real corpus every answer was cut off at the limit (`SMY-W304`), every span degraded to opaque
/// prose, and `comparable` was 0: the arm reported *nothing was comparable* rather than a
/// figure, and it would have done so against any corpus and any provider. A scripted provider
/// cannot catch it, because its answers fit in any budget.
const CONTEXT: usize = 32_768;
const OUTPUT: usize = 8_192;

/// The first configured provider this machine can reach, built as `providers_live.rs` builds one.
fn reachable() -> Option<Box<dyn Provider>> {
    for (id, kind, endpoint, model, key) in [
        (
            "ollama",
            "ollama",
            "http://127.0.0.1:11434",
            "llama3.2",
            None,
        ),
        (
            "deepseek",
            "deepseek",
            "https://api.deepseek.com",
            "deepseek-chat",
            Some("DEEPSEEK_API_KEY"),
        ),
        (
            "gemini",
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.0-flash",
            Some("GEMINI_API_KEY"),
        ),
    ] {
        if let Some(var) = key {
            if !std::env::var(var).is_ok_and(|v| !v.trim().is_empty()) {
                continue;
            }
        }
        let mut cfg = ProviderConfig::new(ProviderId::new(id).unwrap(), kind);
        cfg.endpoint = endpoint.into();
        // `SMYSL_GE_T2_MODEL` names the model, because the default cannot be right for every
        // machine and Ollama is tried first.
        //
        // It was added to find out whether the Ollama arm's result was the model's fault, and
        // the answer is **no**: over this corpus `llama3.2` (3B) and `qwen2.5:14b-instruct`
        // both return one opaque unit per pass, two spanless units, nothing comparable — while
        // the same corpus against `deepseek-chat` attaches every unit it extracts (span
        // coverage 1.000 in all five languages, three runs). Every Ollama call answered 200.
        // So the local arm is not measurable here, and the cause is in the path rather than in
        // the model's size. SMYSL-2.1 H-10 and 2.2 O-3 both flag this mapper as unverified for
        // want of an Ollama to test against; this is the first measurement of it over a real
        // corpus, and diagnosing it is its own work, not GE-T2's.
        cfg.model = std::env::var("SMYSL_GE_T2_MODEL")
            .ok()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| model.to_string());
        cfg.context_window = CONTEXT;
        cfg.max_output = OUTPUT;
        cfg.timeout_secs = 120;
        cfg.api_key_env = key.map(str::to_string);
        let Ok(provider) = smysl_provider::map::build(&cfg) else {
            continue;
        };
        if provider.probe().map(|p| p.reachable).unwrap_or(false) {
            return Some(provider);
        }
    }
    None
}
