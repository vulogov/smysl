//! The five schema tables (§4, §19, D-4).
//!
//! The set is closed in 0.1. The role-assignment rule language below is exactly why: it is
//! not stable enough to freeze as a user-facing surface, so HJSON-defined schemas wait for
//! 0.2 rather than shipping a syntax that would then have to be supported for ever.
//!
//! A table is four things: which roles the schema uses and in what order, how many units
//! each role may hold, an ordered list of matchers that assign units to roles, and the
//! weights those roles contribute to salience.

use std::ops::RangeInclusive;

use smysl_core::{KernelType, RelKind, Role, Status, ThreadSchema};

/// Where a unit sits in the ordering chain, for schemas that are about sequence rather
/// than about kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Position {
    /// Nothing ordered before it.
    First,
    /// Nothing ordered after it.
    Last,
    /// Somewhere in between.
    Middle,
    /// In band `.0` of `.1`, counting along the ordering chain.
    ///
    /// `First` and `Last` are exact but only name two units, so a schema with five
    /// positional roles needs a way to say "about a third of the way through". Bands are
    /// computed from a unit's *rank* in the chain rather than from its index, so they do
    /// not depend on how many units the graph happens to hold outside the scope.
    Band(usize, usize),
}

/// A rule for assigning a unit to a role.
///
/// Matchers are tried in table order and the first hit wins, so a table reads top to
/// bottom as "the most specific thing this could be".
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Matcher {
    /// The unit is of this kernel type.
    Type(KernelType),
    /// The unit is the **source** of an edge of this kind - the one doing the rebutting,
    /// answering, or causing.
    SourceOf(RelKind),
    /// The unit is the **target** of an edge of this kind - the one being rebutted.
    TargetOf(RelKind),
    /// The unit's status is at least this strong.
    StatusAtLeast(Status),
    /// The unit is among the n most salient in the view.
    SalienceTop(usize),
    /// The unit sits here in the ordering chain.
    At(Position),
    /// Anything not already claimed.
    Any,
}

/// One schema's table.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SchemaDef {
    pub schema: ThreadSchema,
    /// Roles in narrative order. A thread reads in this order.
    pub roles: &'static [Role],
    /// How many units each role may hold.
    pub arity: &'static [(Role, RangeInclusive<usize>)],
    /// Ordered assignment rules; first match wins.
    pub rules: &'static [(Matcher, Role)],
    /// What each role contributes to the `w_t` term of salience (§1.5).
    pub weights: &'static [(Role, f32)],
}

impl SchemaDef {
    /// How many units a role may hold. Roles the table does not mention are optional and
    /// unbounded, which keeps a schema usable on a corpus it was not designed for.
    pub fn arity_of(&self, role: Role) -> RangeInclusive<usize> {
        self.arity
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, a)| a.clone())
            .unwrap_or(0..=usize::MAX)
    }

    pub fn weight_of(&self, role: Role) -> f32 {
        self.weights
            .iter()
            .find(|(r, _)| *r == role)
            .map(|(_, w)| *w)
            .unwrap_or(0.0)
    }

    /// Where this role sits in the narrative order.
    pub fn position_of(&self, role: Role) -> Option<usize> {
        self.roles.iter().position(|r| *r == role)
    }

    /// Roles that must hold at least one unit for a thread to be well-formed.
    pub fn required(&self) -> Vec<Role> {
        self.roles
            .iter()
            .copied()
            .filter(|r| *self.arity_of(*r).start() > 0)
            .collect()
    }
}

/// The table for a schema.
///
/// [`ThreadSchema`] is `#[non_exhaustive]` for wire evolution, so a fallback arm is
/// required. It returns the analysis table - the most general of the five - rather than
/// panicking, because a library should not abort on input the kernel considers valid.
/// `every_schema_has_a_table` fails if a variant ever reaches that arm.
pub fn definition(schema: ThreadSchema) -> &'static SchemaDef {
    match schema {
        ThreadSchema::Analysis => &ANALYSIS,
        ThreadSchema::Narrative => &NARRATIVE,
        ThreadSchema::Brief => &BRIEF,
        ThreadSchema::Qa => &QA,
        ThreadSchema::Plan => &PLAN,
        _ => &ANALYSIS,
    }
}

/// Every table, in schema order.
pub fn all() -> Vec<&'static SchemaDef> {
    ThreadSchema::ALL.iter().copied().map(definition).collect()
}

// ---------------------------------------------------------------------------
// analysis: context -> tension -> approach -> finding -> rebuttal -> implication -> next
// ---------------------------------------------------------------------------

static ANALYSIS: SchemaDef = SchemaDef {
    schema: ThreadSchema::Analysis,
    roles: &[
        Role::Context,
        Role::Tension,
        Role::Approach,
        Role::Finding,
        Role::Rebuttal,
        Role::Implication,
        Role::Next,
    ],
    arity: &[
        (Role::Context, 0..=2),
        (Role::Tension, 1..=2),
        (Role::Approach, 0..=2),
        (Role::Finding, 1..=3),
        (Role::Rebuttal, 0..=3),
        (Role::Implication, 0..=2),
        (Role::Next, 0..=1),
    ],
    rules: &[
        // A rebuttal is the *source* of the edge - the unit doing the rebutting. Matching
        // the target instead would put the rebutted claim in the rebuttal slot, which is
        // the opposite of what a reader needs.
        (Matcher::SourceOf(RelKind::Rebuts), Role::Rebuttal),
        (Matcher::Type(KernelType::Definition), Role::Context),
        (Matcher::Type(KernelType::Constraint), Role::Context),
        (Matcher::Type(KernelType::Question), Role::Tension),
        (Matcher::Type(KernelType::Observation), Role::Tension),
        (Matcher::Type(KernelType::Data), Role::Tension),
        (Matcher::Type(KernelType::Hypothesis), Role::Approach),
        (Matcher::Type(KernelType::Procedure), Role::Approach),
        (Matcher::Type(KernelType::Finding), Role::Finding),
        // A decision is what happens next, not what follows logically; an implication is
        // a claim drawn from a finding. Mapping decisions to `implication` would leave
        // `next` unreachable, which is a role the schema promises and never delivers.
        (Matcher::Type(KernelType::Decision), Role::Next),
        (Matcher::Type(KernelType::Claim), Role::Implication),
        (Matcher::Any, Role::Context),
    ],
    weights: &[
        (Role::Context, 0.4),
        (Role::Tension, 0.8),
        (Role::Approach, 0.5),
        (Role::Finding, 1.0),
        (Role::Rebuttal, 0.9),
        (Role::Implication, 0.6),
        (Role::Next, 0.3),
    ],
};

// ---------------------------------------------------------------------------
// narrative: setup -> complication -> turn -> resolution -> coda
// ---------------------------------------------------------------------------

/// The only schema that is about *sequence* rather than kind, so its table is positional.
/// This is the shape GE-2 is a question about: whether a claim graph can carry narrative
/// without damaging it.
static NARRATIVE: SchemaDef = SchemaDef {
    schema: ThreadSchema::Narrative,
    roles: &[
        Role::Setup,
        Role::Complication,
        Role::Turn,
        Role::Resolution,
        Role::Coda,
    ],
    arity: &[
        (Role::Setup, 1..=1),
        (Role::Complication, 0..=2),
        (Role::Turn, 0..=2),
        (Role::Resolution, 0..=2),
        (Role::Coda, 0..=1),
    ],
    // The ends of the chain are exact; the middle is banded into thirds, so a five-unit
    // chain lands one unit in each of the five roles instead of piling three of them into
    // `complication` and leaving `turn` and `resolution` permanently empty.
    rules: &[
        (Matcher::At(Position::First), Role::Setup),
        (Matcher::At(Position::Last), Role::Coda),
        (Matcher::At(Position::Band(1, 5)), Role::Complication),
        (Matcher::At(Position::Band(2, 5)), Role::Turn),
        (Matcher::At(Position::Band(3, 5)), Role::Resolution),
        (Matcher::Any, Role::Complication),
    ],
    weights: &[
        (Role::Setup, 0.6),
        (Role::Complication, 0.8),
        (Role::Turn, 1.0),
        (Role::Resolution, 0.9),
        (Role::Coda, 0.5),
    ],
};

// ---------------------------------------------------------------------------
// brief: bottom-line -> support -> risk -> ask
// ---------------------------------------------------------------------------

static BRIEF: SchemaDef = SchemaDef {
    schema: ThreadSchema::Brief,
    roles: &[Role::BottomLine, Role::Support, Role::Risk, Role::Ask],
    arity: &[
        (Role::BottomLine, 1..=1),
        (Role::Support, 1..=3),
        (Role::Risk, 0..=2),
        (Role::Ask, 0..=1),
    ],
    rules: &[
        (Matcher::SourceOf(RelKind::Rebuts), Role::Risk),
        (Matcher::Type(KernelType::Finding), Role::BottomLine),
        (Matcher::Type(KernelType::Decision), Role::Ask),
        (Matcher::Type(KernelType::Question), Role::Ask),
        (Matcher::Type(KernelType::Evidence), Role::Support),
        (Matcher::Type(KernelType::Observation), Role::Support),
        (Matcher::Type(KernelType::Data), Role::Support),
        (Matcher::Type(KernelType::Hypothesis), Role::Risk),
        (Matcher::Any, Role::Support),
    ],
    weights: &[
        (Role::BottomLine, 1.0),
        (Role::Support, 0.7),
        (Role::Risk, 0.8),
        (Role::Ask, 0.5),
    ],
};

// ---------------------------------------------------------------------------
// qa: question -> evidence -> answer -> caveat
// ---------------------------------------------------------------------------

static QA: SchemaDef = SchemaDef {
    schema: ThreadSchema::Qa,
    roles: &[Role::Question, Role::Evidence, Role::Answer, Role::Caveat],
    arity: &[
        (Role::Question, 1..=1),
        (Role::Evidence, 0..=3),
        (Role::Answer, 1..=2),
        (Role::Caveat, 0..=2),
    ],
    rules: &[
        (Matcher::SourceOf(RelKind::Rebuts), Role::Caveat),
        (Matcher::Type(KernelType::Question), Role::Question),
        (Matcher::SourceOf(RelKind::Answers), Role::Answer),
        (Matcher::Type(KernelType::Finding), Role::Answer),
        (Matcher::Type(KernelType::Evidence), Role::Evidence),
        (Matcher::Type(KernelType::Observation), Role::Evidence),
        (Matcher::Type(KernelType::Data), Role::Evidence),
        (Matcher::Type(KernelType::Hypothesis), Role::Caveat),
        (Matcher::Any, Role::Evidence),
    ],
    weights: &[
        (Role::Question, 0.9),
        (Role::Evidence, 0.6),
        (Role::Answer, 1.0),
        (Role::Caveat, 0.7),
    ],
};

// ---------------------------------------------------------------------------
// plan: goal -> constraint -> step -> decision -> risk
// ---------------------------------------------------------------------------

static PLAN: SchemaDef = SchemaDef {
    schema: ThreadSchema::Plan,
    roles: &[
        Role::Goal,
        Role::Constraint,
        Role::Step,
        Role::Decision,
        Role::Risk,
    ],
    arity: &[
        (Role::Goal, 1..=1),
        (Role::Constraint, 0..=3),
        (Role::Step, 1..=5),
        (Role::Decision, 0..=2),
        (Role::Risk, 0..=2),
    ],
    rules: &[
        (Matcher::SourceOf(RelKind::Rebuts), Role::Risk),
        (Matcher::Type(KernelType::Constraint), Role::Constraint),
        (Matcher::Type(KernelType::Procedure), Role::Step),
        (Matcher::Type(KernelType::Decision), Role::Decision),
        (Matcher::Type(KernelType::Finding), Role::Goal),
        (Matcher::Type(KernelType::Question), Role::Goal),
        (Matcher::Type(KernelType::Hypothesis), Role::Risk),
        (Matcher::Any, Role::Step),
    ],
    weights: &[
        (Role::Goal, 1.0),
        (Role::Constraint, 0.6),
        (Role::Step, 0.8),
        (Role::Decision, 0.7),
        (Role::Risk, 0.7),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_schema_has_a_table() {
        assert_eq!(all().len(), 5);
        for s in ThreadSchema::ALL {
            assert_eq!(definition(*s).schema, *s);
        }
    }

    /// The table's role list must agree with the kernel's, or the wire form and the
    /// derivation would disagree about what a schema is.
    #[test]
    fn table_roles_match_the_kernel_schema() {
        for def in all() {
            assert_eq!(def.roles, def.schema.roles(), "{}", def.schema);
        }
    }

    #[test]
    fn every_role_has_an_arity_and_a_weight() {
        for def in all() {
            for role in def.roles {
                assert!(
                    def.arity.iter().any(|(r, _)| r == role),
                    "{} has no arity for {role}",
                    def.schema
                );
                assert!(
                    def.weight_of(*role) > 0.0,
                    "{} gives {role} no weight",
                    def.schema
                );
            }
        }
    }

    /// A schema that weighted every role equally would make the `w_t` term inert.
    #[test]
    fn role_weights_are_not_uniform() {
        for def in all() {
            let first = def.weight_of(def.roles[0]);
            assert!(
                def.roles.iter().any(|r| def.weight_of(*r) != first),
                "{} weights every role the same",
                def.schema
            );
        }
    }

    #[test]
    fn every_table_ends_in_a_catch_all() {
        for def in all() {
            let last = def.rules.last().expect("a table has rules");
            assert_eq!(
                last.0,
                Matcher::Any,
                "{} would leave units unassigned",
                def.schema
            );
            assert!(def.roles.contains(&last.1));
        }
    }

    #[test]
    fn every_rule_targets_a_role_the_schema_declares() {
        for def in all() {
            for (m, role) in def.rules {
                assert!(
                    def.roles.contains(role),
                    "{}: rule {m:?} targets {role}, which it does not declare",
                    def.schema
                );
            }
        }
    }

    /// A rebuttal is the source of the edge, not its target. Matching the target would
    /// put the rebutted claim in the rebuttal slot - the opposite of what a reader needs.
    #[test]
    fn rebuttal_roles_match_the_source_of_the_edge() {
        for def in all() {
            for (m, role) in def.rules {
                if matches!(role, Role::Rebuttal | Role::Risk | Role::Caveat) {
                    if let Matcher::TargetOf(k) = m {
                        panic!("{}: {role} matches the target of {k}", def.schema);
                    }
                }
            }
        }
        assert!(BRIEF
            .rules
            .contains(&(Matcher::SourceOf(RelKind::Rebuts), Role::Risk)));
    }

    /// The converse of the rule above, and the one that actually bites: a role no rule can
    /// ever target is a promise the schema cannot keep. `narrative` collapsed five roles
    /// into three and `analysis` could never reach `next` until this test was written.
    #[test]
    fn every_declared_role_is_reachable_by_some_rule() {
        for def in all() {
            for role in def.roles {
                assert!(
                    def.rules.iter().any(|(_, r)| r == role),
                    "{}: no rule can ever assign {role}",
                    def.schema
                );
            }
        }
    }

    #[test]
    fn arity_ranges_are_sane() {
        for def in all() {
            for role in def.roles {
                let a = def.arity_of(*role);
                assert!(
                    a.start() <= a.end(),
                    "{}: {role} has an inverted range",
                    def.schema
                );
                assert!(
                    *a.end() > 0,
                    "{}: {role} can never hold anything",
                    def.schema
                );
            }
        }
    }

    /// Every schema names at least one role that must be filled, or a thread of it could
    /// be empty and still count as derived.
    #[test]
    fn every_schema_requires_something() {
        for def in all() {
            assert!(
                !def.required().is_empty(),
                "{} requires nothing",
                def.schema
            );
        }
    }

    #[test]
    fn brief_requires_a_bottom_line_and_support() {
        assert_eq!(BRIEF.required(), vec![Role::BottomLine, Role::Support]);
        assert_eq!(BRIEF.arity_of(Role::BottomLine), 1..=1);
    }

    #[test]
    fn qa_requires_a_question_and_an_answer() {
        assert_eq!(QA.required(), vec![Role::Question, Role::Answer]);
    }

    /// Narrative is the only positional schema - it is about sequence rather than kind.
    #[test]
    fn narrative_assigns_by_position() {
        assert!(NARRATIVE
            .rules
            .iter()
            .any(|(m, _)| matches!(m, Matcher::At(_))));
        for def in all() {
            if def.schema == ThreadSchema::Narrative {
                continue;
            }
            assert!(
                !def.rules.iter().any(|(m, _)| matches!(m, Matcher::At(_))),
                "{} should assign by kind, not position",
                def.schema
            );
        }
    }

    #[test]
    fn role_positions_follow_the_narrative_order() {
        assert_eq!(BRIEF.position_of(Role::BottomLine), Some(0));
        assert_eq!(BRIEF.position_of(Role::Ask), Some(3));
        assert_eq!(BRIEF.position_of(Role::Coda), None);
    }

    #[test]
    fn an_undeclared_role_is_optional_and_unbounded() {
        let a = BRIEF.arity_of(Role::Coda);
        assert_eq!(*a.start(), 0);
        assert_eq!(*a.end(), usize::MAX);
        assert_eq!(BRIEF.weight_of(Role::Coda), 0.0);
    }
}
