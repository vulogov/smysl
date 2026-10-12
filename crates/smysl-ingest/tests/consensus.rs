//! **The exit clause for TX-P5 step 3a** (SMYSL-2.4 §6): a window extracted twice at
//! temperature 0 yields the `identical-span` agreement the fixture states, and the same window
//! at temperature 0.7 is refused rather than silently producing two disjoint sets.
//!
//! Why a scripted provider and not a live one. The thing under test is the *boundary's*
//! arithmetic — two extractions, both staged, nothing merged, one agreement figure — and a
//! live model would make the fixture's figure a property of whatever the endpoint returned
//! today. S0's MS-8 already measured the model half: arm L was bit-identical across all 145
//! runs at T=0 and shared no uid at all at T=0.7 on five of six inputs. What a scripted
//! provider can assert and a live one cannot is that *two disjoint answers are measured as
//! disjoint*, which is the case the refusal exists to prevent reaching a corpus.
//!
//! The fixtures are in `fixtures/ingest/consensus/`, with their own README.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_core::{Code, Mid, Rung, SourceKind, SourcePolicy, SourceRef};
use smysl_graph::Store;
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};

/// A provider that answers from a script, one answer per call, so "the second pass said
/// something else" is expressible.
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
        caps.context_window = 8192;
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
    fn complete(&self, _req: &Request) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let mut answers = self.answers.lock().unwrap();
        // The last answer repeats, so a repair turn gets the same answer the pass gave —
        // which is what "the model is consistent at T=0" means, and keeps a repair from
        // silently supplying the agreement the test is measuring.
        let a = if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers.first().cloned().unwrap_or_default()
        };
        Ok(Completion::new(a, "scripted", Usage::reported(10, 20)))
    }
    fn probe(&self) -> Result<Probe, ProviderError> {
        Ok(Probe::reachable(
            vec!["scripted".into()],
            self.caps.clone(),
            "",
        ))
    }
}

fn registry(p: Scripted) -> (Registry, Arc<AtomicUsize>) {
    let calls = Arc::clone(&p.calls);
    let id = p.id();
    (
        Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
    )
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/ingest/consensus")
        .canonicalize()
        .expect("the consensus fixtures are in the repository")
}

fn read(rel: &str) -> String {
    let p = fixtures().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// The figures a `*/agreement.txt` states.
fn stated(dir: &str) -> (usize, usize, usize, usize, String) {
    let src = read(&format!("{dir}/agreement.txt"));
    let field = |name: &str| -> String {
        src.lines()
            .find_map(|l| l.strip_prefix(&format!("{name} ")))
            .unwrap_or_else(|| panic!("{dir}/agreement.txt states no `{name}`"))
            .trim()
            .to_string()
    };
    let n = |name: &str| field(name).parse::<usize>().expect("a count");
    (
        n("passes"),
        n("comparable"),
        n("agreed"),
        n("spanless"),
        field("rate"),
    )
}

/// The window's own source, with a span, so the units have a *place* to agree about.
///
/// `identical-span` is a relation between places. Per-unit spans are span attachment's (§6
/// TX-P5 step 3); the window's span is what a caller can supply today, through the
/// `--source` option that has existed since 1.6, and it is enough to measure the agreement
/// the exit clause names.
fn window_source() -> (SourceRef, SourcePolicy) {
    let tid = format!("t3:{}", "a".repeat(64));
    let source = SourceRef::new(SourceKind::Doc, format!("{tid}#1"))
        .in_part(0, read("window.txt").len() as u64, Mid::of(b"the window"))
        .expect("a forward range");
    (source, SourcePolicy::Override)
}

fn opts(passes: Passes) -> IngestOptions {
    let (source, policy) = window_source();
    IngestOptions::at_rung(Rung::Document)
        .with_passes(passes)
        .with_source(source, policy)
}

/// Ingest the fixture window under `dir`'s two scripted answers.
fn run(dir: &str, passes: Passes) -> (smysl_ingest::IngestReport, smysl_ingest::Staged, usize) {
    let answers = vec![
        read(&format!("{dir}/pass-1.json")),
        read(&format!("{dir}/pass-2.json")),
    ];
    let (r, calls) = registry(Scripted::saying(answers));
    let (staged, report) = Ingestor::new(&r, opts(passes))
        .ingest(&Store::new(), &read("window.txt"))
        .expect("a scripted provider cannot fail");
    (report, staged, calls.load(Ordering::SeqCst))
}

fn rate(a: &smysl_ingest::Agreement) -> String {
    a.rate().map(|r| format!("{r:.3}")).unwrap_or("-".into())
}

// ---------------------------------------------------------------------------
// The exit clause, first half: the agreement the fixture states
// ---------------------------------------------------------------------------

/// **The gate.** Two passes at temperature 0 over one window, and the agreement is the
/// fixture's — computed by the ingest boundary, not restated by the test.
#[test]
fn a_window_extracted_twice_at_zero_agrees_as_the_fixture_states() {
    for dir in ["agree", "drift"] {
        let (report, _, calls) = run(dir, Passes::Consensus);
        // Two passes over one chunk: two calls, and no repair turn hiding among them.
        assert_eq!(calls, 2, "{dir}: one chunk, two passes");
        assert_eq!(report.passes, 2, "{dir}");
        assert_eq!(
            report.chunks, 1,
            "{dir}: chunks count pieces, not extractions"
        );

        let a = report
            .agreement
            .expect("a consensus run records its agreement");
        let want = stated(dir);
        assert_eq!(
            (a.passes, a.comparable, a.agreed, a.spanless, rate(&a)),
            want,
            "{dir}"
        );
    }
}

/// Both passes are staged, and nothing is deduplicated by the policy: what collapses two
/// identical units into one record is content addressing, which is why the agreement is
/// recorded rather than recoverable from a unit count.
#[test]
fn both_passes_are_staged_and_the_count_cannot_answer_the_agreement() {
    let (agreeing, staged_agree, _) = run("agree", Passes::Consensus);
    let (drifting, staged_drift, _) = run("drift", Passes::Consensus);

    // Identical passes: three units said twice are three units.
    assert_eq!(staged_agree.units.len(), 3);
    assert_eq!(agreeing.agreement.map(|a| a.agreed), Some(3));

    // The drifting run staged *four* units — every place either pass found — and the count
    // on its own would read as a richer extraction rather than a less stable one.
    assert_eq!(staged_drift.units.len(), 4);
    assert_eq!(drifting.agreement.map(|a| a.agreed), Some(1));

    // The second pass gave `e/pool` to a different unit, so the first pass's binding stands
    // and the disagreeing unit is staged without a name. The pre-existing cross-chunk rule,
    // applied to passes; the fixture README states it.
    let w054 = drifting
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::W054)
        .count();
    assert_eq!(w054, 1, "a label that moved between passes is reported");
    assert_eq!(staged_drift.labels.len(), 3);
}

/// A single pass does what every release up to 1.10 did: one call per chunk, and no agreement
/// figure at all — because one extraction agreeing with itself is the policy, not a
/// measurement.
#[test]
fn a_single_pass_run_makes_one_call_and_claims_no_agreement() {
    let (report, staged, calls) = run("drift", Passes::Single);
    assert_eq!(calls, 1);
    assert_eq!(report.passes, 1);
    assert!(report.agreement.is_none());
    assert_eq!(staged.units.len(), 2, "only the first pass ran");
}

// ---------------------------------------------------------------------------
// The exit clause, second half: the refusal
// ---------------------------------------------------------------------------

/// **The gate.** The same window at T=0.7 under consensus is refused, and refused *before*
/// any call: two extractions above temperature 0 share no identity (MS-8), so the alternative
/// is paying for two answers and reporting an agreement of nothing.
#[test]
fn a_second_pass_above_temperature_zero_is_refused_before_any_call() {
    let answers = vec![read("agree/pass-1.json"), read("agree/pass-2.json")];
    let (r, calls) = registry(Scripted::saying(answers));
    let err = Ingestor::new(&r, opts(Passes::Consensus).with_temperature(0.7))
        .ingest(&Store::new(), &read("window.txt"))
        .expect_err("consensus above temperature 0 is refused");

    assert!(
        matches!(err, ProviderError::Config(_)),
        "a caller's configuration, not a provider failure: {err}"
    );
    let msg = err.to_string();
    assert!(msg.contains("--single-pass"), "{msg}");
    assert!(msg.contains("0.7"), "{msg}");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "refused before egress");
}

/// And the opt-out works: sampling deliberately is a thing a caller may do, as long as the
/// corpus records that it is what happened.
#[test]
fn a_single_pass_above_temperature_zero_is_permitted() {
    let answers = vec![read("agree/pass-1.json")];
    let (r, calls) = registry(Scripted::saying(answers));
    let (_, report) = Ingestor::new(&r, opts(Passes::Single).with_temperature(0.7))
        .ingest(&Store::new(), &read("window.txt"))
        .expect("`--single-pass` admits a temperature");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(report.agreement.is_none());
}

// ---------------------------------------------------------------------------
// The recipe
// ---------------------------------------------------------------------------

/// A corpus built one way is not comparable with one built the other, so the recipe says
/// which — and a single-pass recipe is byte for byte the one 1.10 computed before the field
/// existed, which is what makes every corpus already extracted still aggregate.
#[test]
fn the_recipe_records_consensus_and_leaves_a_single_pass_where_it_was() {
    let (single, _, _) = run("agree", Passes::Single);
    let (double, _, _) = run("agree", Passes::Consensus);
    assert_ne!(single.recipe, double.recipe);
    assert_ne!(single.family, double.family);

    let mut base = smysl_ingest::recipe::Conditions::new("t", 1);
    let was = (base.recipe(), base.family());
    base = base.with_passes(Passes::Single);
    assert_eq!(
        (base.recipe(), base.family()),
        was,
        "single pass adds nothing"
    );
    let consensus = base.with_passes(Passes::Consensus);
    assert_ne!((consensus.recipe(), consensus.family()), was);
}
