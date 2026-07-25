//! The thread plane: named, ordered, role-annotated walks (§4).
//!
//! Threads are the one piece of owned state in the format. They are keyed
//! `(thread_id, owner)` and resolved last-writer-wins *within that key*, which sidesteps
//! ordered-sequence CRDTs entirely: presentation order is an opinion, and opinions have
//! authors (§5.2).

use core::fmt;

use crate::ids::{AgentId, ThreadId, Uid};
use crate::types::provenance::Hlc;
use crate::types::unit::Extra;

/// The closed thread-schema set of 0.1 (D-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum ThreadSchema {
    Analysis = 0,
    Narrative = 1,
    Brief = 2,
    Qa = 3,
    Plan = 4,
}

impl ThreadSchema {
    pub const ALL: &'static [ThreadSchema] = &[
        ThreadSchema::Analysis,
        ThreadSchema::Narrative,
        ThreadSchema::Brief,
        ThreadSchema::Qa,
        ThreadSchema::Plan,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<ThreadSchema> {
        match v {
            0 => Some(ThreadSchema::Analysis),
            1 => Some(ThreadSchema::Narrative),
            2 => Some(ThreadSchema::Brief),
            3 => Some(ThreadSchema::Qa),
            4 => Some(ThreadSchema::Plan),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ThreadSchema::Analysis => "analysis",
            ThreadSchema::Narrative => "narrative",
            ThreadSchema::Brief => "brief",
            ThreadSchema::Qa => "qa",
            ThreadSchema::Plan => "plan",
        }
    }

    pub fn parse(s: &str) -> Option<ThreadSchema> {
        ThreadSchema::ALL.iter().copied().find(|t| t.as_str() == s)
    }

    /// The roles this schema uses, in narrative order.
    pub const fn roles(self) -> &'static [Role] {
        match self {
            ThreadSchema::Analysis => &[
                Role::Context,
                Role::Tension,
                Role::Approach,
                Role::Finding,
                Role::Rebuttal,
                Role::Implication,
                Role::Next,
            ],
            ThreadSchema::Narrative => &[
                Role::Setup,
                Role::Complication,
                Role::Turn,
                Role::Resolution,
                Role::Coda,
            ],
            ThreadSchema::Brief => &[Role::BottomLine, Role::Support, Role::Risk, Role::Ask],
            ThreadSchema::Qa => &[Role::Question, Role::Evidence, Role::Answer, Role::Caveat],
            ThreadSchema::Plan => &[
                Role::Goal,
                Role::Constraint,
                Role::Step,
                Role::Decision,
                Role::Risk,
            ],
        }
    }

    pub fn allows(self, role: Role) -> bool {
        self.roles().contains(&role)
    }
}

impl fmt::Display for ThreadSchema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The union of roles across all five schemas. Wire codes are stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Role {
    // analysis
    Context = 0,
    Tension = 1,
    Approach = 2,
    Finding = 3,
    Rebuttal = 4,
    Implication = 5,
    Next = 6,
    // narrative
    Setup = 7,
    Complication = 8,
    Turn = 9,
    Resolution = 10,
    Coda = 11,
    // brief
    BottomLine = 12,
    Support = 13,
    Risk = 14,
    Ask = 15,
    // qa
    Question = 16,
    Evidence = 17,
    Answer = 18,
    Caveat = 19,
    // plan
    Goal = 20,
    Constraint = 21,
    Step = 22,
    Decision = 23,
}

impl Role {
    pub const ALL: &'static [Role] = &[
        Role::Context,
        Role::Tension,
        Role::Approach,
        Role::Finding,
        Role::Rebuttal,
        Role::Implication,
        Role::Next,
        Role::Setup,
        Role::Complication,
        Role::Turn,
        Role::Resolution,
        Role::Coda,
        Role::BottomLine,
        Role::Support,
        Role::Risk,
        Role::Ask,
        Role::Question,
        Role::Evidence,
        Role::Answer,
        Role::Caveat,
        Role::Goal,
        Role::Constraint,
        Role::Step,
        Role::Decision,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Option<Role> {
        Role::ALL.get(v as usize).copied()
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Role::Context => "context",
            Role::Tension => "tension",
            Role::Approach => "approach",
            Role::Finding => "finding",
            Role::Rebuttal => "rebuttal",
            Role::Implication => "implication",
            Role::Next => "next",
            Role::Setup => "setup",
            Role::Complication => "complication",
            Role::Turn => "turn",
            Role::Resolution => "resolution",
            Role::Coda => "coda",
            Role::BottomLine => "bottom-line",
            Role::Support => "support",
            Role::Risk => "risk",
            Role::Ask => "ask",
            Role::Question => "question",
            Role::Evidence => "evidence",
            Role::Answer => "answer",
            Role::Caveat => "caveat",
            Role::Goal => "goal",
            Role::Constraint => "constraint",
            Role::Step => "step",
            Role::Decision => "decision",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        Role::ALL.iter().copied().find(|r| r.as_str() == s)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One position in a thread.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Step {
    pub role: Role,
    pub unit: Uid,
    pub note: Option<String>,
}

impl Step {
    pub fn new(role: Role, unit: Uid) -> Step {
        Step {
            role,
            unit,
            note: None,
        }
    }

    pub fn with_note(mut self, n: impl Into<String>) -> Step {
        self.note = Some(n.into());
        self
    }
}

/// A named, ordered, role-annotated walk over the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: ThreadId,
    pub schema: ThreadSchema,
    pub owner: AgentId,
    pub gist: String,
    pub steps: Vec<Step>,
    pub ts: Hlc,
    pub extra: Extra,
}

impl Thread {
    pub fn new(
        id: ThreadId,
        schema: ThreadSchema,
        owner: AgentId,
        gist: impl Into<String>,
        ts: Hlc,
    ) -> Thread {
        Thread {
            id,
            schema,
            owner,
            gist: gist.into(),
            steps: Vec::new(),
            ts,
            extra: Extra::new(),
        }
    }

    pub fn with_steps(mut self, steps: impl IntoIterator<Item = Step>) -> Thread {
        self.steps = steps.into_iter().collect();
        self
    }

    /// The LWW register key (§5.2). Two agents publishing the same thread id do not
    /// conflict; they publish two registers.
    pub fn register_key(&self) -> (&ThreadId, &AgentId) {
        (&self.id, &self.owner)
    }

    /// Whether `other` supersedes this register. Only comparable within the same key -
    /// an HLC never adjudicates between different owners.
    pub fn superseded_by(&self, other: &Thread) -> bool {
        self.register_key() == other.register_key() && other.ts > self.ts
    }

    pub fn units(&self) -> impl Iterator<Item = &Uid> {
        self.steps.iter().map(|s| &s.unit)
    }

    /// Steps whose role is not part of the declared schema. A derived thread never has
    /// any; an imported one might.
    pub fn foreign_roles(&self) -> Vec<Role> {
        let mut v: Vec<Role> = self
            .steps
            .iter()
            .map(|s| s.role)
            .filter(|r| !self.schema.allows(*r))
            .collect();
        v.sort();
        v.dedup();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    #[test]
    fn five_schemas_round_trip() {
        assert_eq!(ThreadSchema::ALL.len(), 5);
        for &s in ThreadSchema::ALL {
            assert_eq!(ThreadSchema::from_u8(s.as_u8()), Some(s));
            assert_eq!(ThreadSchema::parse(s.as_str()), Some(s));
        }
        assert_eq!(ThreadSchema::from_u8(5), None);
        assert_eq!(ThreadSchema::parse("essay"), None);
    }

    #[test]
    fn schema_role_sequences_match_section_4() {
        let names = |s: ThreadSchema| -> Vec<&'static str> {
            s.roles().iter().map(|r| r.as_str()).collect()
        };
        assert_eq!(
            names(ThreadSchema::Analysis),
            [
                "context",
                "tension",
                "approach",
                "finding",
                "rebuttal",
                "implication",
                "next"
            ]
        );
        assert_eq!(
            names(ThreadSchema::Narrative),
            ["setup", "complication", "turn", "resolution", "coda"]
        );
        assert_eq!(
            names(ThreadSchema::Brief),
            ["bottom-line", "support", "risk", "ask"]
        );
        assert_eq!(
            names(ThreadSchema::Qa),
            ["question", "evidence", "answer", "caveat"]
        );
        assert_eq!(
            names(ThreadSchema::Plan),
            ["goal", "constraint", "step", "decision", "risk"]
        );
    }

    #[test]
    fn twenty_four_roles_round_trip() {
        assert_eq!(Role::ALL.len(), 24);
        for &r in Role::ALL {
            assert_eq!(Role::from_u8(r.as_u8()), Some(r));
            assert_eq!(Role::parse(r.as_str()), Some(r));
        }
        assert_eq!(Role::from_u8(24), None);
    }

    #[test]
    fn role_codes_are_positional_and_stable() {
        assert_eq!(Role::Context.as_u8(), 0);
        assert_eq!(Role::Coda.as_u8(), 11);
        assert_eq!(Role::BottomLine.as_u8(), 12);
        assert_eq!(Role::Decision.as_u8(), 23);
        for (i, &r) in Role::ALL.iter().enumerate() {
            assert_eq!(r.as_u8() as usize, i);
        }
    }

    /// `risk` appears in two schemas and must be one role, not two.
    #[test]
    fn roles_shared_between_schemas_are_the_same_role() {
        assert!(ThreadSchema::Brief.allows(Role::Risk));
        assert!(ThreadSchema::Plan.allows(Role::Risk));
        let all: std::collections::BTreeSet<Role> = ThreadSchema::ALL
            .iter()
            .flat_map(|s| s.roles().iter().copied())
            .collect();
        assert_eq!(all.len(), Role::ALL.len(), "every role belongs to a schema");
    }

    #[test]
    fn schemas_reject_roles_they_do_not_declare() {
        assert!(!ThreadSchema::Brief.allows(Role::Coda));
        assert!(!ThreadSchema::Qa.allows(Role::Goal));
    }

    #[test]
    fn threads_key_on_id_and_owner() {
        let a = agent("model:anthropic/claude-opus-5");
        let b = agent("model:openai/gpt");
        let id = ThreadId::new("t/brief").unwrap();
        let t1 = Thread::new(
            id.clone(),
            ThreadSchema::Brief,
            a.clone(),
            "g",
            Hlc::new(1, 0, a.clone()),
        );
        let t2 = Thread::new(
            id.clone(),
            ThreadSchema::Brief,
            b.clone(),
            "g",
            Hlc::new(9, 0, b),
        );
        assert_ne!(
            t1.register_key(),
            t2.register_key(),
            "two owners publish two registers, they do not conflict"
        );
        assert!(
            !t1.superseded_by(&t2),
            "an HLC never adjudicates across owners"
        );
    }

    #[test]
    fn a_later_write_by_the_same_owner_wins() {
        let a = agent("human:vladimir");
        let id = ThreadId::new("t/brief").unwrap();
        let old = Thread::new(
            id.clone(),
            ThreadSchema::Brief,
            a.clone(),
            "old",
            Hlc::new(1, 0, a.clone()),
        );
        let new = Thread::new(id, ThreadSchema::Brief, a.clone(), "new", Hlc::new(2, 0, a));
        assert!(old.superseded_by(&new));
        assert!(!new.superseded_by(&old));
    }

    #[test]
    fn threads_enumerate_their_units_in_step_order() {
        let a = agent("human:v");
        let t = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            a.clone(),
            "g",
            Hlc::zero(a),
        )
        .with_steps([
            Step::new(Role::BottomLine, uid(5)),
            Step::new(Role::Support, uid(2)).with_note("the pool metrics"),
        ]);
        let order: Vec<u8> = t.units().map(|u| u.as_bytes()[0]).collect();
        assert_eq!(order, [5, 2], "step order is authored, not sorted");
        assert_eq!(t.steps[1].note.as_deref(), Some("the pool metrics"));
    }

    #[test]
    fn foreign_roles_are_reported_not_rejected() {
        let a = agent("human:v");
        let t = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            a.clone(),
            "g",
            Hlc::zero(a),
        )
        .with_steps([
            Step::new(Role::BottomLine, uid(1)),
            Step::new(Role::Coda, uid(2)),
            Step::new(Role::Coda, uid(3)),
        ]);
        assert_eq!(t.foreign_roles(), [Role::Coda]);
    }
}
