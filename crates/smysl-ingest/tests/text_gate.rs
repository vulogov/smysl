//! **Text ingest end to end, offline** (SMYSL-2.4 §5.2): spans attached, the D-1 copy set exact,
//! the ledger digest sent and scoped, the journal written after the commit, resume after a
//! simulated kill re-sending at most one window, and `--reextract`.
//!
//! One of the two exit clauses for TX-P5 step 4. The other is `redteam.rs`.
//!
//! Why scripted and not live. Every property here is the *boundary's*: which bytes a quote
//! becomes, which fields the tool writes, what the journal records and when. A live model would
//! make each of them a property of whatever an endpoint returned today, and the one thing a
//! scripted provider can assert that a live one cannot is that an answer which *tries* something
//! — a source of its own, a uid it was not shown — is handled rather than accidentally absent.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_core::{Code, Rung};
use smysl_ingest::text::journal::{Journal, Key};
use smysl_ingest::text::{Target, TextIngest};
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};
use smysl_text::reading::Level;
use smysl_text::{AddSpec, Caps, Input, Library};

/// Three lines, each long enough that two of them do not fit one window.
///
/// The window budget is derived from the provider's context window
/// (`chunk::Window::for_context`), and its floor is 256 tokens — so a three-line incident report
/// of ordinary length is *one* window, and the resume and digest properties below need more than
/// one. Each line is padded to about 600 bytes, which is 150 tokens under `utf8-div4`: one line
/// per window, with the line before it carried as the overlap.
const TEXT: &str = concat!(
    "the pool wait rose to 1.9 seconds ",
    "and the queue depth followed it up and the retries followed the queue and the ",
    "dashboards followed the retries and every one of them said the same thing about ",
    "the pool and the wait and the time it took to get a connection out of it at all ",
    "and the operators watched the number climb for eleven minutes before the page\n",
    "the rollback returned p99 to baseline ",
    "and the queue drained and the retries stopped and the dashboards went quiet and ",
    "the pool handed out connections as fast as anybody asked for one and the whole ",
    "incident was over in the time it took to redeploy the version before it, which is ",
    "the shortest part of this report and the only part anybody remembers afterwards\n",
    "the shard was never replaced ",
    "and nobody said it had been, and the claim that it was came from somewhere else ",
    "entirely, which is the sort of thing a corpus is for: the report says what it says ",
    "and a reader can see the bytes it said it in, and a claim nobody made is a claim ",
    "nobody has to argue with afterwards when the next incident looks like this one\n",
);

/// The user message of a request, which is where the window and the digest are.
fn prompt(r: &Request) -> String {
    r.messages
        .iter()
        .map(|m| m.content.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A provider that answers from a script and keeps every request it was sent.
struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answers: Mutex<Vec<String>>,
    /// Answers for the pivot pass, which is a different request and so a different script.
    /// Empty means "the model returned nothing", which is a case with a diagnostic of its own.
    translations: Mutex<Vec<String>>,
    sent: Arc<Mutex<Vec<Request>>>,
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
            translations: Mutex::new(Vec::new()),
            sent: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn translating(self, answers: Vec<String>) -> Scripted {
        *self.translations.lock().unwrap() = answers;
        self
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
        self.sent.lock().unwrap().push(req.clone());
        // Which script to answer from is read off the request, not off a call count: the pivot
        // pass makes its call in the middle of a window's work, and a test that counted calls
        // would break the moment a pass or a repair turn was added.
        let mut answers = if req.system.contains("BCP-47") {
            self.translations.lock().unwrap()
        } else {
            self.answers.lock().unwrap()
        };
        // The last answer repeats: a model at temperature 0 is consistent, and a repair turn
        // that silently got a *different* answer would hide which turn the test measured.
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

struct Driver {
    registry: Registry,
    calls: Arc<AtomicUsize>,
    sent: Arc<Mutex<Vec<Request>>>,
}

fn driver(answers: Vec<String>) -> Driver {
    let p = Scripted::saying(answers);
    let calls = Arc::clone(&p.calls);
    let sent = Arc::clone(&p.sent);
    let id = p.id();
    Driver {
        registry: Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
        sent,
    }
}

fn driver_translating(answers: Vec<String>, translations: Vec<String>) -> Driver {
    let p = Scripted::saying(answers).translating(translations);
    let calls = Arc::clone(&p.calls);
    let sent = Arc::clone(&p.sent);
    let id = p.id();
    Driver {
        registry: Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
        sent,
    }
}

/// One JSON answer naming the quote and the node it came from.
fn answer(label: &str, gist: &str, quote: &str, node: &str) -> String {
    format!(
        r#"{{"units":[{{"type":"claim","label":"{label}","gist":"{gist}","status":"cited",
            "quote":"{quote}","node":"{node}"}}]}}"#
    )
}

/// A scratch directory that removes itself.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir =
            std::env::temp_dir().join(format!("smysl-text-gate-{}-{tag}", std::process::id()));
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

/// A library holding `TEXT` as one expression, read by `txt/1`.
fn library(tag: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("a library");
    lib.add(
        &Input::new(TEXT.as_bytes()),
        &AddSpec::new("txt/1", "incident", "public-domain")
            .with_lang(smysl_core::ids::LangTag::new("en").expect("a tag")),
        &Caps::DEFAULT,
    )
    .expect("added the text");
    (dir, lib)
}

/// Apply a `--lang-policy` string to options, the way the CLI does.
///
/// The family and the language are two fields now (see `LangPolicy`), and a test that set only
/// the first would be testing a policy no CLI invocation can produce.
fn policied(o: IngestOptions, named: &str) -> IngestOptions {
    let (policy, lang) = smysl_ingest::LangPolicy::parse_with_lang(named).expect("a policy");
    match lang {
        Some(l) => o.with_pivot(l),
        None => o.with_lang_policy(policy),
    }
}

fn opts(passes: Passes) -> IngestOptions {
    IngestOptions::at_rung(Rung::Document)
        .with_passes(passes)
        .with_model("scripted")
        .with_max_output(256)
        .with_agent(smysl_core::AgentId::new("tool:text-gate").expect("an agent"))
}

fn ingest(
    lib: &mut Library,
    d: &Driver,
    passes: Passes,
    digest_budget: u32,
    reextract: bool,
) -> smysl_ingest::text::TextReport {
    ingest_with(lib, d, opts(passes), digest_budget, reextract)
}

/// As [`ingest`], with the options the caller built — which is how a policy gets in.
fn ingest_with(
    lib: &mut Library,
    d: &Driver,
    o: IngestOptions,
    digest_budget: u32,
    reextract: bool,
) -> smysl_ingest::text::TextReport {
    let mut t = TextIngest::new(
        lib,
        Target::Alias("incident".to_string()),
        Level::new("line").expect("a level"),
    )
    .with_digest_budget(digest_budget);
    t.reextract = reextract;
    Ingestor::new(&d.registry, o)
        .ingest_text(&mut t)
        .expect("the run reached the provider")
}

#[test]
fn a_quote_becomes_a_span_and_the_copy_set_is_exactly_the_six_fields() {
    let (_dir, mut lib) = library("copy-set");
    let d = driver(vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )]);
    let report = ingest(&mut lib, &d, Passes::Single, 0, false);

    assert!(report.windows >= 1, "{report:?}");
    assert!(report.spanned >= 1, "nothing was attached: {report:?}");
    assert_eq!(report.spanless, 0, "{:?}", report.diagnostics);

    let mid = lib.head("incident").expect("a head");
    let manifest = lib.store().manifest(&mid).expect("a manifest").clone();
    let tid = manifest.parts[0].tid;

    let staged: Vec<_> = lib
        .store()
        .units()
        .filter(|(_, u)| u.core.source.as_ref().is_some_and(|s| s.span().is_some()))
        .collect();
    assert!(!staged.is_empty(), "no unit carries a span");
    let (_, unit) = staged[0];
    let source = unit.core.source.as_ref().expect("a source");
    let (start, end) = source.span().expect("a span");
    assert_eq!(
        &TEXT[start as usize..end as usize],
        "the pool wait rose to 1.9 seconds",
        "the span does not name the bytes the quote came from",
    );
    // The copy set, and the reference carries the tid with the node's locator after `#`.
    assert_eq!(source.manifest(), Some(&mid));
    assert!(source.reference.starts_with(&tid.canonical()));
    assert!(source.reference.ends_with("#L1"), "{}", source.reference);
    assert_eq!(unit.core.lang.as_ref().map(|l| l.as_str()), Some("en"));
    // The quote left the payload, because the span is better than it (OQ-41).
    assert_eq!(smysl_core::quote::quote_of(&unit.core), None);
}

#[test]
fn the_journal_makes_a_second_run_do_nothing_and_reextract_do_it_again() {
    let (_dir, mut lib) = library("journal");
    let answers = vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )];

    let first = driver(answers.clone());
    let one = ingest(&mut lib, &first, Passes::Single, 0, false);
    let calls_one = first.calls.load(Ordering::SeqCst);
    assert!(calls_one > 0);
    assert_eq!(one.skipped, 0);

    // Every window is now journalled under this recipe, so a second run makes no call at all.
    let second = driver(answers.clone());
    let two = ingest(&mut lib, &second, Passes::Single, 0, false);
    assert_eq!(second.calls.load(Ordering::SeqCst), 0, "{two:?}");
    assert_eq!(two.skipped, two.windows, "{two:?}");

    // `--reextract` ignores the journal for this expression, and the same answers produce the
    // same uids — so the corpus does not grow, which is content addressing rather than luck.
    let before = lib.store().units().count();
    let third = driver(answers);
    let three = ingest(&mut lib, &third, Passes::Single, 0, true);
    assert!(third.calls.load(Ordering::SeqCst) > 0, "{three:?}");
    assert_eq!(three.skipped, 0);
    assert_eq!(
        lib.store().units().count(),
        before,
        "re-extraction duplicated units"
    );
}

#[test]
fn a_kill_between_the_commit_and_the_journal_costs_one_window() {
    let (dir, mut lib) = library("resume");
    let answers = vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )];
    let first = driver(answers.clone());
    let one = ingest(&mut lib, &first, Passes::Single, 0, false);
    assert!(one.windows >= 2, "the budget gave one window: {one:?}");

    // The kill: the last journal entry was half written when the process died.
    let path = dir.path().join(smysl_ingest::text::journal::FILE);
    let bytes = std::fs::read(&path).expect("a journal");
    std::fs::write(&path, &bytes[..bytes.len() - 5]).expect("truncated");

    let resumed = Journal::open(dir.path());
    assert_eq!(
        resumed.len(),
        one.windows - 1,
        "a truncated tail lost more than the window in flight",
    );

    let second = driver(answers);
    let two = ingest(&mut lib, &second, Passes::Single, 0, false);
    assert_eq!(two.skipped, one.windows - 1);
    assert_eq!(
        two.windows - two.skipped,
        1,
        "the resume re-sent more than one window",
    );
}

#[test]
fn the_second_window_is_sent_a_digest_of_what_the_first_one_produced() {
    let (_dir, mut lib) = library("digest");
    let answers = vec![
        answer(
            "c/pool",
            "the pool wait rose",
            "the pool wait rose to 1.9 seconds",
            "L1",
        ),
        answer(
            "c/rollback",
            "the rollback fixed it",
            "the rollback returned p99 to baseline",
            "L2",
        ),
    ];
    let d = driver(answers);
    let report = ingest(&mut lib, &d, Passes::Single, 512, false);
    assert!(report.windows >= 2, "{report:?}");

    let sent = d.sent.lock().unwrap();
    assert!(sent.len() >= 2);
    // The first window's digest section is **present and empty**: the template is one per run
    // because the recipe is, so the fence is always there and what varies is what is inside it.
    // "Nothing has been extracted yet" is what the first window of every expression means.
    let first = prompt(&sent[0]);
    assert!(first.contains("<<<SMYSL-DIGEST-"));
    assert!(
        !first.contains("l/n1: "),
        "the first window was told about a record nothing had extracted: {first}",
    );
    let later = sent
        .iter()
        .skip(1)
        .find(|r| prompt(r).contains("l/n1: "))
        .expect("no window was sent a digest with anything in it");
    let later = prompt(later);
    assert!(
        later.contains("l/n1: "),
        "the digest has no digest-local label: {later}"
    );
    assert!(
        later.contains("the pool wait rose"),
        "the digest does not carry the first window's gist",
    );
    // Two fences, each twice, and they are different markers.
    let digest_marker = later
        .split("<<<SMYSL-DIGEST-")
        .nth(1)
        .map(|s| s.split(">>>").next().unwrap_or_default().to_string())
        .expect("a digest marker");
    let input_marker = later
        .split("<<<SMYSL-INPUT-")
        .nth(1)
        .map(|s| s.split(">>>").next().unwrap_or_default().to_string())
        .expect("an input marker");
    assert_ne!(digest_marker, input_marker);
    assert_eq!(
        later
            .matches(&format!("<<<SMYSL-DIGEST-{digest_marker}>>>"))
            .count(),
        2,
    );
    assert_eq!(
        later
            .matches(&format!("<<<SMYSL-INPUT-{input_marker}>>>"))
            .count(),
        2,
    );
}

#[test]
fn the_window_is_sent_with_its_locators_and_the_quote_is_checked_without_them() {
    let (_dir, mut lib) = library("rendered");
    let d = driver(vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )]);
    let report = ingest(&mut lib, &d, Passes::Single, 0, false);
    assert_eq!(report.spanless, 0, "{:?}", report.diagnostics);

    let sent = d.sent.lock().unwrap();
    assert!(
        prompt(&sent[0]).contains("[L1] the pool wait rose"),
        "the window was not rendered with its locators: {}",
        prompt(&sent[0])
    );
    // And the answer's quote, which has no `[L1]` in it, still attached — which is the whole
    // reason the rendered window and the searched text are two strings.
    assert!(report.spanned >= 1);
}

#[test]
fn two_passes_is_what_the_text_path_takes_and_both_are_staged() {
    let (_dir, mut lib) = library("passes");
    // One answer per call, in order: window 1 twice, then window 2 twice. A quote that is not
    // in the window it is answered against would buy repair turns and the call count would
    // then measure the repairs instead of the passes.
    let first = answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    );
    let second = answer(
        "c/rollback",
        "the rollback fixed it",
        "the rollback returned p99 to baseline",
        "L2",
    );
    let d = driver(vec![first.clone(), first, second.clone(), second]);
    let report = ingest(&mut lib, &d, Passes::TEXT, 0, false);
    assert_eq!(report.passes, 2);
    // Two calls per window, and one unit per pass pair: the two passes agreed, and agreement is
    // one uid rather than two records (§3.4).
    assert_eq!(report.calls, report.windows * 2, "{report:?}");
    assert_eq!(report.spanned, report.windows * 2, "{report:?}");
    assert_eq!(
        report.units, report.windows,
        "the agreeing pass staged a second record"
    );
}

#[test]
fn a_tool_owned_field_in_the_answer_is_dropped_and_reported() {
    let (_dir, mut lib) = library("w444");
    let d = driver(vec![r#"{"units":[{"type":"claim","label":"c/pool",
            "gist":"the pool wait rose","status":"cited",
            "quote":"the pool wait rose to 1.9 seconds","node":"L1",
            "observed":123,"text:speaker":"mallory"}]}"#
        .to_string()]);
    let report = ingest(&mut lib, &d, Passes::Single, 0, false);
    let w444 = report
        .diagnostics
        .iter()
        .filter(|x| x.code == Code::W444)
        .count();
    assert!(w444 >= 2, "{:?}", report.diagnostics);
    // And nothing the answer said about provenance reached the store: the text has no speaker,
    // so the staged unit has none either.
    for (_, unit) in lib.store().units() {
        let payload = unit
            .core
            .payload
            .as_deref()
            .map(smysl_core::surface::payload::payload_strings)
            .unwrap_or_default();
        assert!(
            !payload.contains_key("text:speaker"),
            "the answer's speaker was believed: {payload:?}",
        );
        if let Some(source) = unit.core.source.as_ref() {
            assert!(
                source.observed.is_none(),
                "the answer's `observed` was believed",
            );
        }
    }
}

#[test]
fn the_recipe_records_the_window_level_the_segmenter_and_the_reader() {
    let (_dir, mut lib) = library("recipe");
    let d = driver(vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )]);
    let one = ingest(&mut lib, &d, Passes::Single, 0, false);
    let recipe = one.recipe.expect("a recipe");

    // The same run at another window level is a different recipe, so the journal does not
    // consider the first run's windows done — which is the property the key is for.
    let d2 = driver(vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )]);
    let mut t = TextIngest::new(
        &mut lib,
        Target::Alias("incident".to_string()),
        Level::new("paragraph").expect("a level"),
    );
    let two = Ingestor::new(&d2.registry, opts(Passes::Single))
        .ingest_text(&mut t)
        .expect("a run");
    assert_ne!(two.recipe.expect("a recipe"), recipe);
    assert_eq!(two.skipped, 0, "another level counted as already done");
}

#[test]
fn a_journal_key_names_the_owned_range_and_not_what_the_window_was_shown() {
    let (dir, mut lib) = library("key");
    let d = driver(vec![answer(
        "c/pool",
        "the pool wait rose",
        "the pool wait rose to 1.9 seconds",
        "L1",
    )]);
    let report = ingest(&mut lib, &d, Passes::Single, 0, false);
    let journal = Journal::open(dir.path());
    assert_eq!(journal.len(), report.windows - report.skipped);

    let mid = lib.head("incident").expect("a head");
    let tid = lib.store().manifest(&mid).expect("a manifest").parts[0].tid;

    let mut ranges: Vec<(u64, u64)> = journal.entries().map(|e| e.key.owned).collect();
    ranges.sort();
    assert_eq!(ranges.len(), report.windows, "one entry per window");
    assert_eq!(
        ranges[0].0, 0,
        "the first window does not start at the part"
    );
    // The owned ranges partition the text: each window picks up where the one before it left
    // off, with the line separator between them. The *shown* ranges overlap by a node and these
    // do not, which is what keeps a window from reporting its neighbour's node as done.
    for pair in ranges.windows(2) {
        assert_eq!(
            pair[1].0,
            pair[0].1 + 1,
            "the owned ranges do not partition the part: {ranges:?}",
        );
    }
    assert!(
        ranges.last().expect("a range").1 <= TEXT.len() as u64,
        "a window owns bytes the part does not have",
    );

    // And a key over the whole part is not one of them: the key is the window's owned range,
    // not the part, or every window of a part would collide with every other.
    let whole = Key::new(
        "incident",
        tid,
        (0, TEXT.len() as u64),
        report.recipe.expect("a recipe"),
    );
    assert!(!journal.contains(&whole));

    let entry = journal
        .entries()
        .find(|e| e.key.owned == ranges[0])
        .expect("the first window's entry");
    assert!(entry.input_tokens > 0);
    assert_eq!(entry.verdicts.total(), 1, "one quote, one verdict");
    assert_eq!(entry.key.tid, tid);
}

// ---------------------------------------------------------------------------
// The pivot policy (§4.3.4, §5.2, TX-P5)
// ---------------------------------------------------------------------------

/// The three quotes, one per window, so every extraction lands a span of its own.
fn three_answers() -> Vec<String> {
    vec![
        answer(
            "c/pool",
            "the pool wait rose",
            "the pool wait rose to 1.9 seconds",
            "L1",
        ),
        answer(
            "c/rollback",
            "the rollback returned p99 to baseline",
            "the rollback returned p99 to baseline",
            "L2",
        ),
        answer(
            "c/shard",
            "the shard was never replaced",
            "the shard was never replaced",
            "L3",
        ),
    ]
}

/// `--lang-policy pivot:ru`: one unit per unit, in the language asked for, with the source in
/// `deps` and an `x.text/translates` edge back to it.
///
/// This is SMYSL-2.4 §5.2's clause for the policy, and the reason it is an end-to-end test
/// rather than a unit test of `text::pivot` is the part a unit test cannot reach: that the
/// translations are in the **same batch and the same commit** as the units they translate, so a
/// window is never journalled as done holding half of what it owes.
#[test]
fn the_pivot_policy_writes_a_translation_per_unit_with_the_source_in_deps() {
    let (_dir, mut lib) = library("pivot");
    let d = driver_translating(
        three_answers(),
        vec![
            "1\tзадержка пула выросла".to_string(),
            "1\tоткат вернул p99 к исходному уровню".to_string(),
            "1\tшард так и не заменили".to_string(),
        ],
    );
    let report = ingest_with(
        &mut lib,
        &d,
        opts(Passes::Single)
            .with_pivot(smysl_ingest::PivotLang::new("ru").expect("a language tag")),
        0,
        false,
    );

    let windows = report.windows;
    assert!(windows >= 2, "{report:?}");
    assert_eq!(report.pivoted, windows, "{:?}", report.diagnostics);
    assert_eq!(report.pivot_missing, 0);
    assert_eq!(
        report.pivot_calls, windows,
        "one call per window, and no repair turn"
    );

    // The pass is not a pass of the extraction, so it is not a repair turn either: one
    // extraction call and one pivot call per window, and a retry rate of nothing repaired.
    assert_eq!(report.calls, windows * 2, "{report:?}");
    assert_eq!(report.retries, 0);
    assert_eq!(report.retry_rate(), Some(0.0));

    let translated: Vec<_> = lib
        .store()
        .units()
        .filter(|(_, u)| u.core.lang.as_ref().is_some_and(|l| l.as_str() == "ru"))
        .collect();
    assert_eq!(
        translated.len(),
        windows,
        "one Russian unit per English one"
    );

    let edges = lib
        .store()
        .relations_of_kind(&smysl_core::RelKind::Extension(
            smysl_core::schemas::text::TRANSLATES.to_string(),
        ));
    assert_eq!(edges.len(), windows, "an edge per translation");

    // And the declaration the edge needs, or every check of this library afterwards reports
    // `SMY-W013` about a kind this build does in fact define (A-11).
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.code == smysl_core::Code::W013)
            .count(),
        0,
        "{:?}",
        report.diagnostics
    );

    for (uid, unit) in &translated {
        // Rule M reads `deps`, a reader reads the edge, and both have to name the same unit.
        let edge = edges
            .iter()
            .find(|e| &e.from == *uid)
            .expect("the translation has an edge of its own");
        assert!(
            unit.core.deps.contains(&edge.to),
            "the source of the translation is not in its deps"
        );
        let source = lib.store().get(&edge.to).expect("the source unit");
        assert_eq!(
            source.core.lang.as_ref().map(|l| l.as_str()),
            Some("en"),
            "the edge points at the unit in the passage's language"
        );
        // The place came from the source unit and not from the model, which is why the pivot
        // prompt never mentions one.
        assert_eq!(
            unit.core.source.as_ref().and_then(|s| s.span()),
            source.core.source.as_ref().and_then(|s| s.span()),
            "the translation is a claim about the same bytes"
        );
    }
}

/// The recipe records the language, not just the policy.
///
/// `pivot:en` and `pivot:ru` are two different things to have asked for, and a recipe that
/// called both `pivot` would let two incomparable corpora corroborate each other (§3.5).
#[test]
fn two_pivot_languages_are_two_recipes_and_source_is_a_third() {
    let mut recipes = Vec::new();
    for policy in ["source", "pivot:en", "pivot:ru"] {
        let (_dir, mut lib) = library(&format!("recipe-{}", policy.replace(':', "-")));
        let d = driver_translating(three_answers(), vec!["1\tsomething".to_string()]);
        let report = ingest_with(
            &mut lib,
            &d,
            policied(opts(Passes::Single), policy),
            0,
            false,
        );
        recipes.push(report.recipe.expect("a recipe"));
    }
    assert_ne!(
        recipes[0], recipes[1],
        "`source` and `pivot:en` are one recipe"
    );
    assert_ne!(
        recipes[1], recipes[2],
        "`pivot:en` and `pivot:ru` are one recipe"
    );
}

/// The control: a pass that returns nothing leaves the extraction alone and says so.
///
/// There is no repair turn for a missing line — see `Ingestor::pivot_window` — so what a reader
/// gets is the units in their own language and one `SMY-W453` per unit that was not translated.
/// The alternative, a silently untranslated store under a recipe that says `pivot:ru`, is the
/// failure F-6's policy exists to rule out.
#[test]
fn control_a_pivot_pass_that_returns_nothing_reports_every_unit_it_did_not_translate() {
    let (_dir, mut lib) = library("pivot-silent");
    let d = driver_translating(three_answers(), vec![String::new()]);
    let report = ingest_with(
        &mut lib,
        &d,
        opts(Passes::Single)
            .with_pivot(smysl_ingest::PivotLang::new("ru").expect("a language tag")),
        0,
        false,
    );

    assert_eq!(report.pivoted, 0);
    assert_eq!(report.pivot_missing, report.windows, "{report:?}");
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|d| d.code == smysl_core::Code::W453)
            .count(),
        report.windows,
    );
    assert!(
        report.units >= report.windows,
        "the extraction was kept: {report:?}"
    );
    assert!(
        lib.store()
            .relations_of_kind(&smysl_core::RelKind::Extension(
                smysl_core::schemas::text::TRANSLATES.to_string(),
            ))
            .is_empty(),
        "an untranslated unit got an edge anyway"
    );
}

/// A document ingest has nothing to attach a translation to, and says so before egress.
#[test]
fn a_document_ingest_refuses_the_policy_instead_of_quietly_extracting_in_the_passage() {
    let (_dir, lib) = library("pivot-document");
    let d = driver(three_answers());
    let e = Ingestor::new(
        &d.registry,
        opts(Passes::Single)
            .with_pivot(smysl_ingest::PivotLang::new("ru").expect("a language tag")),
    )
    .ingest(lib.store(), "the pool wait rose to 1.9 seconds.")
    .expect_err("a document ingest cannot pivot");
    assert!(format!("{e}").contains("pivot:ru"), "{e}");
    assert_eq!(
        d.calls.load(Ordering::SeqCst),
        0,
        "it was refused before egress"
    );
}
