//! TX-P7 step 3's two passes, end to end: the linker and the summary tree (SMYSL-2.4 §6).
//!
//! The step's exit has two clauses. GE-T11 — recall over planted far contradictions — belongs
//! to the candidate generator and is measured where the generator is, in
//! `crates/smysl-text/tests/ge_t11.rs`, without a model and therefore on every `make test`.
//! The other clause is here: **summary nodes capped by rule M (asserted)**.
//!
//! Everything a model would say is said by a scripted provider, which is what makes these
//! assertions about this crate rather than about a vendor. What the script cannot fake is the
//! part that matters: the edges, the grounds, the identities and the cap are computed from the
//! answers by the code under test.
//!
//! # What would pass a weaker version of the rule M clause
//!
//! A summary staged as `derived` over `derived` grounds is capped by doing nothing. So the
//! chapter here has one `speculative` unit among its `cited` ones, the summariser claims
//! `derived` as it always does, and the assertion is that what reaches the batch is
//! `speculative` — the weakest ground and not the claim. The edges have to follow it, because
//! weakening moves the unit's uid, and an `x.text/summarises` edge still pointing at the uid
//! the summariser computed would be a dangling edge that every later check reports.

#![cfg(all(feature = "text", feature = "model"))]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use smysl_core::ids::{AgentId, LangTag, Uid};
use smysl_core::types::provenance::{Hlc, Rung};
use smysl_core::types::{Carry, RelKind};
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_ingest::link::{CONTRADICTS, ENTAILS};
use smysl_ingest::summarise::SUMMARISES;
use smysl_ingest::{IngestOptions, Ingestor};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};
use smysl_text::link::Budget;
use smysl_text::provenance::{self, Drawn};
use smysl_text::reading::Level;
use smysl_text::{AddSpec, Caps, Input, Library};

// ---------------------------------------------------------------------------
// A provider that says what the test tells it to.
// ---------------------------------------------------------------------------

struct Scripted {
    id: ProviderId,
    caps: Capabilities,
    answers: Mutex<Vec<Result<String, ()>>>,
    seen: Arc<Mutex<Vec<String>>>,
    calls: Arc<AtomicUsize>,
}

impl Scripted {
    fn saying(answers: Vec<Result<String, ()>>) -> Scripted {
        let mut caps = Capabilities::default();
        caps.offline = true;
        caps.context_window = 8192;
        caps.structured = StructuredMode::JsonSchema;
        Scripted {
            id: ProviderId::new("scripted").expect("a provider id"),
            caps,
            answers: Mutex::new(answers),
            seen: Arc::new(Mutex::new(Vec::new())),
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
        self.seen.lock().expect("the lock").push(
            req.messages
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<String>>()
                .join("\n"),
        );
        let mut answers = self.answers.lock().expect("the lock");
        let a = if answers.len() > 1 {
            answers.remove(0)
        } else {
            answers.first().cloned().unwrap_or(Ok(String::new()))
        };
        match a {
            Ok(text) => Ok(Completion::new(text, "scripted", Usage::reported(10, 20))),
            Err(()) => Err(ProviderError::Unreachable),
        }
    }
    fn probe(&self) -> Result<Probe, ProviderError> {
        Ok(Probe::reachable(
            vec!["scripted".into()],
            self.caps.clone(),
            "",
        ))
    }
}

/// The registry, the call counter, and what the provider was actually sent.
fn registry(p: Scripted) -> (Registry, Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::clone(&p.calls);
    let seen = Arc::clone(&p.seen);
    let id = p.id();
    (
        Registry::new()
            .with_provider(Box::new(p))
            .route(Task::ContentIngest, id),
        calls,
        seen,
    )
}

fn options() -> IngestOptions {
    IngestOptions::at_rung(Rung::Model)
        .with_agent(AgentId::new("model:scripted").expect("an agent id"))
        .with_now(Hlc::new(
            1_760_000_000_000,
            0,
            AgentId::new("model:scripted").expect("an agent id"),
        ))
}

// ---------------------------------------------------------------------------
// Scratch libraries.
// ---------------------------------------------------------------------------

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(what: &str) -> Scratch {
    use std::sync::atomic::AtomicU32;
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-link-{what}-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    Scratch(root)
}

/// A claim in English with no place: enough for the lexical source, which is all `link` needs.
///
/// `speculative`, because a unit with no source may not be `cited` — and the linker does not
/// read a unit's status anyway. What is being judged is what two gists say.
fn claim(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .lang(LangTag::new("en").expect("a tag"))
        .build()
        .expect("a unit")
}

// ---------------------------------------------------------------------------
// The linker.
// ---------------------------------------------------------------------------

/// Two claims that cannot both hold, judged, staged, and attested by the caller's agent.
#[test]
fn a_contradiction_the_model_names_becomes_one_edge_and_nothing_else_does() {
    // Three units: a pair that contradicts and a third that is merely nearby.
    let a = claim("the elders sat in the gate of the city");
    let b = claim("no elders were gathered and the gate stood empty");
    let c = claim("the elders of the city sat down in the gate at noon");
    let (ua, ub, uc) = (canonical_uid(&a), canonical_uid(&b), canonical_uid(&c));
    let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b), Record::Unit(c)]);

    // One turn, three pairs. Whatever order the generator offers them in, the script answers
    // `contradicts` for the first and `unrelated` for the rest — so the test asserts the
    // *shape* of what one `contradicts` produces, and that two `unrelated`s produce nothing.
    let (reg, calls, seen) = registry(Scripted::saying(vec![Ok(
        "1\tcontradicts\n2\tunrelated\n3\tunrelated\n".to_string(),
    )]));
    let report = Ingestor::new(&reg, options())
        .link(&store, None, None, Budget::DEFAULT, 10)
        .expect("the scripted provider is reachable");

    assert_eq!(calls.load(Ordering::SeqCst), 1, "three pairs is one turn");
    assert_eq!(report.candidates, 3, "three units are three pairs");
    assert_eq!(report.asked, 3);
    assert_eq!(report.already, 0);
    assert_eq!(report.contradicts, 1);
    assert_eq!(report.unrelated, 2);
    assert_eq!(report.unanswered, 0);
    assert_eq!(report.new_edges(), 1, "`unrelated` stages nothing");

    let prompt = seen.lock().expect("the lock")[0].clone();
    assert!(
        prompt.contains("<<<SMYSL-INPUT>>>") || prompt.contains("<<<SMYSL-"),
        "the gists go inside a fence: {prompt}"
    );

    let staged = report.staged.as_ref().expect("a batch");
    assert!(
        !staged.has_errors(),
        "the batch does not check: {:?}",
        staged.report.diagnostics
    );
    let edge = &staged.relations[0];
    assert_eq!(edge.kind.as_str(), CONTRADICTS);
    assert!(
        [ua, ub, uc].contains(&edge.from) && [ua, ub, uc].contains(&edge.to),
        "the edge joins two of the three units"
    );

    // One attestation, by the caller's agent, at the caller's rung. `model` and not
    // `computed`: a judgement is a reading, not arithmetic.
    let attestations: Vec<_> = staged
        .attestations
        .iter()
        .filter(|at| at.uid == edge.uid())
        .collect();
    assert_eq!(attestations.len(), 1);
    assert_eq!(attestations[0].agent.as_str(), "model:scripted");
    assert_eq!(attestations[0].rung, Rung::Model);
    assert!(
        attestations[0].recipe.is_some(),
        "the recipe records the budget the pairs were chosen under"
    );
}

/// An edge the store already holds is counted, not asked about again.
#[test]
fn a_rerun_over_a_store_that_already_holds_the_edge_asks_nothing() {
    let a = claim("the elders sat in the gate of the city");
    let b = claim("no elders were gathered and the gate stood empty");
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::Relation(smysl_core::Relation::new(
            RelKind::Extension(CONTRADICTS.to_string()),
            ua,
            ub,
        )),
    ]);

    let (reg, calls, _) = registry(Scripted::saying(vec![Ok("1\tcontradicts\n".to_string())]));
    let report = Ingestor::new(&reg, options())
        .link(&store, None, None, Budget::DEFAULT, 10)
        .expect("reachable");

    assert_eq!(report.candidates, 1);
    assert_eq!(report.already, 1);
    assert_eq!(report.asked, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was asked");
    assert!(report.staged.is_none());
    assert_eq!(report.new_edges(), 0);
}

/// Entailment is directional, and the two spellings produce two different edges.
#[test]
fn the_direction_of_an_entailment_is_the_direction_the_model_named() {
    let a = claim("every elder of the city was present in the gate");
    let b = claim("some elder of the city was present in the gate");
    let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
    let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

    // The generator orders a pair by uid, so `A` is the smaller uid and `B` the larger
    // whatever this test calls them. That is the whole reason the two spellings exist: the
    // model is answering about the pair as it was shown, not as the caller holds it.
    let (small, large) = if ua < ub { (ua, ub) } else { (ub, ua) };
    for (answer, expect) in [
        ("entails-ab", (small, large)),
        ("entails-ba", (large, small)),
    ] {
        let (reg, _, _) = registry(Scripted::saying(vec![Ok(format!("1\t{answer}\n"))]));
        let report = Ingestor::new(&reg, options())
            .link(&store, None, None, Budget::DEFAULT, 10)
            .expect("reachable");
        let staged = report.staged.as_ref().expect("a batch");
        assert_eq!(staged.relations.len(), 1);
        let edge = &staged.relations[0];
        assert_eq!(edge.kind.as_str(), ENTAILS);
        assert_eq!((edge.from, edge.to), expect, "`{answer}`");
    }
}

/// A turn that does not come back costs its pairs and not the run.
#[test]
fn a_provider_failure_is_a_diagnostic_and_the_other_turn_still_lands() {
    let mut records = Vec::new();
    for i in 0..4 {
        records.push(Record::Unit(claim(&format!(
            "the elders sat in the gate of the city on the {i}th day"
        ))));
    }
    let store = Store::from_records(records);

    // Four units are six pairs; one pair per turn makes six turns, and the first fails.
    let (reg, calls, _) = registry(Scripted::saying(vec![
        Err(()),
        Ok("1\tcontradicts\n".to_string()),
    ]));
    let report = Ingestor::new(&reg, options())
        .link(&store, None, None, Budget::DEFAULT, 1)
        .expect("the registry can route; the turn is what failed");

    assert_eq!(report.candidates, 6);
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    assert_eq!(report.turns, 6);
    assert_eq!(report.unanswered, 1, "the failed turn's pair");
    assert_eq!(report.contradicts, 5, "the other five answered");
    assert_eq!(report.new_edges(), 5);
    assert_eq!(report.diagnostics.len(), 1);
    assert_eq!(report.diagnostics[0].code, smysl_core::Code::W453);
}

/// A model answering about a pair it was not given stages nothing for it.
#[test]
fn an_answer_about_a_pair_that_was_not_sent_stages_nothing() {
    let a = claim("the elders sat in the gate of the city");
    let b = claim("no elders were gathered and the gate stood empty");
    let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);

    let (reg, _, _) = registry(Scripted::saying(vec![Ok(
        "7\tcontradicts\n9\tentails-ab\n".to_string(),
    )]));
    let report = Ingestor::new(&reg, options())
        .link(&store, None, None, Budget::DEFAULT, 10)
        .expect("reachable");

    assert_eq!(report.asked, 1);
    assert_eq!(report.unanswered, 1, "the one pair sent was not answered");
    assert_eq!(report.contradicts, 0);
    assert!(report.staged.is_none(), "nothing was said about pair 1");
}

// ---------------------------------------------------------------------------
// The summary tree.
// ---------------------------------------------------------------------------

/// A library holding one chapter of real scripture, and one unit per verse.
///
/// Returns the library, the alias, and the uid of the unit this test makes `speculative` —
/// the weakest ground, which is what rule M has to find.
fn chapter_library(root: &Path) -> (Library, String, Uid) {
    let mut lib = Library::create(root).expect("a library");
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/library/bibles/slice/eng-kjv/65-JUD.usfm");
    let raw = std::fs::read(&source).unwrap_or_else(|e| panic!("{}: {e}", source.display()));
    let alias = "kjv/jude".to_string();
    let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text);
    spec.versification = Some("kjv".to_string());
    let added = lib
        .add(&Input::new(&raw), &spec, &Caps::DEFAULT)
        .expect("Jude is added");
    let manifest = lib
        .store()
        .manifest(&added.mid)
        .expect("its manifest")
        .clone();

    let mut units = Vec::new();
    let mut weakest = None;
    for tid in &added.parts {
        let reading = lib.part_reading(&added.mid, tid).expect("the reading");
        for (i, row) in reading
            .rows
            .iter()
            .filter(|r| r.level.as_str() == "verse")
            .enumerate()
            // Four verses: enough for a group of more than one, few enough to read.
            .take(4)
        {
            let at = row.locator.to_string();
            let en = LangTag::new("en").expect("a tag");
            // The third is `speculative` and the rest are `cited`: one weak ground is what
            // rule M is for, and a chapter where every ground is weak would not tell a cap
            // from a coincidence.
            let status = if i == 2 {
                Status::Speculative
            } else {
                Status::Cited
            };
            let b = UnitCoreBuilder::new(
                KernelType::Claim,
                format!("verse {at} says something in particular"),
                status,
            );
            let unit = provenance::stamp(
                b,
                &Drawn {
                    tid: *tid,
                    locator: Some(&at),
                    manifest: &manifest,
                    observed: None,
                    span: Some((row.start, row.end)),
                    lang: Some(&en),
                },
            )
            .build()
            .expect("a unit of a verse");
            let uid = canonical_uid(&unit);
            if i == 2 {
                weakest = Some(uid);
            }
            units.push(Record::Unit(unit));
        }
    }
    lib.append_records(&units).expect("the units are written");
    (lib, alias, weakest.expect("a speculative ground"))
}

/// **TX-P7 step 3's second exit clause:** a summary node is capped by rule M.
#[test]
fn a_summary_is_grounded_on_its_children_and_capped_at_the_weakest_of_them() {
    let s = scratch("summarise");
    let (lib, alias, weak) = chapter_library(&s.0);

    let (reg, calls, seen) = registry(Scripted::saying(vec![Ok(
        "The chapter speaks of several particular things.".to_string(),
    )]));
    let levels = [Level::new("chapter").expect("a level")];
    let report = Ingestor::new(&reg, options())
        .summarise(&lib, &alias, &levels, None)
        .expect("reachable");

    assert_eq!(calls.load(Ordering::SeqCst), 1, "one chapter is one turn");
    assert_eq!(report.summaries, 1);
    assert_eq!(report.levels, vec![("chapter".to_string(), 1)]);

    // The four gists went in, numbered, inside the fence.
    let prompt = seen.lock().expect("the lock")[0].clone();
    assert!(
        prompt.contains("1\tverse"),
        "the children are sent: {prompt}"
    );
    assert!(prompt.contains("4\tverse"), "all of them: {prompt}");

    let staged = report.staged.as_ref().expect("a batch");
    assert!(
        !staged.has_errors(),
        "the batch does not check: {:?}",
        staged.report.diagnostics
    );
    assert_eq!(staged.units.len(), 1);
    let summary = &staged.units[0];

    // The clause.
    assert_eq!(
        summary.status,
        Status::Speculative,
        "rule M caps a summary at its weakest ground; it claimed `derived`"
    );
    assert_eq!(report.capped, 1, "and the run says it happened");
    assert!(
        summary.grounds.contains(&weak),
        "the weak verse is one of the grounds it was capped by"
    );
    assert_eq!(summary.grounds.len(), 4, "grounded on all four children");

    // And the edges followed the weakening. A `summarises` edge still pointing at the uid the
    // summariser computed before the cap would be an edge to a unit that was never written.
    let uid = canonical_uid(summary);
    let edges: Vec<_> = staged
        .relations
        .iter()
        .filter(|r| r.kind.as_str() == SUMMARISES)
        .collect();
    assert_eq!(edges.len(), 4, "one edge per child");
    for e in &edges {
        assert_eq!(e.from, uid, "the edge points at the unit that was staged");
        assert!(summary.grounds.contains(&e.to));
    }
}

/// A node with one child in scope is not summarised, and says so.
#[test]
fn a_node_with_one_child_is_not_summarised() {
    let s = scratch("thin");
    let (lib, alias, _) = chapter_library(&s.0);
    let one: BTreeSet<Uid> = lib.store().units().map(|(uid, _)| *uid).take(1).collect();

    let (reg, calls, _) = registry(Scripted::saying(vec![Ok("a summary".to_string())]));
    let levels = [Level::new("chapter").expect("a level")];
    let report = Ingestor::new(&reg, options())
        .summarise(&lib, &alias, &levels, Some(&one))
        .expect("reachable");

    assert_eq!(calls.load(Ordering::SeqCst), 0, "nothing was worth asking");
    assert_eq!(report.summaries, 0);
    assert_eq!(report.thin, 1);
    assert!(report.staged.is_none());
}
