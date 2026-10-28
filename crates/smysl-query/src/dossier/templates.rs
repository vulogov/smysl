//! The template set `smysl/dossier/1` (SMYSL-2.5 §3.13 step 3).
//!
//! An intent is a list of slots and a slot is a role and an `sq/1` file, compiled into the
//! binary so a dossier is a function of its question and of this build. The set's id and its
//! hash go into every dossier's recipe, so a template edited in place cannot pass for the one
//! a saved dossier was built with; a set id this build does not carry is `SMY-E477`.
//!
//! **A slot file is shared by every intent that has the slot.** §3.13 sketched one file per
//! intent and slot (`define/counter.sq`); a counter-argument is the same question whether the
//! dossier defines its subject or verifies a claim about it, and two copies of one query are
//! two places for them to drift. The hash covers each slot once.
//!
//! **Three parameters.** `$subject` is bound to the subject's name and resolved by
//! `about entity(…)` through the picked candidate (`SMY-W417`). `$scope` is expanded *before*
//! parsing, because sq/1 has no parameter in scope position (SMYSL-2.9 §4.3): every pipeline
//! in a slot file opens with `about entity($subject)`, and a scope given to `ask` is written as
//! a `from` line in front of each. `$lang` is bound to the question's language and no slot in
//! this set reads it — a dossier's subject resolves across `same-as`, which is what makes it
//! answer in more than one language, and a slot that filtered on the question's language would
//! undo that.
//!
//! **Which way an arrow points.** `->k` is `trace_via`: it walks what a unit *rests on*, in the
//! orientation `smysl-graph`'s adjacency gives every kernel kind, and for an edge
//! `A --k--> B` that is from `B` to `A` — the rebutted unit, the effect, the exemplified claim
//! and the elaborated unit all rest on the edge's source. So `expand ->rebuts` from a unit
//! about the subject reaches its rebuttals, `->causes` its causes and `->exemplifies` its
//! examples, and `<-k` walks to what the unit itself rebuts, causes or exemplifies. §3.13's
//! sketch wrote `expand <-rebuts`, the arrow a reader of `A --rebuts--> B` expects. Step 3
//! corrected that and then wrote `<-exemplifies`, on the belief that the example kinds point
//! the other way; nothing caught it, because the step-3 fixture's `exemplifies` slot was meant
//! to be empty, and GE-T10's first run (step 4) found the slot empty for every question.
//! `grounds` is not an edge but a field, and `->grounds` walks to a unit's grounds, which is
//! the same reading: what the unit rests on.
//!
//! **What a walk reached, and only that** (set 3, OQ-86). A slot defined by an edge —
//! `counter` is what rebuts the units about the subject — is `expand ->rebuts depth 1 only`.
//! Sets 1 and 2 wrote it `expand ->rebuts depth 1 except about entity($subject)`, because
//! `expand` *adds* to the set it was given and sq/1 had no way to ask for the walk's result
//! alone; the subtraction also removed a rebuttal that itself names the subject, which then
//! answered `characteristic` instead. `only` keeps a unit reached at depth one or more even when
//! it is a seed too, which is that rebuttal exactly.
//!
//! # Sets 2 and 3: what was asked, the current account, and the walk alone (OQ-86–OQ-88)
//!
//! Set 1's GE-T10 run fired the kill criterion on counter-evidence, and both causes were in
//! this file rather than in the grammar. Set 2 changed three things and its id said so, since a
//! dossier saved under set 1 answered a different question of the same templates; set 3 takes
//! the grammar's `only` (OQ-86) and brings `revision` back with it (OQ-87).
//!
//! **A slot can be narrowed to the question's topic** (OQ-88). A verify or explain question
//! asks something *of* its subject — "is scurvy cured by citrus fruit" — and set 1 bound only
//! the subject, so `counter` held the rebuttal of every claim about scurvy. [`narrowed`] names
//! the (intent, slot) pairs that take the topic: `counter` and `reported`, in the two intents
//! whose questions carry a claim. OQ-88 named `evidence` too, and it is out on measurement: an
//! observation is told in its own words ("Anson lost most of his men"), not the question's,
//! so narrowing it took its recall to 0 of 9 over set 1's questions and halved it over the
//! fresh ones — decided after both runs, which makes the fresh set in-sample for this one
//! choice (SMYSL-2.5 OQ-88). The topic's words are an `or` of `matches` written
//! into the slot's *first* pipeline, after its `about entity($subject)` ([`narrow`]): the units
//! the slot starts from must say something the question said. For an edge slot that is the
//! seed, so `counter` walks to the rebuttals of the claim asked about and not to rebuttals that
//! share its words; for a kind slot it is the unit itself. The words are written into the text
//! for the reason `$scope` is: sq/1's `matches` asks for every term and a topic asks for any,
//! and the saved query should carry what ran. **When no unit about the subject says any of
//! them** — a question in a language the store does not hold, or in words it does not use —
//! the slot runs unnarrowed and the dossier records that it did, because a slot emptied by a
//! filter nothing could pass would report an absence that is not in the store.
//!
//! **Edge slots walk from the current account** (OQ-87, part b). The supersession pass runs
//! after traversals (§3.2), so a walk from a replaced unit reached *its* rebuttals before the
//! pass replaced it. Every slot is planned with
//! [`PlanOptions::supersede_before_walk`](crate::PlanOptions), which runs the pass on the seeds
//! as well; the recipe records it.
//!
//! **`revision` was withdrawn in set 2 and is back in set 3** (OQ-87, part a). It asks for what a
//! later account replaced, and until set 3 sq/1 could not select a superseded unit: under the
//! current lens the old unit is replaced, and under `--all-versions` it names the subject and
//! the `except` removed it, so set 1's `trace` carried a slot that was empty for every
//! question. Set 3's is `expand <-supersedes only`: the walk from a version to the versions it
//! replaced reaches history by construction, and the supersession pass leaves what it reached
//! as written (SMYSL-2.9 §4.6).

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{hash_bytes, Role};

use super::intent::Intent;

/// The set this build carries.
pub const SET: &str = "smysl/dossier/3";

/// One slot: the role its units take in the dossier thread, and its query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Slot {
    /// The slot's name, which is its file's stem and its role's name.
    pub name: &'static str,
    pub role: Role,
    /// The template, with `$subject` unbound and `$scope` unexpanded.
    pub text: &'static str,
}

macro_rules! slot {
    ($name:literal, $role:expr) => {
        Slot {
            name: $name,
            role: $role,
            text: include_str!(concat!("templates/", $name, ".sq")),
        }
    };
}

/// Every slot, once, in role-code order: the order the hash reads them in.
pub const SLOTS: &[Slot] = &[
    slot!("support", Role::Support),
    slot!("evidence", Role::Evidence),
    slot!("exemplifies", Role::Exemplifies),
    slot!("definition", Role::Definition),
    slot!("introduction", Role::Introduction),
    slot!("characteristic", Role::Characteristic),
    slot!("counter", Role::Counter),
    slot!("reported", Role::Reported),
    slot!("contrast", Role::Contrast),
    slot!("open-question", Role::OpenQuestion),
    slot!("related", Role::Related),
    slot!("cause", Role::Cause),
    slot!("hypothesis", Role::Hypothesis),
    slot!("holder", Role::Holder),
    slot!("stance", Role::Stance),
    slot!("revision", Role::Revision),
    slot!("location", Role::Location),
    slot!("dependent", Role::Dependent),
];

/// The slots of one intent, in the order the dossier presents them.
pub fn slots(intent: Intent) -> Vec<&'static Slot> {
    let names: &[&str] = match intent {
        Intent::Define => &[
            "definition",
            "characteristic",
            "exemplifies",
            "counter",
            "open-question",
        ],
        Intent::Explain => &[
            "cause",
            "evidence",
            "hypothesis",
            "counter",
            "open-question",
        ],
        Intent::Verify => &["support", "evidence", "counter", "reported"],
        Intent::Attribute => &["reported", "holder", "stance"],
        Intent::Trace => &["introduction", "revision", "cause"],
        Intent::Locate => &["location", "related"],
        Intent::Depend => &["dependent", "related"],
        Intent::Compare => &["characteristic", "contrast", "counter"],
    };
    names
        .iter()
        .map(|n| {
            SLOTS
                .iter()
                .find(|s| s.name == *n)
                .expect("every slot an intent names is in SLOTS")
        })
        .collect()
}

/// Whether a slot of this intent is narrowed to the question's topic (OQ-88).
pub fn narrowed(intent: Intent, slot: &str) -> bool {
    match intent {
        Intent::Verify => matches!(slot, "counter" | "reported"),
        Intent::Explain => slot == "counter",
        _ => false,
    }
}

/// A slot's text with its first pipeline narrowed to any of `terms`, written as an `or` of
/// `matches` after the line that opens it. `terms` empty leaves the text as it was.
pub fn narrow(text: &str, terms: &[String]) -> String {
    if terms.is_empty() {
        return text.to_string();
    }
    let filter = terms
        .iter()
        .map(|t| {
            format!(
                "matches \"{}\"",
                t.replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect::<Vec<_>>()
        .join(" or ");
    let mut out = String::with_capacity(text.len() + filter.len() + 8);
    let mut done = false;
    for line in text.lines() {
        out.push_str(line);
        out.push('\n');
        if !done && line.starts_with("about entity(") {
            out.push_str("where ");
            out.push_str(&filter);
            out.push('\n');
            done = true;
        }
    }
    out
}

/// The set's hash: its id, then each intent's slot list, then each slot's name and text, framed
/// so no two sets can hash alike by moving a byte between fields.
pub fn hash() -> [u8; 32] {
    let mut b = Vec::new();
    let mut field = |s: &str| {
        b.extend_from_slice(&(s.len() as u64).to_be_bytes());
        b.extend_from_slice(s.as_bytes());
    };
    field(SET);
    for intent in Intent::ALL {
        field(intent.as_str());
        for slot in slots(*intent) {
            field(slot.name);
            field(if narrowed(*intent, slot.name) {
                "topic"
            } else {
                ""
            });
        }
    }
    for slot in SLOTS {
        field(slot.name);
        field(slot.text);
    }
    hash_bytes(&b)
}

/// The set named `id`, or `SMY-E477`. One set exists; the check is what lets a recipe name it.
pub fn set(id: &str) -> Result<&'static str, Diagnostic> {
    if id == SET {
        Ok(SET)
    } else {
        Err(Diagnostic::new(Code::E477).with_message(format!(
            "`{id}` is not a dossier template set this build carries; it carries `{SET}`"
        )))
    }
}

/// A slot's text with `$scope` expanded: a `from` line in front of every pipeline.
pub fn expand_scope(text: &str, scope: Option<&str>) -> String {
    let Some(scope) = scope.map(str::trim).filter(|s| !s.is_empty()) else {
        return text.to_string();
    };
    let scope = scope.strip_prefix("from ").unwrap_or(scope).trim();
    let mut out = String::with_capacity(text.len() + 32);
    for line in text.lines() {
        if line.starts_with("about entity(") {
            out.push_str("from ");
            out.push_str(scope);
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slot_s_role_is_its_name() {
        for slot in SLOTS {
            assert_eq!(slot.role.as_str(), slot.name);
        }
    }

    #[test]
    fn every_slot_parses_and_every_pipeline_opens_on_the_subject() {
        for slot in SLOTS {
            crate::parse(slot.text).unwrap_or_else(|d| panic!("{}: {}", slot.name, d.message));
            let scoped = expand_scope(slot.text, Some("texts"));
            crate::parse(&scoped).unwrap_or_else(|d| panic!("{}: {}", slot.name, d.message));
            assert_eq!(
                scoped.matches("from texts").count(),
                slot.text.matches("about entity($subject)").count()
            );
        }
    }

    #[test]
    fn every_role_of_the_dossier_schema_but_two_has_a_slot() {
        // `question` is the root and `gap` marks an empty slot; every other role is a slot's.
        for role in smysl_core::ThreadSchema::Dossier.roles() {
            if matches!(role, Role::Question | Role::Gap) {
                continue;
            }
            assert!(SLOTS.iter().any(|s| s.role == *role), "{role} has no slot");
        }
    }

    #[test]
    fn an_unknown_set_is_e477() {
        assert!(set(SET).is_ok());
        assert_eq!(set("smysl/dossier/2").unwrap_err().code, Code::E477);
    }

    #[test]
    fn no_slot_subtracts_the_subject() {
        // Set 3 (OQ-86): an edge slot is the walk alone, which `except about entity(…)` could
        // only approximate by removing every unit that named the subject.
        for slot in SLOTS {
            assert!(!slot.text.contains("except"), "{}", slot.name);
        }
        assert!(slots(Intent::Trace).iter().any(|s| s.name == "revision"));
    }

    #[test]
    fn a_narrowed_slot_filters_its_first_pipeline_only() {
        let counter = SLOTS.iter().find(|s| s.name == "counter").unwrap();
        let text = narrow(counter.text, &["citrus".into(), "cured".into()]);
        assert!(text.starts_with(
            "about entity($subject)\nwhere matches \"citrus\" or matches \"cured\"\nexpand"
        ));
        assert_eq!(text.matches("where matches").count(), 1);
        crate::parse(&text).unwrap();
        crate::parse(&expand_scope(&text, Some("texts"))).unwrap();
        assert_eq!(narrow(counter.text, &[]), counter.text);
        for intent in Intent::ALL {
            for slot in slots(*intent) {
                if narrowed(*intent, slot.name) {
                    assert!(
                        slot.text.starts_with("about entity($subject)"),
                        "{}",
                        slot.name
                    );
                }
            }
        }
    }
}
