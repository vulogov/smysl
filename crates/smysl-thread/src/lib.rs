//! `smysl-thread` - the five thread schemas and deterministic derivation (§19).
//!
//! The schema set is closed in 0.1 (D-4): the role-assignment rule language is not stable
//! enough to freeze as a user-facing surface. HJSON-defined schemas are deferred to 0.2.
//!
//! [`ThreadSchema`] and [`Role`] are wire format and therefore live in `smysl-core`; what
//! belongs here is the *rule table* that turns a graph into a thread, and the four stages
//! that apply it.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod derive;
pub mod schema;

pub use derive::{derive_thread, satisfies_rule_l, DeriveOptions, DeriveReport, GIST_TOKENS};
pub use schema::{all, definition, Matcher, Position, SchemaDef};
pub use smysl_core::{Role, Step, Thread, ThreadSchema};

/// What a role contributes to the `w_t` term of salience (§1.5).
///
/// A schema that weighted every role equally would make the term inert, so the tables are
/// non-uniform by construction - and a role outside its schema weighs nothing at all.
pub fn role_weight(schema: ThreadSchema, role: Role) -> f32 {
    if !schema.allows(role) {
        return 0.0;
    }
    smysl_core::quantise(definition(schema).weight_of(role))
}

/// The per-unit role weights a thread implies, for the `w_t` term of salience (§1.5).
///
/// Salience lives below threads in the crate graph, so it takes this map rather than
/// deriving it. A unit appearing in several steps takes its highest weight: being both the
/// bottom line and a supporting point should not average out to less than either.
pub fn role_weights(thread: &Thread) -> std::collections::BTreeMap<smysl_core::Uid, f32> {
    let mut out: std::collections::BTreeMap<smysl_core::Uid, f32> =
        std::collections::BTreeMap::new();
    for step in &thread.steps {
        let w = role_weight(thread.schema, step.role);
        let slot = out.entry(step.unit).or_insert(0.0);
        if w > *slot {
            *slot = w;
        }
    }
    out
}

/// The personalisation vector a thread implies (§16.4).
///
/// The `question` and `finding` units of the thread: what the reader is here to resolve.
/// A thread with neither falls back to every unit it names, because personalising against
/// nothing is the same as not personalising at all.
pub fn salience_seed(thread: &Thread) -> std::collections::BTreeSet<smysl_core::Uid> {
    let focal: std::collections::BTreeSet<smysl_core::Uid> = thread
        .steps
        .iter()
        .filter(|s| matches!(s.role, Role::Question | Role::Finding | Role::BottomLine))
        .map(|s| s.unit)
        .collect();
    if focal.is_empty() {
        thread.units().copied().collect()
    } else {
        focal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_set_is_closed_at_five() {
        assert_eq!(ThreadSchema::ALL.len(), 5);
    }

    #[test]
    fn schema_names_round_trip() {
        for &s in ThreadSchema::ALL {
            assert_eq!(ThreadSchema::parse(s.as_str()), Some(s));
        }
        assert_eq!(ThreadSchema::parse("essay"), None);
    }

    #[test]
    fn a_role_outside_its_schema_carries_no_weight() {
        assert_eq!(role_weight(ThreadSchema::Brief, Role::Coda), 0.0);
        assert!(role_weight(ThreadSchema::Brief, Role::BottomLine) > 0.0);
    }

    #[test]
    fn every_role_of_every_schema_is_weighted_and_quantised() {
        for &s in ThreadSchema::ALL {
            for &r in s.roles() {
                let w = role_weight(s, r);
                assert!(w > 0.0, "{s}/{r} weighs nothing");
                assert!(w <= 1.0, "{s}/{r} weighs more than the maximum");
                assert!(smysl_core::quantise(w) == w, "{s}/{r} is not quantised");
            }
        }
    }

    /// Weight follows what a role *does*, not where it sits: a rebuttal late in an
    /// analysis outweighs the context that opens it. A monotone table would make the
    /// weight a restatement of the order and tell salience nothing new.
    #[test]
    fn weight_is_not_merely_the_narrative_order() {
        let d = definition(ThreadSchema::Analysis);
        assert!(
            role_weight(ThreadSchema::Analysis, Role::Rebuttal)
                > role_weight(ThreadSchema::Analysis, Role::Context)
        );
        assert!(d.position_of(Role::Rebuttal) > d.position_of(Role::Context));
    }
}

#[cfg(test)]
mod salience_tests {
    use super::*;
    use smysl_core::{AgentId, Hlc, Step, ThreadId, Uid};

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn thread(schema: ThreadSchema, steps: Vec<Step>) -> Thread {
        let a = AgentId::new("human:v").unwrap();
        Thread::new(
            ThreadId::new("t/x").unwrap(),
            schema,
            a.clone(),
            "g",
            Hlc::zero(a),
        )
        .with_steps(steps)
    }

    #[test]
    fn role_weights_follow_the_schema_table() {
        let t = thread(
            ThreadSchema::Brief,
            vec![
                Step::new(Role::BottomLine, uid(1)),
                Step::new(Role::Ask, uid(2)),
            ],
        );
        let w = role_weights(&t);
        assert!(w[&uid(1)] > w[&uid(2)], "the bottom line outweighs the ask");
    }

    /// A unit that is both the bottom line and a supporting point should not average out
    /// to less than either.
    #[test]
    fn a_unit_in_two_roles_takes_the_higher_weight() {
        let t = thread(
            ThreadSchema::Brief,
            vec![
                Step::new(Role::Risk, uid(1)),
                Step::new(Role::BottomLine, uid(1)),
            ],
        );
        let w = role_weights(&t);
        let bottom = role_weight(ThreadSchema::Brief, Role::BottomLine);
        assert_eq!(w[&uid(1)], bottom);
    }

    #[test]
    fn a_unit_outside_the_thread_has_no_role_weight() {
        let t = thread(ThreadSchema::Brief, vec![Step::new(Role::Ask, uid(1))]);
        assert!(!role_weights(&t).contains_key(&uid(9)));
    }

    #[test]
    fn the_seed_is_what_the_reader_came_to_resolve() {
        let t = thread(
            ThreadSchema::Qa,
            vec![
                Step::new(Role::Question, uid(1)),
                Step::new(Role::Evidence, uid(2)),
                Step::new(Role::Answer, uid(3)),
            ],
        );
        assert_eq!(salience_seed(&t), [uid(1)].into_iter().collect());
    }

    #[test]
    fn a_brief_seeds_on_its_bottom_line() {
        let t = thread(
            ThreadSchema::Brief,
            vec![
                Step::new(Role::BottomLine, uid(1)),
                Step::new(Role::Support, uid(2)),
            ],
        );
        assert_eq!(salience_seed(&t), [uid(1)].into_iter().collect());
    }

    /// Personalising against nothing is the same as not personalising, so a thread with
    /// no focal role seeds on everything it names rather than on the empty set.
    #[test]
    fn a_thread_with_no_focal_role_seeds_on_everything_it_names() {
        let t = thread(
            ThreadSchema::Narrative,
            vec![
                Step::new(Role::Setup, uid(1)),
                Step::new(Role::Coda, uid(2)),
            ],
        );
        assert_eq!(salience_seed(&t), [uid(1), uid(2)].into_iter().collect());
    }

    #[test]
    fn an_empty_thread_seeds_on_nothing() {
        let t = thread(ThreadSchema::Brief, vec![]);
        assert!(salience_seed(&t).is_empty());
        assert!(role_weights(&t).is_empty());
    }
}
