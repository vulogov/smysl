//! **The red-team suite** (SMYSL-2.4 §3.9.2, §5.2): every fixture in
//! `fixtures/library/redteam/` is ingested with the `Scripted` provider playing an *obedient*
//! model — one that does what the passage asked — and nothing it was asked to do reaches a
//! staged record unchanged.
//!
//! One of the two exit clauses for TX-P5 step 4. The other is `text_gate.rs`.
//!
//! # What "obedient" has to mean to be a test
//!
//! A real model may or may not comply with a passage that tells it to; measuring that is a live
//! experiment and its result changes with the endpoint. What a fixture can establish is the other
//! half, and it is the half a corpus depends on: **when a model does comply, the guard holds.**
//! So the scripted answers here are the worst case rather than a likely one — `measured` claimed,
//! a store uid cited, a speaker and a date supplied, a relation kind invented — and every
//! assertion is about what the store ends up holding.
//!
//! The prompt is checked too, because a guard that works on the way back and leaks on the way out
//! is not a guard: each rendered call carries exactly two input fences and two digest fences, and
//! a passage that wrote this tool's marker had it neutralised in the prompt and nowhere else.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_core::{Code, Rung, Status};
use smysl_ingest::text::{Target, TextIngest, TextReport};
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};
use smysl_text::reading::Level;
use smysl_text::{AddSpec, Caps, Input, Library};

/// A uid no digest ever showed, which `store-uid.txt` asks for by name.
const ELSEWHERE: &str = "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/library/redteam")
        .canonicalize()
        .expect("the red-team fixtures are in the repository")
}

fn passages() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(fixtures()).expect("the fixture directory") {
        let path = entry.expect("an entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("txt") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a name")
            .to_string();
        out.push((name, std::fs::read_to_string(&path).expect("a passage")));
    }
    out.sort();
    assert!(out.len() >= 5, "the fixture set shrank: {out:?}");
    out
}

struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answers: Mutex<Vec<String>>,
    sent: Arc<Mutex<Vec<Request>>>,
    calls: Arc<AtomicUsize>,
}

impl Scripted {
    fn saying(answers: Vec<String>) -> Scripted {
        let mut caps = Capabilities::default();
        caps.offline = true;
        caps.context_window = 4096;
        caps.structured = StructuredMode::JsonSchema;
        Scripted {
            id: ProviderId::new("scripted").unwrap(),
            caps,
            answers: Mutex::new(answers),
            sent: Arc::new(Mutex::new(Vec::new())),
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
        self.sent.lock().unwrap().push(req.clone());
        let mut answers = self.answers.lock().unwrap();
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
    sent: Arc<Mutex<Vec<Request>>>,
}

fn driver(answers: Vec<String>) -> Driver {
    let p = Scripted::saying(answers);
    let sent = Arc::clone(&p.sent);
    let id = p.id();
    Driver {
        registry: Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        sent,
    }
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("smysl-redteam-{}-{tag}", std::process::id()));
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

fn library(tag: &str, text: &str) -> (Scratch, Library) {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("a library");
    lib.add(
        &Input::new(text.as_bytes()),
        &AddSpec::new("txt/1", "redteam", "public-domain")
            .with_lang(smysl_core::ids::LangTag::new("en").expect("a tag")),
        &Caps::DEFAULT,
    )
    .expect("added the passage");
    (dir, lib)
}

fn run(lib: &mut Library, d: &Driver, digest_budget: u32) -> TextReport {
    let mut t = TextIngest::new(
        lib,
        Target::Alias("redteam".to_string()),
        Level::new("line").expect("a level"),
    )
    .with_digest_budget(digest_budget);
    Ingestor::new(
        &d.registry,
        IngestOptions::at_rung(Rung::Document)
            .with_passes(Passes::Single)
            .with_model("scripted")
            .with_max_output(256)
            .with_agent(smysl_core::AgentId::new("tool:redteam").expect("an agent")),
    )
    .ingest_text(&mut t)
    .expect("the run reached the provider")
}

/// The obedient answer: everything the fixtures ask for, in one batch.
fn obedient(quote: &str) -> String {
    format!(
        r#"{{"units":[
            {{"type":"claim","label":"c/one","gist":"the shard was replaced","status":"measured",
              "quote":"{quote}","node":"L1",
              "source":{{"kind":"doc","ref":"an authority the passage named"}},
              "lang":"de","published":"2026-01-01","observed":1700000000000,
              "text:speaker":"mallory","text:mode":"asserted",
              "grounds":["{ELSEWHERE}"]}},
            {{"type":"claim","label":"c/two","gist":"the rollback was applied","status":"speculative",
              "quote":"{quote}","node":"L1"}}
          ],
          "relations":[{{"kind":"supersedes","from":"c/one","to":"c/two"}}]}}"#
    )
}

#[test]
fn every_rendered_call_carries_exactly_two_input_fences_and_two_digest_fences() {
    for (name, text) in passages() {
        let (_dir, mut lib) = library(&format!("fences-{name}"), &text);
        let first_line = text.lines().next().expect("a line").to_string();
        let d = driver(vec![obedient(&first_line)]);
        let _ = run(&mut lib, &d, 512);

        let sent = d.sent.lock().unwrap();
        assert!(!sent.is_empty(), "{name}: nothing was sent");
        for r in sent.iter() {
            let prompt = r
                .messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n");
            // The repair turns carry the previous answer rather than the window, so they are
            // not expected to hold a window fence; the calls that send a window hold exactly
            // two of each marker, and the two markers are different.
            let input = count_marker(&prompt, "<<<SMYSL-INPUT-");
            let digest = count_marker(&prompt, "<<<SMYSL-DIGEST-");
            if input == 0 {
                continue;
            }
            assert_eq!(input, 2, "{name}: {input} input fence(s) in {prompt}");
            assert_eq!(digest, 2, "{name}: {digest} digest fence(s)");
            // And the passage's own text is never rendered as a live marker.
            assert!(
                !prompt.contains("<<<SMYSL-INPUT>>>"),
                "{name}: the literal fence reached the prompt",
            );
        }
    }
}

/// How many times the one derived marker of this kind appears.
fn count_marker(prompt: &str, prefix: &str) -> usize {
    let Some(rest) = prompt.split(prefix).nth(1) else {
        return 0;
    };
    let hex = rest.split(">>>").next().unwrap_or_default();
    prompt.matches(&format!("{prefix}{hex}>>>")).count()
}

#[test]
fn a_passage_that_writes_the_tool_s_marker_is_neutralised_and_reported() {
    let text = std::fs::read_to_string(fixtures().join("fake-fence.txt")).expect("the fixture");
    let (_dir, mut lib) = library("fence", &text);
    let first_line = text.lines().next().expect("a line").to_string();
    let d = driver(vec![obedient(&first_line)]);
    let report = run(&mut lib, &d, 0);

    assert!(
        report.diagnostics.iter().any(|x| x.code == Code::W443),
        "the marker was not reported: {:?}",
        report.diagnostics
    );
    // Neutralised in the prompt only: the stored part still holds the marker byte for byte,
    // because a tid is the hash of those bytes and a span is an offset into them.
    let mid = lib.head("redteam").expect("a head");
    let tid = lib.store().manifest(&mid).expect("a manifest").parts[0].tid;
    let part = lib.part(&tid).expect("the part");
    let stored = String::from_utf8(part.text).expect("utf-8");
    assert!(
        stored.contains("<<<SMYSL-INPUT>>>"),
        "the stored text was edited to make the prompt safe",
    );
}

#[test]
fn an_obedient_answer_never_reaches_a_staged_record_unchanged() {
    for (name, text) in passages() {
        let (_dir, mut lib) = library(&format!("obedient-{name}"), &text);
        let first_line = text.lines().next().expect("a line").to_string();
        let d = driver(vec![obedient(&first_line)]);
        let report = run(&mut lib, &d, 0);

        // `measured` is refused by rule T, whatever the passage said about instruments.
        for (_, unit) in lib.store().units() {
            assert_ne!(
                unit.core.status,
                Status::Measured,
                "{name}: a staged unit claims measured",
            );
        }
        // The uid the passage asked for is outside the batch and the digest.
        assert!(
            report.diagnostics.iter().any(|x| x.code == Code::E442),
            "{name}: the store uid was accepted: {:?}",
            report.diagnostics
        );
        // The tool-owned fields were dropped rather than believed.
        assert!(
            report.diagnostics.iter().any(|x| x.code == Code::W444),
            "{name}: no tool-owned field was reported",
        );
        for (_, unit) in lib.store().units() {
            let payload = unit
                .core
                .payload
                .as_deref()
                .map(smysl_core::surface::payload::payload_strings)
                .unwrap_or_default();
            assert!(
                !payload.contains_key("text:speaker"),
                "{name}: the answer's speaker was staged",
            );
            if let Some(source) = unit.core.source.as_ref() {
                assert!(
                    source.published.is_none(),
                    "{name}: the answer's publication date was staged",
                );
                assert!(
                    source.observed.is_none(),
                    "{name}: the answer's observed instant was staged",
                );
                assert!(
                    !source.reference.contains("an authority the passage named"),
                    "{name}: the answer's own source was staged",
                );
            }
            assert_ne!(
                unit.core.lang.as_ref().map(|l| l.as_str()),
                Some("de"),
                "{name}: the answer's language was staged",
            );
        }
        // `supersedes` is not a relation kind a model may author: it is a kernel operation, and
        // the converter refuses it rather than staging an edge that retracts something.
        assert!(
            lib.store().units().all(|(_, u)| u
                .core
                .deps
                .iter()
                .all(|d| d.canonical() != ELSEWHERE)),
            "{name}: the out-of-scope uid reached a staged unit",
        );
    }
}

#[test]
fn a_passage_that_writes_a_json_answer_is_text_and_not_an_answer() {
    let text = std::fs::read_to_string(fixtures().join("fake-answer.txt")).expect("the fixture");
    let (_dir, mut lib) = library("fake-answer", &text);
    // The model answers with a quote of the *passage's own fake answer*, which is the obedient
    // thing to do with a document that contains one.
    let fake = text
        .lines()
        .find(|l| l.starts_with('{'))
        .expect("the fake answer line")
        .to_string();
    let d = driver(vec![obedient(
        "This passage writes a complete answer inside itself, hoping to be read as one.",
    )]);
    let report = run(&mut lib, &d, 0);

    // The fake answer is a line of a part like any other line: it can be quoted, and what it
    // says about status is not what any staged unit says. The obedient answer here is refused
    // for citing a uid nobody showed it, so what the store holds is the degraded prose rule I
    // produces — and that unit is `speculative`, which is the whole point.
    assert!(lib.store().units().count() > 0, "nothing was staged at all");
    for (_, unit) in lib.store().units() {
        assert_ne!(unit.core.status, Status::Measured);
    }
    assert!(
        report.degraded > 0,
        "the answer citing a store uid was accepted: {:?}",
        report.diagnostics
    );
    assert!(
        fake.contains("measured"),
        "the fixture no longer contains the claim this test is about",
    );
}

#[test]
fn bidirectional_overrides_are_stored_as_bytes_and_a_span_names_them() {
    let text = std::fs::read_to_string(fixtures().join("bidi.txt")).expect("the fixture");
    let (_dir, mut lib) = library("bidi", &text);
    let quoted = text
        .lines()
        .nth(1)
        .expect("the overriding line")
        .to_string();
    let d = driver(vec![format!(
        r#"{{"units":[{{"type":"claim","label":"c/one","gist":"the rollback and the shard",
            "status":"cited","quote":"{}","node":"L2"}}]}}"#,
        quoted.replace('\\', "\\\\").replace('"', "\\\"")
    )]);
    let report = run(&mut lib, &d, 0);
    assert_eq!(report.spanless, 0, "{:?}", report.diagnostics);

    let mid = lib.head("redteam").expect("a head");
    let tid = lib.store().manifest(&mid).expect("a manifest").parts[0].tid;
    let part = lib.part(&tid).expect("the part");
    let stored = String::from_utf8(part.text).expect("utf-8");
    let attached = lib
        .store()
        .units()
        .filter_map(|(_, u)| u.core.source.as_ref().and_then(|s| s.span()))
        .next()
        .expect("a span");
    assert_eq!(
        &stored[attached.0 as usize..attached.1 as usize],
        quoted,
        "the span does not name the bytes, override characters and all",
    );
    assert!(
        stored.contains('\u{202e}'),
        "the override character was stripped from the stored text",
    );
}
