//! **Reply edges from a text ingest** (1.14.0): step 9 of `ingest_text`, the producer SMYSL-2.5
//! §6 TX-P8 step 2 recorded as owed.
//!
//! A chat long enough to take several windows, with a reply in one window to a message in an
//! earlier one, and a reply to a message the export does not hold. What is asserted is the
//! boundary's: which edges reach the store, that a rerun adds none, that rule E sees them as
//! reply constraints, and that the missing message is `SMY-W448` rather than silence.
//!
//! The provider answers from the window it was shown — one unit per message whose text is in
//! the prompt — so the units, and therefore the edges, do not depend on how the windowing
//! happened to fall.

#![cfg(feature = "model")]

use std::path::{Path, PathBuf};

use smysl_core::types::RelKind;
use smysl_core::{Code, Rung};
use smysl_ingest::text::{Target, TextIngest, TextReport};
use smysl_ingest::{IngestOptions, Ingestor, Passes};
use smysl_provider::{
    Capabilities, Completion, Probe, Provider, ProviderError, ProviderId, Registry, Request,
    StructuredMode, Task, Usage,
};
use smysl_text::reading::Level;
use smysl_text::time::constraints::{self, Free, FreeKind};
use smysl_text::{AddSpec, Caps, Input, Library, Subject};

/// Nine messages, each long enough that a window holds only a few. Message ids run 2 to 10;
/// id 5 answers id 2 and id 9 answers id 3, each across a window, and id 10 answers 99, which
/// the export does not hold.
fn chat() -> String {
    let who = ["ada", "bruno", "clare"];
    let replies = [(5, "2"), (9, "3"), (10, "99")];
    let mut messages = String::new();
    for i in 0..9usize {
        let id = i + 2;
        let text = format!(
            "message {i} said the pool wait rose and then said it again{}",
            " and the operators read it in the morning and it still said what it said".repeat(8),
        );
        let reply = replies
            .iter()
            .find(|(from, _)| *from == id)
            .map(|(_, to)| format!(r#", "reply_to_message_id": {to}"#))
            .unwrap_or_default();
        if i > 0 {
            messages.push_str(",\n");
        }
        messages.push_str(&format!(
            r#"  {{"id": {id}, "type": "message", "date": "2024-01-15T10:{i:02}:00",
   "date_unixtime": "{}", "from": "{w}", "from_id": "{w}", "text": "{text}"{reply}}}"#,
            1_705_314_000 + i * 60,
            w = who[i % who.len()],
        ));
    }
    format!("{{\n \"name\": \"Incident\",\n \"type\": \"private_group\",\n \"id\": 77,\n \"messages\": [\n{messages}\n ]\n}}\n")
}

/// A provider that extracts one unit per message it can see in the window.
struct Reading {
    id: ProviderId,
    caps: Capabilities,
}

impl Provider for Reading {
    fn id(&self) -> ProviderId {
        self.id.clone()
    }
    fn caps(&self) -> Capabilities {
        self.caps.clone()
    }
    fn complete(&self, req: &Request) -> Result<Completion, ProviderError> {
        let prompt: String = req
            .messages
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let units: Vec<String> = (0..9)
            .filter(|i| prompt.contains(&format!("message {i} said")))
            .map(|i| {
                format!(
                    r#"{{"type":"claim","label":"c/m{i}","gist":"message {i} reported the wait",
                        "status":"cited","quote":"message {i} said the pool wait rose"}}"#
                )
            })
            .collect();
        Ok(Completion::new(
            format!(r#"{{"units":[{}]}}"#, units.join(",")),
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

fn registry() -> Registry {
    let mut caps = Capabilities::default();
    caps.offline = true;
    caps.context_window = 2048;
    caps.structured = StructuredMode::JsonSchema;
    let p = Reading {
        id: ProviderId::new("scripted").unwrap(),
        caps,
    };
    let id = p.id();
    Registry::new()
        .with_provider(Box::new(p))
        .route(Task::ContentIngest, id)
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("smysl-replies-{}-{tag}", std::process::id()));
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

fn ingest(lib: &mut Library, registry: &Registry) -> TextReport {
    let mut t = TextIngest::new(
        lib,
        Target::Alias("incident".to_string()),
        Level::new("message").expect("a level"),
    )
    .with_digest_budget(0);
    let opts = IngestOptions::at_rung(Rung::Document)
        .with_passes(Passes::Single)
        .with_model("scripted")
        .with_max_output(256)
        .with_agent(smysl_core::AgentId::new("tool:replies").expect("an agent"));
    Ingestor::new(registry, opts)
        .ingest_text(&mut t)
        .expect("the run reached the provider")
}

/// `(from, to)` of every reply edge, as the gists of the two units.
fn edges(lib: &Library) -> Vec<(String, String)> {
    let kind = RelKind::Extension(smysl_core::schemas::text::REPLIES.to_string());
    let gist = |u| lib.store().get(u).expect("a unit").core.gist.clone();
    let mut out: Vec<(String, String)> = lib
        .store()
        .relations_of_kind(&kind)
        .into_iter()
        .map(|r| (gist(&r.from), gist(&r.to)))
        .collect();
    out.sort();
    out
}

#[test]
fn a_reply_links_its_units_to_what_it_answers_across_windows_and_once() {
    let dir = Scratch::new("ingest");
    let mut lib = Library::create(dir.path()).expect("a library");
    lib.add(
        &Input::new(chat().as_bytes()),
        &AddSpec::new("telegram/1", "incident", "public-domain")
            .with_lang(smysl_core::ids::LangTag::new("en").expect("a tag")),
        &Caps::DEFAULT,
    )
    .expect("the chat was added");
    let registry = registry();

    let first = ingest(&mut lib, &registry);
    assert!(
        first.windows >= 3,
        "one window tests no crossing: {first:?}"
    );
    assert_eq!(first.spanless, 0, "{:?}", first.diagnostics);
    assert_eq!(first.replies, 2, "{:?}", first.diagnostics);
    assert_eq!(
        edges(&lib),
        vec![
            (
                "message 3 reported the wait".to_string(),
                "message 0 reported the wait".to_string()
            ),
            (
                "message 7 reported the wait".to_string(),
                "message 1 reported the wait".to_string()
            ),
        ]
    );

    // The reply to a message the export does not hold is said, once, naming the id.
    let missing: Vec<_> = first
        .diagnostics
        .iter()
        .filter(|d| d.code == Code::W448)
        .collect();
    assert_eq!(missing.len(), 1, "{:?}", first.diagnostics);
    assert!(missing[0].message.contains("99"), "{}", missing[0].message);

    // Rule E reads them: the answered message is the earlier subject, at `cited`.
    let (free, unbuilt) = constraints::free(lib.store());
    assert!(!unbuilt.reply_needs_ingest);
    let replies: Vec<&Free> = free
        .iter()
        .filter(|f| {
            matches!(
                f,
                Free::Order {
                    kind: FreeKind::Reply,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(replies.len(), 2);
    for f in replies {
        let Free::Order { earlier, later, .. } = f else {
            unreachable!()
        };
        let (Subject::Unit(earlier), Subject::Unit(later)) = (earlier, later) else {
            panic!("a reply orders units: {f:?}")
        };
        let gist = |u| lib.store().get(u).expect("a unit").core.gist.clone();
        assert!(
            gist(earlier) < gist(later),
            "{} came before {}",
            gist(later),
            gist(earlier)
        );
    }

    // A rerun skips every window and finds every edge already there.
    let second = ingest(&mut lib, &registry);
    assert_eq!(second.skipped, second.windows, "{second:?}");
    assert_eq!(second.replies, 0, "{:?}", second.diagnostics);
    assert_eq!(edges(&lib).len(), 2);
}
