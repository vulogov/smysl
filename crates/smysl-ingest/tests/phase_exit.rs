//! **The TX-P5 phase exit** (SMYSL-2.4 §6 TX-P5 step 5), measured rather than asserted.
//!
//! Six clauses close the phase. Four of them are figures over a run, and this harness produces
//! them; two are measured elsewhere, and this file says where rather than restating them:
//!
//! | clause | where |
//! |---|---|
//! | resume after a kill loses ≤ one batch | here, `clause_1_*` |
//! | ≥ 95% of units carry a span | here, `clause_2_*` — with the control that makes it a measurement |
//! | entity duplication < 2% across windows | here, `clause_3_*` |
//! | the probe-set gate of step 3b | `probe_set.rs`; **owed**, and the reason is a finding about the readers |
//! | GE-T2 rerun under the consensus policy | `ge_t2.rs` |
//! | a cost report from the journal | here, `clause_6_*` |
//!
//! # Why a scripted provider can measure four of them and not the other two
//!
//! A clause about the *boundary* — which bytes a quote becomes, which uid a holder gets, what the
//! journal holds after a kill, what a window cost — is a property of this code, and a scripted
//! answer pins it exactly where a live one would make it a property of whatever an endpoint
//! returned today. The two clauses that are **not** of that kind are the ones left elsewhere:
//! GE-T2 asks how stable a *model* is, and the probe set asks how often a model misreads reported
//! speech. Neither can be answered by a provider that says what the test told it to say, so
//! neither is answered here, and a harness that scripted them would be reporting its own fixture.
//!
//! What this file does claim is that each measurable clause **can fail**: every figure has a
//! control beside it that moves the number the wrong way, because a gate that cannot fail is a
//! constant with an assertion around it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_ingest::text::cost;
use smysl_ingest::text::journal::Journal;
use smysl_ingest::text::{Target, TextIngest, TextReport};
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};
use smysl_text::reading::Level;
use smysl_text::{AddSpec, Caps, Input, Library};

/// The phase exit's own numbers (§6 TX-P5 step 5).
const SPAN_COVERAGE: f64 = 0.95;
const ENTITY_DUPLICATION: f64 = 0.02;

/// How many nodes the corpus has, and therefore how many windows: the window budget's floor is
/// 256 tokens, and each line below is padded past it, so one line is one window.
const LINES: usize = 8;

/// A line of the corpus, and the quote an answer about it names.
///
/// The opening of each line is unique, so an answer's quote locates its own window and nothing
/// else — which is what keeps the coverage figure about attachment rather than about a quote
/// that happened to occur twice (`SMY-W419` is step 3's test for the other case).
fn quote(i: usize) -> String {
    format!("finding {i} held through the whole review")
}

fn corpus() -> String {
    let mut out = String::new();
    for i in 0..LINES {
        out.push_str(&quote(i));
        // Padded past the window floor, so the line does not share a window with its neighbour.
        for _ in 0..9 {
            out.push_str(
                " and the operators read it again in the morning and it still said what it had \
                 said the night before, which is the only kind of finding a report is for",
            );
        }
        out.push('\n');
    }
    out
}

/// A provider that answers from a script and counts what it was asked.
struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answers: Mutex<Vec<String>>,
    calls: Arc<AtomicUsize>,
}

impl Scripted {
    fn saying(answers: Vec<String>) -> Scripted {
        let mut caps = Capabilities::default();
        caps.offline = true;
        caps.context_window = 2048;
        caps.structured = StructuredMode::JsonSchema;
        Scripted {
            id: ProviderId::new("scripted").unwrap(),
            caps,
            answers: Mutex::new(answers),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Provider for Scripted {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }
    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }
    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        // The answer whose quote this window actually contains, which is what makes a run of
        // eight windows scriptable without an answer order to keep in step. A window that
        // matches nothing gets the first answer, which is then a quote the window does not
        // support — the control the coverage figure needs.
        let shown = req
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let answers = self.answers.lock().unwrap();
        let a = answers
            .iter()
            .find(|a| quoted(a).map(|q| shown.contains(&q)).unwrap_or_default())
            .or_else(|| answers.first())
            .cloned()
            .unwrap_or_default();
        Ok(Completion::new(a, "scripted", Usage::reported(120, 40)))
    }
    fn probe(&self) -> Result<Probe, ProviderError> {
        Ok(Probe::reachable(
            vec!["scripted".into()],
            self.caps.clone(),
            "",
        ))
    }
}

/// The `quote` field of a scripted answer, which is how an answer is matched to its window.
fn quoted(answer: &str) -> Option<String> {
    let at = answer.find("\"quote\":\"")? + "\"quote\":\"".len();
    let rest = &answer[at..];
    Some(rest[..rest.find('"')?].to_string())
}

struct Driver {
    registry: Registry,
    calls: Arc<AtomicUsize>,
}

fn driver(answers: Vec<String>) -> Driver {
    let p = Scripted::saying(answers);
    let calls = Arc::clone(&p.calls);
    let id = p.id();
    Driver {
        registry: Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
    }
}

fn answer(label: &str, gist: &str, quote: &str) -> String {
    format!(
        r#"{{"units":[{{"type":"claim","label":"{label}","gist":"{gist}","status":"cited","quote":"{quote}"}}]}}"#
    )
}

/// One answer per line of the corpus.
fn answers() -> Vec<String> {
    (0..LINES)
        .map(|i| answer(&format!("c/f{i}"), &format!("finding {i} held"), &quote(i)))
        .collect()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("smysl-phase-exit-{}-{tag}", std::process::id()));
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

fn library(tag: &str, text: &str, reader: &str, alias: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("a library");
    lib.add(
        &Input::new(text.as_bytes()),
        &AddSpec::new(reader, alias, "public-domain")
            .with_lang(smysl_core::ids::LangTag::new("en").expect("a tag")),
        &Caps::DEFAULT,
    )
    .expect("the text was added");
    (dir, lib)
}

fn opts(passes: Passes) -> IngestOptions {
    IngestOptions::at_rung(smysl_core::Rung::Document)
        .with_passes(passes)
        .with_model("scripted")
        .with_max_output(256)
        .with_agent(smysl_core::AgentId::new("tool:phase-exit").expect("an agent"))
}

fn ingest(lib: &mut Library, d: &Driver, alias: &str, level: &str, passes: Passes) -> TextReport {
    let mut t = TextIngest::new(
        lib,
        Target::Alias(alias.to_string()),
        Level::new(level).expect("a level"),
    )
    .with_digest_budget(0);
    Ingestor::new(&d.registry, opts(passes))
        .ingest_text(&mut t)
        .expect("the run reached the provider")
}

// ---------------------------------------------------------------- clause 1: resume

#[test]
fn clause_1_a_kill_loses_at_most_the_batch_in_flight() {
    let text = corpus();
    let (dir, mut lib) = library("resume", &text, "txt/1", "report");
    let first = driver(answers());
    let one = ingest(&mut lib, &first, "report", "line", Passes::Single);
    assert_eq!(
        one.windows, LINES,
        "the corpus is not {LINES} windows: {one:?}"
    );
    assert_eq!(one.skipped, 0);
    let committed = lib.store().units().count();

    // The kill: the process died in the middle of the last journal append.
    let path = dir.path().join(smysl_ingest::text::journal::FILE);
    let bytes = std::fs::read(&path).expect("a journal");
    std::fs::write(&path, &bytes[..bytes.len() - 5]).expect("truncated");
    assert_eq!(
        Journal::open(dir.path()).len(),
        LINES - 1,
        "a truncated tail cost more than the entry being written",
    );

    let second = driver(answers());
    let two = ingest(&mut lib, &second, "report", "line", Passes::Single);
    let lost = two.windows - two.skipped;
    eprintln!(
        "clause 1: windows {}, re-sent after the kill {lost}",
        two.windows
    );
    assert_eq!(lost, 1, "the resume re-sent {lost} windows");
    // And the re-extraction cost the corpus nothing: the same answer gives the same uid, so a
    // window done twice is one record (content addressing, not deduplication).
    assert_eq!(lib.store().units().count(), committed);
}

// ---------------------------------------------------------------- clause 2: span coverage

#[test]
fn clause_2_at_least_95_percent_of_units_carry_a_span() {
    let text = corpus();
    let (_dir, mut lib) = library("coverage", &text, "txt/1", "report");
    let d = driver(answers());
    let report = ingest(&mut lib, &d, "report", "line", Passes::TEXT);

    let coverage = report
        .span_coverage()
        .expect("something attributed a quote");
    eprintln!(
        "clause 2: {} spanned, {} spanless, coverage {:.3}",
        report.spanned, report.spanless, coverage,
    );
    assert!(
        coverage >= SPAN_COVERAGE,
        "span coverage {coverage:.3} is under the phase exit's {SPAN_COVERAGE}: {:?}",
        report.diagnostics,
    );
    // Every unit in the store, and not only the report's counters: the figure is about what a
    // reader can resolve, which is what is on disk.
    let (with, without): (Vec<_>, Vec<_>) = lib
        .store()
        .units()
        .filter(|(_, u)| u.core.schema.as_str() != smysl_core::schemas::text::ENTITY)
        .partition(|(_, u)| u.core.source.as_ref().is_some_and(|s| s.span().is_some()));
    assert!(!with.is_empty());
    assert_eq!(without.len(), 0, "a committed unit carries no span");
}

/// The control: the clause is a measurement, so there has to be a run that comes out **under**
/// the bar — and what it takes to get there is worth knowing.
///
/// A quote the window does not hold at all does **not** lower this figure: the repair loop
/// refuses it (`SMY-E307`) and the unit is degraded to opaque prose, which removes the quote, so
/// it never reaches attachment. The case the figure is actually about is a quote the window
/// *does* hold and a part does not: the corpus below is padded past the part policy's minimum,
/// so each line is its own part, and a window shows the node it owns after the last node of the
/// part before it. A quote spanning that join is in the window's text and in no part's bytes in
/// that order, so it is staged with no span and a capped status (`SMY-W441`, §3.4) — which is
/// step 3's rule that a span must not cross a node boundary unless the bytes do.
#[test]
fn clause_2_control_a_quote_that_crosses_a_part_boundary_lowers_the_figure() {
    let text = corpus();
    let (_dir, mut lib) = library("coverage-control", &text, "txt/1", "report");
    let mid = lib.head("report").expect("a head");
    let parts = lib.store().manifest(&mid).expect("a manifest").parts.len();
    assert!(
        parts > 1,
        "the corpus is one part, so it has no join to cross"
    );

    // The tail of one line and the head of the next, with the separator the window renders
    // between them. In the window; in no part.
    let across = format!("the only kind of finding a report is for\\n{}", quote(1));
    let mut answers = vec![answer(
        "c/across",
        "the finding held across the join",
        &across,
    )];
    // Every other window answers for itself; the window that owns line 1 is left without an
    // answer of its own, so it falls back to the one above — and that quote *is* in the text it
    // was shown, because it owns the second half of it.
    answers.extend(
        (2..LINES).map(|i| answer(&format!("c/f{i}"), &format!("finding {i} held"), &quote(i))),
    );
    let d = driver(answers);
    let report = ingest(&mut lib, &d, "report", "line", Passes::Single);
    let coverage = report.span_coverage().expect("the other windows attached");
    eprintln!(
        "clause 2 control: {} spanned, {} spanless, coverage {coverage:.3}",
        report.spanned, report.spanless,
    );
    assert!(
        report.spanless > 0,
        "a quote spanning two parts attached anyway: {report:?}",
    );
    assert!(
        coverage < SPAN_COVERAGE,
        "one window in {LINES} quoting across a part left coverage at {coverage:.3}",
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.code == smysl_core::Code::W441),
        "nothing reported the absent quote: {:?}",
        report.diagnostics,
    );
}

// ---------------------------------------------------------------- clause 3: entities

/// A chat export: three speakers, one long message each, repeated — so every speaker appears in
/// several windows and the duplication figure has something to measure.
fn chat() -> String {
    let who = ["ada", "bruno", "clare"];
    let mut messages = String::new();
    for i in 0..9usize {
        let text = format!(
            "message {i} said the pool wait rose and then said it again{}",
            " and the operators read it in the morning and it still said what it said".repeat(8),
        );
        if i > 0 {
            messages.push_str(",\n");
        }
        messages.push_str(&format!(
            r#"  {{"id": {}, "type": "message", "date": "2024-01-15T10:{:02}:00",
   "date_unixtime": "{}", "from": "{}", "from_id": "{}", "text": "{text}"}}"#,
            i + 2,
            i,
            1_705_314_000 + i * 60,
            who[i % who.len()],
            who[i % who.len()],
        ));
    }
    format!("{{\n \"name\": \"Incident\",\n \"type\": \"private_group\",\n \"id\": 77,\n \"messages\": [\n{messages}\n ]\n}}\n")
}

#[test]
fn clause_3_entity_duplication_across_windows_is_under_two_percent() {
    let (_dir, mut lib) = library("entities", &chat(), "telegram/1", "incident");
    // One answer per message, matched by its quote as every other run here is.
    let answers: Vec<String> = (0..9)
        .map(|i| {
            answer(
                &format!("c/m{i}"),
                &format!("message {i} reported the wait"),
                &format!("message {i} said the pool wait rose"),
            )
        })
        .collect();
    let d = driver(answers);
    let report = ingest(&mut lib, &d, "incident", "message", Passes::Single);
    assert!(
        report.windows >= 3,
        "one window measures nothing: {report:?}"
    );

    // Every `x.text/entity` the store holds, and the speakers the reading actually has.
    let staged: Vec<_> = lib
        .store()
        .units()
        .filter(|(_, u)| u.core.schema.as_str() == smysl_core::schemas::text::ENTITY)
        .collect();
    let speakers = 3usize;
    assert!(
        report.entities >= speakers,
        "no holder reached the store: {report:?}",
    );
    let duplication = (staged.len() as f64 - speakers as f64) / speakers as f64;
    eprintln!(
        "clause 3: {} windows, {} entity record(s) in the store, {speakers} speaker(s), \
         duplication {duplication:.3}",
        report.windows,
        staged.len(),
    );
    assert!(
        duplication < ENTITY_DUPLICATION,
        "{} entity records for {speakers} speakers across {} windows",
        staged.len(),
        report.windows,
    );
    // And the figure is not vacuous: the holder was attributed in more windows than there are
    // entities, so the records really did collapse (A-2.4 rule 7) rather than never being made.
    assert!(
        report.units > staged.len(),
        "fewer claims than entities: nothing was attributed twice",
    );
}

// ---------------------------------------------------------------- clause 6: the cost report

#[test]
fn clause_6_the_cost_report_is_per_window_per_language_and_measures_its_retries() {
    let text = corpus();
    let (dir, mut lib) = library("cost", &text, "txt/1", "report");
    let d = driver(answers());
    let report = ingest(&mut lib, &d, "report", "line", Passes::TEXT);

    let journal = Journal::open(dir.path());
    let priced = cost::cost(&journal, None);
    let en = priced.lang("en").expect("the corpus is in English");
    eprintln!(
        "clause 6: {} window(s), {} call(s), {} retr(ies), rate {:?}, tokens/window {:?}",
        en.windows,
        en.calls,
        en.retries,
        en.retry_rate(),
        en.tokens_per_window(),
    );
    assert_eq!(en.windows, report.windows);
    assert_eq!(en.unpriced, 0, "this release wrote an unpriced entry");
    assert_eq!(en.calls, report.calls as u64);
    // Per window, and that is the regression this step fixed: step 4 recorded the run's running
    // total against each window, so the mean grew with the position of the window in the run.
    let (input, _) = en.tokens_per_window().expect("a measured window");
    assert!(
        (input - 120.0 * report.passes as f64).abs() < 1.0,
        "the per-window figure is not one window's: {input}",
    );
    assert_eq!(
        en.input_tokens, report.usage.input_tokens,
        "the journal and the run disagree about what the corpus cost",
    );
    // No repairs here, so the retry term is a measured zero rather than an absent figure.
    assert_eq!(en.retry_rate(), Some(0.0));
    assert_eq!(priced.total.windows, en.windows);
    // An alias nobody extracted has no figures, rather than zeroed ones.
    assert_eq!(
        cost::cost(&journal, Some("synodal")).total.retry_rate(),
        None
    );
}

#[test]
fn clause_6_control_a_repair_turn_shows_up_as_a_measured_retry_rate() {
    let text = corpus();
    let (dir, mut lib) = library("cost-retry", &text, "txt/1", "report");
    // A quote no window holds buys the repair loop, which is exactly what the retry term prices.
    let d = driver(vec![answer(
        "c/ghost",
        "the shard was replaced",
        "the shard was replaced on the Tuesday before the incident",
    )]);
    let report = ingest(&mut lib, &d, "report", "line", Passes::Single);
    assert!(
        d.calls.load(Ordering::SeqCst) > report.windows,
        "{report:?}"
    );

    let journal = Journal::open(dir.path());
    let en = cost::cost(&journal, None)
        .lang("en")
        .expect("a row")
        .clone();
    let rate = en.retry_rate().expect("a measured rate");
    eprintln!(
        "clause 6 control: {} call(s) over {} window(s), {} repair turn(s), rate {rate:.2}",
        en.calls, en.windows, en.retries,
    );
    assert!(
        rate > 0.0,
        "the repair loop ran and the rate stayed at zero"
    );
    assert_eq!(
        en.attempts(),
        en.windows as u64,
        "one first attempt per window"
    );
    assert_eq!(
        report.retry_rate(),
        Some(rate),
        "the run and the journal disagree"
    );
}
