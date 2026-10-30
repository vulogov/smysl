//! `--lang-policy source` is checked, not just stated (`SMY-W436`, MS-5).
//!
//! 1.9 shipped the policy and the prompt clause that states it, and nothing that told you
//! whether the model obeyed. The S0 spike measured both directions on one corpus: a local model
//! wrote 78.6% of its Russian gists in Latin script, and a hosted one wrote every gist of an
//! English chapter in Chinese across four runs of five. Both staged without a remark.
//!
//! These tests drive the whole pipeline rather than `smysl_core::lang::verify` directly, which
//! has its own unit tests. What is asserted here is the wiring: that the check sees the chunk's
//! own text, that it reaches the report, and — the one that costs money if it is wrong — that a
//! warning does not buy a repair turn.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use smysl_core::{Code, Rung};
use smysl_graph::Store;
use smysl_ingest::{IngestOptions, Ingestor, LangPolicy};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};

struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answer: String,
    calls: Arc<AtomicUsize>,
}

impl Scripted {
    fn saying(answer: &str) -> Scripted {
        let mut caps = Capabilities::default();
        caps.offline = true;
        caps.context_window = 8192;
        caps.structured = StructuredMode::JsonSchema;
        Scripted {
            id: ProviderId::new("scripted").unwrap(),
            caps,
            answer: answer.to_string(),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// A small context, so a two-paragraph document really does chunk.
    fn with_context(mut self, n: usize) -> Scripted {
        self.caps.context_window = n;
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
    fn complete(&self, _req: &Request) -> Result<Completion, ProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Completion::new(
            self.answer.clone(),
            "scripted",
            Usage::reported(10, 20),
        ))
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

/// A Russian passage, long enough to be one chunk and unambiguously Cyrillic.
const RUSSIAN: &str = "Шард eu-west замедлился в четверг после полудня. \
                       Время ожидания в пуле соединений выросло вместе с задержкой запросов.";

const ENGLISH: &str = "The eu-west shard slowed on Thursday afternoon. \
                       Connection pool wait time rose alongside request latency.";

fn answer(gist: &str) -> String {
    format!(
        r#"{{"units":[{{"type":"claim","label":"c/one","gist":"{gist}","status":"speculative"}}]}}"#
    )
}

/// How many of a code a diagnostic list carries. `IngestReport::diagnostics` is a plain `Vec`,
/// not a `Report`, so there is no `count` to borrow.
fn count(diags: &[smysl_core::Diagnostic], code: Code) -> usize {
    diags.iter().filter(|d| d.code == code).count()
}

fn ingest(passage: &str, gist: &str) -> (Vec<smysl_core::Diagnostic>, usize) {
    let (r, calls) = registry(Scripted::saying(&answer(gist)));
    let (_, report) = Ingestor::new(&r, IngestOptions::at_rung(Rung::Document))
        .ingest(&Store::new(), passage)
        .expect("a scripted provider cannot fail");
    let n = calls.load(Ordering::SeqCst);
    (report.diagnostics, n)
}

/// **The gate for MS-5, direction one.** The local model's measured failure: a Russian passage
/// answered in English. Before 1.10 this staged silently.
#[test]
fn a_latin_gist_of_a_russian_passage_is_reported() {
    let (diags, _) = ingest(RUSSIAN, "The connection pool wait time rose");
    assert_eq!(
        count(&diags, Code::W436),
        1,
        "a Russian passage answered in English went unremarked: {diags:?}"
    );
    let d = diags.iter().find(|d| d.code == Code::W436).unwrap();
    assert!(d.message.contains("Cyrillic"), "{}", d.message);
    assert!(d.message.contains("Latin"), "{}", d.message);
}

/// **The gate for MS-5, direction two.** The hosted model's measured failure: an English
/// chapter answered in Chinese.
#[test]
fn a_chinese_gist_of_an_english_passage_is_reported() {
    let (diags, _) = ingest(ENGLISH, "连接池的等待时间随着请求延迟一起上升");
    assert_eq!(count(&diags, Code::W436), 1, "{diags:?}");
}

/// The passage and the answer in one script is the ordinary case, and must stay silent.
#[test]
fn a_gist_in_the_passages_script_is_not_reported() {
    for (passage, gist) in [
        (RUSSIAN, "Время ожидания в пуле соединений выросло"),
        (ENGLISH, "The connection pool wait time rose"),
    ] {
        let (diags, _) = ingest(passage, gist);
        assert_eq!(count(&diags, Code::W436), 0, "{gist}: {diags:?}");
    }
}

/// **The one that costs money if it is wrong.** `SMY-W436` is a warning, so it must not spend a
/// repair turn: one call in, one call out. An error here would re-ask the model on the strength
/// of a code-point table, and would do it for every gist carrying a Latin loanword.
#[test]
fn a_wrong_script_does_not_buy_a_repair_turn() {
    let (diags, calls) = ingest(RUSSIAN, "The connection pool wait time rose");
    assert_eq!(count(&diags, Code::W436), 1, "the warning did fire");
    assert_eq!(calls, 1, "a warning bought {calls} calls; it must buy one");
    assert!(
        !diags
            .iter()
            .any(|d| d.severity == smysl_core::Severity::Error),
        "a wrong script is advisory, not fatal: {diags:?}"
    );
}

/// The check is per chunk, against *that* chunk's text — not against the document as a whole.
///
/// A bilingual document under a context small enough to split it, answered in Russian every
/// time: the Russian half is right and the English half is wrong, so exactly one chunk
/// complains. Were the comparison made against the whole document, the majority script would
/// decide both halves together and this would be 0 or 2 rather than 1.
#[test]
fn each_chunk_is_judged_against_its_own_text() {
    // Each half repeated until it is big enough that a 1400-token context cannot hold both —
    // the sizing the chunking test in `gate.rs` uses.
    let ru = [RUSSIAN; 6].join(" ");
    let en = [ENGLISH; 6].join(" ");
    let bilingual = format!("{ru}\n\n{en}");

    let (r, _) = registry(
        Scripted::saying(&answer("Время ожидания в пуле соединений выросло")).with_context(1400),
    );
    let (_, report) = Ingestor::new(
        &r,
        IngestOptions::at_rung(Rung::Document).with_max_output(64),
    )
    .ingest(&Store::new(), &bilingual)
    .expect("a scripted provider cannot fail");

    assert!(
        report.chunks > 1,
        "the fixture did not chunk, so this asserts nothing"
    );
    assert_eq!(
        count(&report.diagnostics, Code::W436),
        1,
        "the Russian half is right and the English half is wrong: {:?}",
        report.diagnostics
    );
}

/// `Source` is the only policy there is, and naming it explicitly must behave as the default
/// does — the check follows from the policy, so it needs no flag of its own.
#[test]
fn naming_the_policy_explicitly_changes_nothing() {
    let (r, _) = registry(Scripted::saying(&answer(
        "The connection pool wait time rose",
    )));
    let (_, report) = Ingestor::new(
        &r,
        IngestOptions::at_rung(Rung::Document).with_lang_policy(LangPolicy::Source),
    )
    .ingest(&Store::new(), RUSSIAN)
    .expect("a scripted provider cannot fail");
    assert_eq!(count(&report.diagnostics, Code::W436), 1);
}
