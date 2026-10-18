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
//! orientation `smysl-graph`'s adjacency gives every kernel kind — for `causes` and `rebuts`
//! the effect or the rebutted unit rests on the source, so `expand ->causes` from a unit about
//! the subject reaches its causes and `expand ->rebuts` its rebuttals; for `exemplifies` and
//! `elaborates` the source rests on the target, so an example of a subject's unit is behind
//! `<-exemplifies`. §3.13's sketch wrote `expand <-rebuts`, the arrow a reader of the relation
//! `A --rebuts--> B` expects; under the orientation every traversal in this build shares, that
//! walks to what the rebutted unit itself rebuts. The slots are written for the orientation
//! that runs, and the sketch is corrected in §3.13.
//!
//! **What sq/1 cannot say, and what that costs.** A slot defined by an edge — `counter` is
//! what rebuts the units about the subject — is written `expand ->rebuts depth 1 except
//! about entity($subject)`, because `expand` *adds* to the set it was given and the language
//! has no way to ask for only what the walk reached. The subtraction also removes a rebuttal
//! that itself names the subject, which then answers `characteristic` instead. Recorded as
//! OQ-86; GE-T10 (step 4) is what measures whether it matters.

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{hash_bytes, Role};

use super::intent::Intent;

/// The set this build carries.
pub const SET: &str = "smysl/dossier/1";

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
}
