//! Dossiers: a question, answered by the units a fixed set of queries finds about its subject
//! (SMYSL-2.5 §3.13; TX-P8 step 3).
//!
//! The pipeline is §3.13's five steps, each pure:
//!
//! 1. **Subject.** The intent rule's topic is searched for a name the entity port knows —
//!    every run of its words, longest first, leftmost first — so `why did Naomi return to
//!    Bethlehem` finds `Naomi` without the rule having to know where the name ends. A name
//!    with more than one candidate ([`crate::ports::Entities::candidates`]) is `SMY-W417` and nothing
//!    runs: the candidates are listed, and [`AskOptions::subject`] picks one.
//! 2. **Intent**, by [`intent::classify`] or named by [`AskOptions::intent`] (`SMY-E468` when
//!    neither applies).
//! 3. **Slots.** Each slot of the intent's list ([`templates::slots`]) is planned and run like
//!    any query, with `$subject` bound and the template set recorded in its recipe, and saved
//!    as an `x.query/saved` unit whose `x.query/answers` edges are its result
//!    ([`crate::saved::to_records`]): a slot is a saved query, so every slot can be rerun on
//!    its own.
//! 4. **Rank within a slot** by the product of the retrieval score (normalised to the slot's
//!    best), salience renormalised over the slot, and status rank, each quantised; ties by uid.
//!    §3.13 lists a fourth term, the independent-witness count, and it is **not** here:
//!    `Witnesses` (§3.7) is not built, and counting attestations instead would be
//!    `SMY-W479`'s fallback presented as the thing itself. OQ-52 asks whether a product is the
//!    right combination at all, and GE-T10 (step 4) is what decides it. Salience is floored at
//!    one quantum, so a slot whose units nothing points at is ordered by status rather than
//!    collapsed to a tie.
//! 5. **Pack**, when a budget is given: scoped to the slots' units, with each slot's best in
//!    focus; rule R pins live rebuttals of what is packed, as it does for every pack.
//!
//! # What a dossier is made of
//!
//! A `question` unit with the question's text (`cited`, source `ask:<recipe>`); one saved query
//! per slot; a `dossier` thread whose first step is the question and whose other steps are the
//! slots' units in slot order, with the slot's role; and a `View` whose roots are the question
//! and the slot queries, so its membership reaches every result through the result edges.
//!
//! **An empty slot is a step**, with role `gap`, whose unit is the slot's saved query and whose
//! note names the slot. An absence is a finding, and one that reruns.
//!
//! A unit two slots found is placed once, in the first; a slot every one of whose units an
//! earlier slot placed is a `gap` step whose note says so, which keeps "this slot found
//! nothing" and "this slot found nothing new" apart. And the thread keeps rule L: a dep of a
//! placed unit that no slot placed is added as a `related` step with a note naming it, because
//! a dossier a renderer cannot render is not a dossier.
//!
//! Nothing here writes. [`Dossier::records`] are what a caller appends, after which every uid
//! in the thread resolves; the CLI is the caller that writes (rule A).

pub mod intent;
pub mod templates;

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::ids::{AgentId, LangTag, SchemaId, ThreadId, Uid, ViewId};
use smysl_core::types::{Record, RelKind, SourceKind, SourceRef, Step, Thread, View};
use smysl_core::{
    canonical_uid, hash_bytes, quantise, Hlc, KernelType, Role, Status, ThreadSchema,
    UnitCoreBuilder,
};
use smysl_graph::SalienceRequest;

use crate::exec::{Answer, Run};
use crate::plan::{Env, PlanOptions, Purity};
use crate::ports::Entities;

pub use intent::{classify, Classified, Intent};

/// What `ask` was told besides the question.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct AskOptions {
    /// The intent, overriding the rule table (`--intent`).
    pub intent: Option<Intent>,
    /// The question's language, narrowing the rule table to one (`--lang`).
    pub lang: Option<String>,
    /// The entity to take, when the subject names more than one (`--subject`).
    pub subject: Option<Uid>,
    /// Scope items written in front of every slot pipeline (`--scope`), e.g. `texts`.
    pub scope: Option<String>,
    /// The template set (`--templates`); `SMY-E477` when this build does not carry it.
    pub templates: String,
    /// Pack the dossier to this many tokens (`--budget`).
    pub budget: Option<u64>,
    /// Options every slot is planned under.
    pub plan: PlanOptions,
    /// Who the thread and view belong to.
    pub owner: AgentId,
    /// The thread's timestamp. Supplied, so a dossier is a function of its inputs (rule D).
    pub ts: Option<Hlc>,
}

impl AskOptions {
    pub fn new(owner: AgentId) -> AskOptions {
        AskOptions {
            intent: None,
            lang: None,
            subject: None,
            scope: None,
            templates: templates::SET.to_string(),
            budget: None,
            plan: PlanOptions::default(),
            owner,
            ts: None,
        }
    }

    pub fn with_intent(mut self, intent: Intent) -> AskOptions {
        self.intent = Some(intent);
        self
    }

    pub fn with_lang(mut self, lang: impl Into<String>) -> AskOptions {
        self.lang = Some(lang.into());
        self
    }

    pub fn with_subject(mut self, uid: Uid) -> AskOptions {
        self.subject = Some(uid);
        self
    }

    pub fn with_scope(mut self, scope: impl Into<String>) -> AskOptions {
        self.scope = Some(scope.into());
        self
    }

    pub fn with_templates(mut self, id: impl Into<String>) -> AskOptions {
        self.templates = id.into();
        self
    }

    pub fn with_budget(mut self, tokens: u64) -> AskOptions {
        self.budget = Some(tokens);
        self
    }

    pub fn with_ts(mut self, ts: Hlc) -> AskOptions {
        self.ts = Some(ts);
        self
    }
}

/// Why `ask` produced no dossier.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum AskError {
    /// `SMY-E468`, or `SMY-E477`: the question or the template set.
    Refused(Box<Diagnostic>),
    /// The environment binds no entity port, so there is no subject to resolve. A library binds
    /// one; a plain store does not.
    NoEntities,
    /// No run of the topic's words names an entity in scope.
    NoSubject { topic: String },
    /// `SMY-W417`: the subject names several things. Each candidate is one thing — a group of
    /// entities `same-as` joins — and [`AskOptions::subject`] takes any member of one.
    Ambiguous {
        name: String,
        candidates: Vec<Vec<Uid>>,
        diagnostic: Box<Diagnostic>,
    },
    /// `--subject` named a uid that is no candidate of any name in the question.
    NotACandidate { uid: Uid, topic: String },
    /// A slot did not plan or run. A template this build carries failing is a defect in the
    /// template set; the diagnostics say which stage.
    Slot {
        slot: &'static str,
        diagnostics: Vec<Diagnostic>,
    },
    /// The pack could not meet the budget.
    Pack(String),
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AskError::Refused(d) => write!(f, "{}: {}", d.code.as_str(), d.message),
            AskError::NoEntities => write!(
                f,
                "ask resolves its subject through an entity port, and this environment binds \
                 none; a library binds one (--library)"
            ),
            AskError::NoSubject { topic } => write!(
                f,
                "no part of `{topic}` names an entity in scope; ask about something the store \
                 has an x.text/entity for"
            ),
            AskError::Ambiguous { diagnostic, .. } => {
                write!(f, "{}: {}", diagnostic.code.as_str(), diagnostic.message)
            }
            AskError::NotACandidate { uid, topic } => write!(
                f,
                "--subject {uid} is not an entity any part of `{topic}` names"
            ),
            AskError::Slot { slot, diagnostics } => {
                write!(f, "slot `{slot}` did not run:")?;
                for d in diagnostics {
                    write!(f, " {}: {};", d.code.as_str(), d.message)?;
                }
                Ok(())
            }
            AskError::Pack(e) => write!(f, "the dossier cannot be packed: {e}"),
        }
    }
}

impl std::error::Error for AskError {}

/// One slot, run.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct SlotResult {
    pub name: &'static str,
    pub role: Role,
    /// The slot's text after binding: what its saved query carries.
    pub text: String,
    /// The slot's saved query.
    pub saved: Uid,
    /// What the slot found, ranked (step 4).
    pub units: Vec<Uid>,
    /// Each unit's rank score, in the same order.
    pub scores: Vec<f32>,
    /// Whether the slot was narrowed to the question's topic ([`templates::narrowed`]): false
    /// for a slot the set does not narrow, and for one it does when no unit about the subject
    /// says any of the topic's words, so the slot ran on the subject alone.
    pub narrowed: bool,
}

/// A dossier, ready to append.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Dossier {
    pub question: Uid,
    pub classified: Classified,
    /// The name the subject was found under, and the entities taken for it.
    pub subject: String,
    pub entities: Vec<Uid>,
    /// The words of the topic a narrowed slot looks for ([`topic_terms`]), empty for an intent
    /// with no narrowed slot, and whether any unit about the subject says one — which is what
    /// decides whether those slots narrow.
    pub topic: Vec<String>,
    pub topic_found: bool,
    pub slots: Vec<SlotResult>,
    pub thread: Thread,
    pub view: View,
    /// The question unit, each slot's saved query and edges, the thread and the view.
    pub records: Vec<Record>,
    /// Pure: every slot is a pure plan, and the intent came from the rule table.
    pub purity: Purity,
    pub recipe: [u8; 32],
    /// The packed selection, when a budget was given.
    pub packed: Option<Vec<Uid>>,
    /// Warnings the slots' runs raised (`W463`, `W464`, `W415`), each once.
    pub warnings: Vec<Diagnostic>,
}

/// The entity port with one name pinned to the candidate `ask` took.
struct Picked<'a> {
    name: String,
    group: Vec<Uid>,
    inner: &'a dyn Entities,
}

impl Entities for Picked<'_> {
    fn resolve(&self, name: &str) -> Vec<Uid> {
        if name == self.name {
            self.group.clone()
        } else {
            self.inner.resolve(name)
        }
    }
}

/// The words of a topic, with the punctuation a name does not carry trimmed off each.
fn words(topic: &str) -> Vec<String> {
    topic
        .split_whitespace()
        .map(|w| {
            let w = w.trim_matches(|c: char| {
                matches!(
                    c,
                    ',' | ';'
                        | ':'
                        | '"'
                        | '«'
                        | '»'
                        | '“'
                        | '”'
                        | '„'
                        | '('
                        | ')'
                        | '?'
                        | '!'
                        | '.'
                )
            });
            w.strip_suffix("'s")
                .or_else(|| w.strip_suffix("’s"))
                .unwrap_or(w)
                .to_string()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Words that carry no topic of their own, per tier-1 language: articles, pronouns, auxiliaries,
/// prepositions and conjunctions. Written from the languages, not from any question set, and
/// short on purpose — a word left out of this list costs a little precision in a narrowed slot,
/// and a content word put in it costs the slot the one word the question turned on.
const FUNCTION_WORDS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "are", "was", "were", "been", "being", "has", "have", "had", "does",
            "did", "for", "from", "with", "that", "this", "these", "those", "its", "his", "her",
            "their", "them", "they", "who", "whom", "what", "which", "when", "where", "why", "how",
            "not", "but", "into", "onto", "than", "then", "there", "very", "really", "can",
            "could", "would", "should", "will", "shall", "may", "might", "must", "any", "all",
            "some", "true",
        ],
    ),
    (
        "de",
        &[
            "der", "die", "das", "den", "dem", "des", "ein", "eine", "einen", "einem", "einer",
            "eines", "und", "oder", "aber", "ist", "sind", "war", "waren", "wird", "werden",
            "wurde", "wurden", "hat", "haben", "hatte", "durch", "mit", "von", "vom", "zum", "zur",
            "für", "auf", "aus", "bei", "nach", "über", "unter", "dass", "wie", "was", "wer",
            "warum", "wieso", "weshalb", "nicht", "sich", "sein", "seine", "ihre", "ihr", "stimmt",
            "wahr", "wirklich", "denn", "doch", "auch", "noch", "schon", "kann", "konnte",
        ],
    ),
    (
        "es",
        &[
            "el",
            "la",
            "los",
            "las",
            "una",
            "uno",
            "unos",
            "unas",
            "del",
            "por",
            "para",
            "con",
            "sin",
            "que",
            "qué",
            "como",
            "cómo",
            "cuando",
            "donde",
            "dónde",
            "porque",
            "quien",
            "quién",
            "es",
            "son",
            "era",
            "eran",
            "fue",
            "fueron",
            "ser",
            "está",
            "están",
            "ha",
            "han",
            "había",
            "hizo",
            "sus",
            "su",
            "se",
            "no",
            "más",
            "muy",
            "cierto",
            "verdad",
            "realmente",
            "también",
            "puede",
            "pudo",
        ],
    ),
    (
        "fr",
        &[
            "le", "la", "les", "une", "des", "du", "de", "et", "ou", "mais", "est", "sont",
            "était", "étaient", "été", "être", "fut", "ont", "avait", "par", "pour", "avec",
            "sans", "dans", "sur", "sous", "que", "qui", "quoi", "comment", "quand", "pourquoi",
            "est-ce", "ce", "cette", "ces", "son", "sa", "ses", "leur", "leurs", "il", "elle",
            "ils", "elles", "pas", "ne", "vrai", "vraiment", "aussi", "peut", "pu", "fait", "si",
        ],
    ),
    (
        "ru",
        &[
            "это",
            "что",
            "как",
            "кто",
            "где",
            "когда",
            "почему",
            "зачем",
            "или",
            "но",
            "был",
            "была",
            "было",
            "были",
            "есть",
            "быть",
            "его",
            "её",
            "их",
            "она",
            "они",
            "оно",
            "для",
            "при",
            "через",
            "над",
            "под",
            "без",
            "так",
            "уже",
            "ещё",
            "тоже",
            "также",
            "очень",
            "правда",
            "действительно",
            "ли",
            "не",
            "ни",
            "из",
            "от",
            "до",
            "по",
            "на",
            "за",
            "мог",
            "могла",
            "могли",
        ],
    ),
];

/// The topic's words a narrowed slot looks for (OQ-88): the topic as `matches` tokenises it,
/// less the subject's own words, words under three characters, and the question language's
/// function words — every language's, when the language is not known (an `--intent` with
/// no `--lang` skips the rule table, which is what names it). Each once, in the order the
/// question has them.
pub fn topic_terms(topic: &str, subject: &str, lang: &str) -> Vec<String> {
    let tok = smysl_retrieve::Tokenizer::plain();
    let own: BTreeSet<String> = tok.terms(subject).into_iter().collect();
    let function: BTreeSet<&str> = FUNCTION_WORDS
        .iter()
        .filter(|(l, _)| lang.is_empty() || *l == lang)
        .flat_map(|(_, w)| w.iter().copied())
        .collect();
    let mut seen = BTreeSet::new();
    tok.terms(topic)
        .into_iter()
        .filter(|t| t.chars().count() >= 3)
        .filter(|t| !own.contains(t) && !function.contains(t.as_str()))
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

/// Step 1: the longest run of the topic's words the port has candidates for.
fn find_subject(
    topic: &str,
    port: &dyn Entities,
    pick: Option<Uid>,
) -> Result<(String, Vec<Uid>), AskError> {
    let ws = words(topic);
    for len in (1..=ws.len()).rev() {
        for start in 0..=ws.len() - len {
            let name = ws[start..start + len].join(" ");
            let candidates = port.candidates(&name);
            if candidates.is_empty() {
                continue;
            }
            if let Some(uid) = pick {
                if let Some(group) = candidates.iter().find(|g| g.contains(&uid)) {
                    return Ok((name, group.clone()));
                }
                // The pick may name an entity a shorter run means; keep looking.
                continue;
            }
            if candidates.len() > 1 {
                let listed = candidates
                    .iter()
                    .map(|g| {
                        g.iter()
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join(" = ")
                    })
                    .collect::<Vec<_>>()
                    .join("; ");
                let diagnostic = Diagnostic::new(Code::W417).with_message(format!(
                    "`{name}` names {} different entities: {listed}; pick one with --subject",
                    candidates.len()
                ));
                return Err(AskError::Ambiguous {
                    name,
                    candidates,
                    diagnostic: Box::new(diagnostic),
                });
            }
            let group = candidates.into_iter().next().expect("one candidate");
            return Ok((name, group));
        }
    }
    match pick {
        Some(uid) => Err(AskError::NotACandidate {
            uid,
            topic: topic.to_string(),
        }),
        None => Err(AskError::NoSubject {
            topic: topic.to_string(),
        }),
    }
}

/// Step 4's terms for one unit.
fn rank(
    env: &Env<'_>,
    slot: &[Uid],
    run: &Run,
    salience: &smysl_graph::SalienceReport,
) -> Vec<(Uid, f32)> {
    let retrieval: BTreeMap<Uid, f64> = match &run.answer {
        Answer::Ranked(fused) => {
            let best = fused.iter().map(|f| f.score).fold(0.0f64, f64::max);
            fused
                .iter()
                .map(|f| (f.uid, if best > 0.0 { f.score / best } else { 1.0 }))
                .collect()
        }
        _ => BTreeMap::new(),
    };
    let renormalised = salience.renormalise(slot);
    let floor = 1.0 / 1024.0;
    let max_status = Status::Measured as u8 as f32;
    let mut out: Vec<(Uid, f32)> = slot
        .iter()
        .map(|uid| {
            let r = quantise(retrieval.get(uid).copied().unwrap_or(1.0) as f32);
            let s = quantise(renormalised.get(uid).copied().unwrap_or(0.0).max(floor));
            let st = env
                .store
                .get(uid)
                .map(|u| u.core.status as u8 as f32 / max_status)
                .unwrap_or(0.0);
            (*uid, quantise(r * s * quantise(st)))
        })
        .collect();
    out.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Ask a question of an environment.
pub fn ask(question: &str, env: &Env<'_>, opts: &AskOptions) -> Result<Dossier, AskError> {
    let set = templates::set(&opts.templates).map_err(|d| AskError::Refused(Box::new(d)))?;

    // Step 2 before step 1: the rule's topic is where the subject is looked for.
    let classified = match opts.intent {
        Some(intent) => intent::untouched(question, intent),
        None => {
            classify(question, opts.lang.as_deref()).map_err(|d| AskError::Refused(Box::new(d)))?
        }
    };
    let Some(port) = env.entities else {
        return Err(AskError::NoEntities);
    };
    let (subject, group) = find_subject(&classified.topic, port, opts.subject)?;

    let picked = Picked {
        name: subject.clone(),
        group: group.clone(),
        inner: port,
    };
    let scoped = env.with_entities_of(&picked);
    let lang = if classified.lang.is_empty() {
        opts.lang.clone().unwrap_or_default()
    } else {
        classified.lang.to_string()
    };

    // The topic, and whether the store has anything about the subject that says it. Asked
    // once, the way a narrowed slot would ask it, so a slot narrows exactly when the probe
    // finds something to narrow to.
    let narrows = templates::slots(classified.intent)
        .iter()
        .any(|s| templates::narrowed(classified.intent, s.name));
    let topic = if narrows {
        topic_terms(&classified.topic, &subject, &lang)
    } else {
        Vec::new()
    };
    let topic_found = !topic.is_empty() && {
        let text = templates::expand_scope(
            &templates::narrow("about entity($subject)\n", &topic),
            opts.scope.as_deref(),
        );
        let fail = |diagnostics: Vec<Diagnostic>| AskError::Slot {
            slot: "topic",
            diagnostics,
        };
        let parsed = crate::parse(&text).map_err(|d| fail(vec![d]))?;
        let mut popts = opts.plan.clone();
        popts.params.insert("subject".to_string(), subject.clone());
        let bound = crate::bind(&parsed.query, &popts.params).map_err(|d| fail(vec![d]))?;
        let plan = crate::plan(&bound, &scoped, &popts).map_err(fail)?;
        let run = crate::run(&plan, &scoped, &popts).map_err(fail)?;
        !run.answer.uids().is_empty()
    };

    let salience = smysl_graph::salience(env.store, &SalienceRequest::default());
    let mut slot_results = Vec::new();
    let mut records: Vec<Record> = Vec::new();
    let mut warnings: Vec<Diagnostic> = Vec::new();
    for slot in templates::slots(classified.intent) {
        let narrowed = topic_found && templates::narrowed(classified.intent, slot.name);
        let text = if narrowed {
            templates::narrow(slot.text, &topic)
        } else {
            slot.text.to_string()
        };
        let text = templates::expand_scope(&text, opts.scope.as_deref());
        let fail = |diagnostics: Vec<Diagnostic>| AskError::Slot {
            slot: slot.name,
            diagnostics,
        };
        let parsed = crate::parse(&text).map_err(|d| fail(vec![d]))?;
        let mut popts = opts.plan.clone();
        popts.params.insert("subject".to_string(), subject.clone());
        popts.params.insert("lang".to_string(), lang.clone());
        popts.template_set = set.to_string();
        popts.supersede_before_walk = true;
        let bound = crate::bind(&parsed.query, &popts.params).map_err(|d| fail(vec![d]))?;
        let plan = crate::plan(&bound, &scoped, &popts).map_err(fail)?;
        let run = crate::run(&plan, &scoped, &popts).map_err(fail)?;
        for w in &run.warnings {
            if !warnings.contains(w) {
                warnings.push(w.clone());
            }
        }
        let (unit, edges) =
            crate::saved::to_records(&bound, &plan, &run, &popts).map_err(|d| fail(vec![d]))?;
        let saved = canonical_uid(&unit);
        let found = run.answer.uids();
        let ranked = rank(env, &found, &run, &salience);
        records.push(Record::Unit(unit));
        records.extend(edges.into_iter().map(Record::Relation));
        slot_results.push(SlotResult {
            name: slot.name,
            role: slot.role,
            text: crate::print(&bound),
            saved,
            units: ranked.iter().map(|(u, _)| *u).collect(),
            scores: ranked.iter().map(|(_, s)| *s).collect(),
            narrowed,
        });
    }

    // The recipe: the set and its hash, how the question was read, the subject taken, the
    // scope, and every slot's own recipe through its saved query's identity.
    let mut b = Vec::new();
    let mut field = |s: &str| {
        b.extend_from_slice(&(s.len() as u64).to_be_bytes());
        b.extend_from_slice(s.as_bytes());
    };
    field("smysl/ask/1");
    field(set);
    field(&hex(&templates::hash()));
    field(question.trim());
    field(classified.intent.as_str());
    field(&classified.rule);
    field(&subject);
    for uid in &group {
        field(&uid.to_string());
    }
    field(&topic.join(" "));
    field(opts.scope.as_deref().unwrap_or(""));
    field(&opts.budget.map(|n| n.to_string()).unwrap_or_default());
    for slot in &slot_results {
        field(slot.name);
        field(&slot.saved.to_string());
    }
    let recipe = hash_bytes(&b);
    let short = &hex(&recipe)[..12];

    // The question unit.
    let mut qb = UnitCoreBuilder::new(KernelType::Question, question.trim(), Status::Cited)
        .source(SourceRef::new(SourceKind::Doc, format!("ask:{short}")));
    if let Ok(tag) = LangTag::new(lang.clone()) {
        if !lang.is_empty() {
            qb = qb.lang(tag);
        }
    }
    let question_unit = qb.build().map_err(|e| {
        AskError::Refused(Box::new(
            Diagnostic::new(Code::E460).with_message(format!("the question cannot be a unit: {e}")),
        ))
    })?;
    let question_uid = canonical_uid(&question_unit);
    records.insert(0, Record::Unit(question_unit));

    // Step 5, when asked: the slots' units in focus, scoped to themselves and their closures.
    let members: Vec<Uid> = {
        let mut seen = BTreeSet::new();
        slot_results
            .iter()
            .flat_map(|s| s.units.iter().copied())
            .filter(|u| seen.insert(*u))
            .collect()
    };
    let packed: Option<BTreeSet<Uid>> = match opts.budget {
        None => None,
        Some(budget) => {
            let report = smysl_graph::salience(
                env.store,
                &SalienceRequest::default().seeded(members.iter().copied()),
            );
            // In scope: every slot's units. In focus: each slot's best, which the pack must
            // carry with its closure — rule R's rebuttals among it — and which is what makes a
            // budget a choice among the rest rather than a test the floor passes or fails.
            let best = slot_results.iter().filter_map(|s| s.units.first().copied());
            let request = smysl_pack::PackRequest::budget(budget)
                .scoped(members.iter().copied())
                .focusing(best);
            let pack = smysl_pack::pack(env.store, &report, &request)
                .map_err(|e| AskError::Pack(e.to_string()))?;
            Some(pack.selection.keys().copied().collect())
        }
    };

    // The thread.
    let owner = opts.owner.clone();
    let ts = opts
        .ts
        .clone()
        .unwrap_or_else(|| Hlc::new(0, 0, owner.clone()));
    let id = ThreadId::new(format!("dossier/r{short}")).expect("a well-formed thread id");
    let mut thread = Thread::new(
        id.clone(),
        ThreadSchema::Dossier,
        owner.clone(),
        question.trim(),
        ts,
    );
    thread.steps.push(Step::new(Role::Question, question_uid));
    let mut placed: BTreeSet<Uid> = BTreeSet::new();
    for slot in &slot_results {
        let kept: Vec<Uid> = slot
            .units
            .iter()
            .copied()
            .filter(|u| packed.as_ref().is_none_or(|p| p.contains(u)))
            .collect();
        if slot.units.is_empty() {
            thread.steps.push(
                Step::new(Role::Gap, slot.saved)
                    .with_note(format!("{}: nothing in scope answers this slot", slot.name)),
            );
            continue;
        }
        if kept.is_empty() {
            thread
                .steps
                .push(Step::new(Role::Gap, slot.saved).with_note(format!(
                    "{}: {} unit(s) found, none within the budget",
                    slot.name,
                    slot.units.len()
                )));
            continue;
        }
        let fresh: Vec<Uid> = kept.into_iter().filter(|u| !placed.contains(u)).collect();
        if fresh.is_empty() {
            thread
                .steps
                .push(Step::new(Role::Gap, slot.saved).with_note(format!(
                    "{}: every unit it found is placed in an earlier slot",
                    slot.name
                )));
            continue;
        }
        for uid in fresh {
            placed.insert(uid);
            thread.steps.push(Step::new(slot.role, uid));
        }
    }
    // What the pack added beyond the slots: rebuttals rule R pinned, and closure.
    if let Some(p) = &packed {
        for uid in p {
            if placed.contains(uid) {
                continue;
            }
            let rebuts = env
                .store
                .relations_of_kind(&RelKind::Rebuts)
                .iter()
                .any(|r| r.from == *uid && placed.contains(&r.to));
            placed.insert(*uid);
            thread.steps.push(if rebuts {
                Step::new(Role::Counter, *uid).with_note("pinned by rule R: it rebuts a unit above")
            } else {
                Step::new(Role::Related, *uid).with_note("in the pack's closure")
            });
        }
    }
    // Rule L: every dep of a placed unit is in the thread.
    loop {
        let missing: BTreeSet<(Uid, Uid)> = placed
            .iter()
            .filter_map(|u| env.store.get(u).map(|unit| (u, unit)))
            .flat_map(|(u, unit)| {
                unit.core
                    .deps
                    .iter()
                    .filter(|d| env.store.contains_uid(d) && !placed.contains(d))
                    .map(move |d| (*d, *u))
            })
            .collect();
        if missing.is_empty() {
            break;
        }
        let mut added = BTreeSet::new();
        for (dep, of) in missing {
            if added.insert(dep) {
                placed.insert(dep);
                thread.steps.push(
                    Step::new(Role::Related, dep)
                        .with_note(format!("a dep of {} (rule L)", of.short())),
                );
            }
        }
    }

    // The view.
    let mut view = View::new(
        ViewId::new(format!("dossier/r{short}")).expect("a well-formed view id"),
        "dossier",
    );
    view.roots.insert(question_uid);
    for slot in &slot_results {
        view.roots.insert(slot.saved);
    }
    view.threads.insert(id);
    for schema in [
        smysl_core::schemas::query::ID,
        smysl_core::schemas::text::ID,
    ] {
        if let Ok(s) = SchemaId::parse(schema) {
            view.requires.insert(s);
        }
    }
    if let Ok(tag) = LangTag::new(lang.clone()) {
        if !lang.is_empty() {
            view.lang = tag;
        }
    }
    records.push(Record::Thread(thread.clone()));
    records.push(Record::View(view.clone()));

    Ok(Dossier {
        question: question_uid,
        classified,
        subject,
        entities: group,
        topic,
        topic_found,
        slots: slot_results,
        thread,
        view,
        records,
        purity: Purity::Pure,
        recipe,
        packed: packed.map(|p| p.into_iter().collect()),
        warnings,
    })
}
