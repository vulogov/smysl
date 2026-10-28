//! Dossiers (TX-P8 step 3; SMYSL-2.5 §5.5, the unit bullet).
//!
//! §5.5 lists six things to assert: intent rules per tier-1 language with positive and negative
//! examples, `SMY-W417` on an ambiguous subject ("Mary"), `SMY-E468` on an unclassifiable
//! question, `gap` steps for empty slots, the view's membership reaching every slot result, and
//! rule R in the pack. Each has a test here, over `fixtures/dossier/subjects.smy`, which says
//! beside each unit which slot should find it.
//!
//! The store is plain, and `StoreScope` is its entity port: a name resolves to the
//! `x.text/entity` units spelled that way, and `same-as` joins them into one candidate. A
//! library answers the port with the same two functions, so this is the resolution `ask`
//! performs over a library too, without the catalogue a library adds.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{AgentId, Uid};
use smysl_core::surface::parse_surface;
use smysl_core::{canonical_uid, Code, Role};
use smysl_graph::{membership, Store};
use smysl_query::dossier::{classify, templates, Intent};
use smysl_query::{ask, AskError, AskOptions, Env, StoreScope};

const FIXTURE: &str = "../../fixtures/dossier/subjects.smy";

struct Fixture {
    store: Store,
    labels: BTreeMap<String, Uid>,
}

fn fixture() -> Fixture {
    let src = std::fs::read_to_string(FIXTURE).expect("the fixture");
    let parsed = parse_surface(&src).expect("the fixture parses");
    let labels = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    Fixture {
        store: Store::from_records(parsed.records),
        labels,
    }
}

impl Fixture {
    fn uid(&self, label: &str) -> Uid {
        self.labels[label]
    }
}

fn opts() -> AskOptions {
    AskOptions::new(AgentId::new("tool:test").expect("an agent"))
}

fn slot<'a>(d: &'a smysl_query::Dossier, name: &str) -> &'a smysl_query::SlotResult {
    d.slots.iter().find(|s| s.name == name).expect("the slot")
}

/// §5.5: intent rules per tier-1 language, positive examples.
#[test]
fn every_intent_is_recognised_in_every_tier_one_language() {
    let cases: &[(&str, &str, Intent, &str)] = &[
        ("en", "What is a covenant?", Intent::Define, "a covenant"),
        (
            "en",
            "Why did Naomi return?",
            Intent::Explain,
            "Naomi return",
        ),
        (
            "en",
            "Is it true that Ruth gleaned?",
            Intent::Verify,
            "Ruth gleaned",
        ),
        (
            "en",
            "Who said Naomi was bitter?",
            Intent::Attribute,
            "Naomi was bitter",
        ),
        (
            "en",
            "Where did the levirate come from?",
            Intent::Trace,
            "the levirate",
        ),
        ("en", "Where is Moab mentioned?", Intent::Locate, "Moab"),
        ("en", "What depends on Boaz?", Intent::Depend, "Boaz"),
        (
            "en",
            "Compare Ruth and Orpah",
            Intent::Compare,
            "Ruth and Orpah",
        ),
        ("ru", "Что такое завет?", Intent::Define, "завет"),
        (
            "ru",
            "Почему Ноеминь вернулась?",
            Intent::Explain,
            "Ноеминь вернулась",
        ),
        (
            "ru",
            "Правда ли, что Руфь собирала колосья?",
            Intent::Verify,
            "Руфь собирала колосья",
        ),
        (
            "ru",
            "Кто сказал, что Ноеминь горька?",
            Intent::Attribute,
            "Ноеминь горька",
        ),
        ("ru", "Откуда пришла Руфь?", Intent::Trace, "пришла Руфь"),
        ("ru", "Где упоминается Моав?", Intent::Locate, "Моав"),
        ("ru", "Что зависит от Вооза?", Intent::Depend, "Вооза"),
        ("ru", "Сравни Руфь и Орфу", Intent::Compare, "Руфь и Орфу"),
        ("de", "Was ist ein Bund?", Intent::Define, "ein Bund"),
        (
            "de",
            "Warum kehrte Noomi zurück?",
            Intent::Explain,
            "kehrte Noomi zurück",
        ),
        (
            "de",
            "Stimmt es, dass Rut Ähren las?",
            Intent::Verify,
            "Rut Ähren las",
        ),
        ("de", "Wer sagte das?", Intent::Attribute, "das"),
        ("de", "Woher kommt Rut?", Intent::Trace, "Rut"),
        ("de", "Wo wird Moab erwähnt?", Intent::Locate, "Moab"),
        ("de", "Was hängt von Boas ab?", Intent::Depend, "Boas"),
        (
            "de",
            "Vergleiche Rut und Orpa",
            Intent::Compare,
            "Rut und Orpa",
        ),
        (
            "fr",
            "Qu'est-ce qu'une alliance ?",
            Intent::Define,
            "une alliance",
        ),
        (
            "fr",
            "Pourquoi Naomi est-elle revenue ?",
            Intent::Explain,
            "Naomi est-elle revenue",
        ),
        (
            "fr",
            "Est-il vrai que Ruth glanait ?",
            Intent::Verify,
            "Ruth glanait",
        ),
        ("fr", "Qui a dit cela ?", Intent::Attribute, "cela"),
        ("fr", "D'où vient Ruth ?", Intent::Trace, "Ruth"),
        ("fr", "Où se trouve Moab ?", Intent::Locate, "Moab"),
        ("fr", "Qui dépend de Boaz ?", Intent::Depend, "Boaz"),
        (
            "fr",
            "Comparez Ruth et Orpa",
            Intent::Compare,
            "Ruth et Orpa",
        ),
        ("es", "¿Qué es una alianza?", Intent::Define, "una alianza"),
        (
            "es",
            "¿Por qué volvió Noemí?",
            Intent::Explain,
            "volvió Noemí",
        ),
        (
            "es",
            "¿Es cierto que Rut espigaba?",
            Intent::Verify,
            "Rut espigaba",
        ),
        ("es", "¿Quién dijo eso?", Intent::Attribute, "eso"),
        ("es", "¿De dónde viene Rut?", Intent::Trace, "Rut"),
        ("es", "¿Dónde está Moab?", Intent::Locate, "Moab"),
        ("es", "¿Qué depende de Booz?", Intent::Depend, "Booz"),
        ("es", "Compara Rut y Orfa", Intent::Compare, "Rut y Orfa"),
    ];
    for (lang, question, intent, topic) in cases {
        // Once with the language named, once with every language tried: a question in one
        // tier-1 language must not be claimed by another's rule.
        for narrowed in [Some(*lang), None] {
            let c = classify(question, narrowed)
                .unwrap_or_else(|d| panic!("{question}: {}", d.message));
            assert_eq!(c.intent, *intent, "{question} ({narrowed:?}): {}", c.rule);
            assert_eq!(c.lang, *lang, "{question}: {}", c.rule);
            assert_eq!(c.topic, *topic, "{question}");
        }
    }
}

/// §5.5: negative examples, and `SMY-E468`.
#[test]
fn a_question_no_rule_matches_is_e468() {
    for question in [
        "Naomi?",
        "Tell a story about Moab",
        "whatever Ruth did",
        "Ruth and Naomi",
        "Расскажи сказку",
        "Erzähl mir etwas",
    ] {
        let d = classify(question, None).expect_err(question);
        assert_eq!(d.code, Code::E468, "{question}");
        assert!(d.message.contains("--intent"), "{}", d.message);
    }
    // A language that does not hold the rule refuses what another language would take.
    assert_eq!(
        classify("Что такое завет?", Some("en")).unwrap_err().code,
        Code::E468
    );

    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    match ask("Naomi, please", &env, &opts()) {
        Err(AskError::Refused(d)) => assert_eq!(d.code, Code::E468),
        other => panic!("{other:?}"),
    }
    // `--intent` is the way past it.
    let d = ask("Naomi, please", &env, &opts().with_intent(Intent::Define)).expect("a dossier");
    assert_eq!(d.subject, "Naomi");
}

/// §5.5: `SMY-W417` on "Mary", and `--subject` picking one of them.
#[test]
fn an_ambiguous_subject_lists_its_candidates_and_runs_nothing() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);

    let (magdala, nazareth) = (f.uid("m/magdala"), f.uid("m/nazareth"));
    match ask("Who is Mary?", &env, &opts()) {
        Err(AskError::Ambiguous {
            name,
            candidates,
            diagnostic,
        }) => {
            assert_eq!(name, "Mary");
            let mut want = vec![vec![magdala], vec![nazareth]];
            want.sort();
            assert_eq!(candidates, want);
            assert_eq!(diagnostic.code, Code::W417);
            assert!(diagnostic.message.contains(&magdala.to_string()));
            assert!(diagnostic.message.contains("--subject"));
        }
        other => panic!("{other:?}"),
    }

    let d = ask("Who is Mary?", &env, &opts().with_subject(magdala)).expect("a dossier");
    assert_eq!(d.entities, vec![magdala]);
    assert_eq!(slot(&d, "characteristic").units, vec![f.uid("c/magdala")]);

    // A uid that is no candidate of any name in the question.
    match ask("Who is Mary?", &env, &opts().with_subject(f.uid("n/naomi"))) {
        Err(AskError::NotACandidate { .. }) => {}
        other => panic!("{other:?}"),
    }
}

/// The slots of `define` find what the fixture says they find, across `same-as`.
#[test]
fn each_slot_of_a_definition_finds_what_the_fixture_marks() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("What is Naomi?", &env, &opts()).expect("a dossier");

    assert_eq!(d.classified.intent, Intent::Define);
    assert_eq!(d.subject, "Naomi");
    assert_eq!(d.entities.len(), 2, "Naomi and Ноеминь, one thing");

    let set = |name: &str| -> BTreeSet<Uid> { slot(&d, name).units.iter().copied().collect() };
    let of = |labels: &[&str]| -> BTreeSet<Uid> { labels.iter().map(|l| f.uid(l)).collect() };
    assert_eq!(set("definition"), of(&["d/naomi"]));
    assert_eq!(
        set("characteristic"),
        of(&["c/bitter", "c/mara", "c/empty", "c/left"])
    );
    assert_eq!(set("counter"), of(&["c/not-empty"]));
    assert_eq!(set("open-question"), of(&["q/kinsman"]));
    assert!(set("exemplifies").is_empty());

    // Ranked: cited before inferred when nothing else separates them.
    let characteristic = &slot(&d, "characteristic").units;
    assert_eq!(characteristic.last(), Some(&f.uid("c/empty")));

    // Every slot is a saved query carrying the set in its recipe.
    let store = {
        let mut s = f.store.clone();
        s.append(&d.records).expect("the dossier appends");
        s
    };
    for s in &d.slots {
        let saved = smysl_query::load(&store, &s.saved).expect("a saved query");
        assert_eq!(saved.text, s.text);
        assert!(
            saved.text.contains("about entity(\"Naomi\")"),
            "{}",
            saved.text
        );
        let mut sorted = s.units.clone();
        sorted.sort();
        assert_eq!(saved.results, sorted);
    }

    // `explain` and `trace` reach a cause that does not name her.
    let d = ask("Why did Naomi leave?", &env, &opts()).expect("a dossier");
    assert_eq!(slot(&d, "cause").units, vec![f.uid("c/famine")]);
    assert_eq!(slot(&d, "evidence").units, vec![f.uid("o/return")]);
}

/// §5.5: `gap` steps for empty slots, whose unit is the slot's saved query.
#[test]
fn an_empty_slot_is_a_gap_step_on_its_saved_query() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("What is Naomi?", &env, &opts()).expect("a dossier");

    assert_eq!(d.thread.schema, smysl_core::ThreadSchema::Dossier);
    assert_eq!(d.thread.steps[0].role, Role::Question);
    assert_eq!(d.thread.steps[0].unit, d.question);

    let exemplifies = slot(&d, "exemplifies");
    let gap = d
        .thread
        .steps
        .iter()
        .find(|s| s.role == Role::Gap)
        .expect("a gap");
    assert_eq!(gap.unit, exemplifies.saved);
    assert!(
        gap.note
            .as_deref()
            .is_some_and(|n| n.starts_with("exemplifies:")),
        "{:?}",
        gap.note
    );

    // Slot order, then rank order within a slot.
    let roles: Vec<Role> = d.thread.steps.iter().map(|s| s.role).collect();
    let first = |r: Role| roles.iter().position(|x| *x == r).expect("present");
    assert!(first(Role::Definition) < first(Role::Characteristic));
    assert!(first(Role::Characteristic) < first(Role::Gap));
    assert!(first(Role::Gap) < first(Role::Counter));
    assert!(first(Role::Counter) < first(Role::OpenQuestion));
}

/// §5.5: the view's membership reaches every slot result.
#[test]
fn the_view_reaches_every_slot_result_through_the_result_edges() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("What is Naomi?", &env, &opts()).expect("a dossier");

    let mut store = f.store.clone();
    store.append(&d.records).expect("the dossier appends");
    let members: BTreeSet<Uid> = membership(&store, &d.view.roots).into_iter().collect();
    for s in &d.slots {
        assert!(members.contains(&s.saved));
        for uid in &s.units {
            assert!(members.contains(uid), "{} of {}", uid, s.name);
        }
    }
    assert!(members.contains(&d.question));
    assert_eq!(d.view.intent, "dossier");
    assert!(d.view.threads.contains(&d.thread.id));
    let requires: Vec<String> = d.view.requires.iter().map(|s| s.to_string()).collect();
    assert_eq!(requires, vec!["x.query/v1", "x.text/v1"]);

    // Every uid the thread names resolves once the records are in.
    for step in &d.thread.steps {
        assert!(store.contains_uid(&step.unit), "{}", step.unit);
    }
    // The question cites the recipe it was asked under.
    let q = store.get(&d.question).expect("the question");
    let short: String = d.recipe[..6].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        q.core.source.as_ref().map(|s| s.reference.clone()),
        Some(format!("ask:{short}"))
    );
}

/// §5.5: rule R in the pack — a rebuttal of a packed unit is packed, though no slot asked for it.
#[test]
fn a_packed_dossier_pins_the_rebuttal_of_what_it_packs() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);

    // `locate` has no counter slot, so `c/not-empty` is in the dossier only if rule R put it
    // there.
    let unpacked = ask("Where is Naomi mentioned?", &env, &opts()).expect("a dossier");
    assert!(unpacked
        .slots
        .iter()
        .all(|s| !s.units.contains(&f.uid("c/not-empty"))));
    assert!(!unpacked.thread.units().any(|u| *u == f.uid("c/not-empty")));

    let d = ask("Where is Naomi mentioned?", &env, &opts().with_budget(4000)).expect("a dossier");
    let packed: BTreeSet<Uid> = d.packed.clone().expect("packed").into_iter().collect();
    assert!(packed.contains(&f.uid("c/empty")));
    assert!(packed.contains(&f.uid("c/not-empty")), "rule R");
    let pinned = d
        .thread
        .steps
        .iter()
        .find(|s| s.unit == f.uid("c/not-empty"))
        .expect("the rebuttal is a step");
    assert_eq!(pinned.role, Role::Counter);
    assert!(pinned.note.as_deref().is_some_and(|n| n.contains("rule R")));
}

/// Rule D: the same question over the same store is the same dossier, byte for byte.
#[test]
fn a_dossier_is_a_function_of_its_question_and_its_store() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let a = ask("What is Naomi?", &env, &opts()).expect("a dossier");
    let b = ask("What is Naomi?", &env, &opts()).expect("a dossier");
    assert_eq!(a.records, b.records);
    assert_eq!(a.recipe, b.recipe);

    // A different scope is a different recipe and a different question unit.
    let c = ask("What is Naomi?", &env, &opts().with_scope("store:subjects")).expect("a dossier");
    assert_ne!(a.recipe, c.recipe);
    assert_ne!(a.question, c.question);
    assert_eq!(
        a.slots.iter().map(|s| s.units.clone()).collect::<Vec<_>>(),
        c.slots.iter().map(|s| s.units.clone()).collect::<Vec<_>>(),
        "the whole store is the store"
    );
    let _ = canonical_uid;
}

/// `SMY-E477`, and an environment with no entity port.
#[test]
fn an_unknown_template_set_and_a_missing_port_are_refused_by_name() {
    let f = fixture();
    let scope = StoreScope::new("subjects", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    match ask(
        "What is Naomi?",
        &env,
        &opts().with_templates("smysl/dossier/9"),
    ) {
        Err(AskError::Refused(d)) => assert_eq!(d.code, Code::E477),
        other => panic!("{other:?}"),
    }
    let bare = Env::new(&f.store, &scope);
    assert_eq!(
        ask("What is Naomi?", &bare, &opts()).unwrap_err(),
        AskError::NoEntities
    );
    match ask("What is Elimelech?", &env, &opts()) {
        Err(AskError::NoSubject { topic }) => assert_eq!(topic, "Elimelech"),
        other => panic!("{other:?}"),
    }
    assert_eq!(templates::set(templates::SET), Ok(templates::SET));
}

/// A store for set 2's two changes (OQ-87, OQ-88): an account of tea that a later one replaced,
/// a rebuttal of each, and a third claim with its own rebuttal.
const TEA: &str = r#"@schema x.text/v1 { version: 1, types: ["x.text/entity"], relations: [x.text/mentions, x.text/same-as] }

@x.text/entity e/tea { status: speculative, source: { kind: doc, ref: "alias:en" } }
~ tea

@claim t/old { status: cited, source: { kind: doc, ref: "tea:1" } }
~ Tea cures the ague.

@claim t/new { status: cited, source: { kind: doc, ref: "tea:2" } }
~ Tea eases the ague.

@claim t/awake { status: cited, source: { kind: doc, ref: "tea:3" } }
~ Tea keeps a drinker awake.

@claim r/old { status: cited, source: { kind: doc, ref: "tea:4" } }
~ The fevers came back whatever was drunk.

@claim r/awake { status: cited, source: { kind: doc, ref: "tea:5" } }
~ The monks drank it at night and slept well.

@rel t/new --supersedes--> t/old
@rel r/old --rebuts--> t/old
@rel r/awake --rebuts--> t/awake
@rel t/old --x.text/mentions--> e/tea
@rel t/new --x.text/mentions--> e/tea
@rel t/awake --x.text/mentions--> e/tea
"#;

fn tea() -> Fixture {
    let parsed = parse_surface(TEA).expect("the tea store parses");
    Fixture {
        labels: parsed
            .labels
            .iter()
            .map(|(l, u)| (l.to_string(), *u))
            .collect(),
        store: Store::from_records(parsed.records),
    }
}

/// OQ-87: an edge slot walks from the current account, so the rebuttal of the account a later
/// one replaced is not counter-evidence about the subject now.
#[test]
fn counter_walks_from_the_current_account() {
    let f = tea();
    let scope = StoreScope::new("tea", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("What is tea?", &env, &opts()).expect("a dossier");
    let counter: BTreeSet<Uid> = slot(&d, "counter").units.iter().copied().collect();
    assert_eq!(counter, BTreeSet::from([f.uid("r/awake")]));
    assert!(!slot(&d, "counter").narrowed, "define does not narrow");
}

/// OQ-88: a verify question's counter slot holds the rebuttals of the claim it asks about, and
/// says it was narrowed.
#[test]
fn a_verify_counter_is_narrowed_to_what_was_asked() {
    let f = tea();
    let scope = StoreScope::new("tea", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("Is it true that tea keeps you awake?", &env, &opts()).expect("a dossier");
    assert!(d.topic.contains(&"awake".to_string()), "{:?}", d.topic);
    assert!(d.topic_found);
    let counter = slot(&d, "counter");
    assert!(counter.narrowed);
    assert!(
        counter.text.contains("matches \"awake\""),
        "{}",
        counter.text
    );
    assert_eq!(counter.units, vec![f.uid("r/awake")]);
    assert!(
        !slot(&d, "support").narrowed,
        "support is not a narrowed slot"
    );
}

/// OQ-88: when nothing about the subject says any of the topic's words, no slot narrows, so the
/// dossier does not report an absence a filter made.
#[test]
fn a_topic_the_store_does_not_use_narrows_nothing() {
    let f = tea();
    let scope = StoreScope::new("tea", &f.store);
    let env = Env::new(&f.store, &scope).with_entities(&scope);
    let d = ask("Is it true that tea is green?", &env, &opts()).expect("a dossier");
    assert_eq!(d.topic, vec!["green".to_string()]);
    assert!(!d.topic_found);
    let counter = slot(&d, "counter");
    assert!(!counter.narrowed);
    assert!(!counter.text.contains("matches"), "{}", counter.text);
    assert_eq!(counter.units, vec![f.uid("r/awake")]);
}
