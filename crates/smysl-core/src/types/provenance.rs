//! Provenance: hybrid logical clocks and attestations (§1.1, §9.3).
//!
//! Attestations are never hashed. That is the whole point of the split: identity is
//! content (P1), so the same claim asserted by two agents is one unit with two
//! attestations, and corroboration becomes countable rather than duplicative.

use core::fmt;
use std::collections::BTreeSet;

use crate::ids::{AgentId, Uid};
use crate::types::epistemics::Status;

// ---------------------------------------------------------------------------
// Hybrid logical clock
// ---------------------------------------------------------------------------

/// A hybrid logical clock.
///
/// HLCs order supersession chains and thread registers, and nothing else. They MUST NOT
/// resolve conflicts over content (§5.1) - a later timestamp is not a better claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    pub wall_ms: u64,
    pub counter: u32,
    pub agent: AgentId,
}

impl Hlc {
    pub const fn new(wall_ms: u64, counter: u32, agent: AgentId) -> Hlc {
        Hlc {
            wall_ms,
            counter,
            agent,
        }
    }

    /// The clock at the very start of time for `agent`, for seeding a fresh chain.
    pub fn zero(agent: AgentId) -> Hlc {
        Hlc {
            wall_ms: 0,
            counter: 0,
            agent,
        }
    }

    /// Advance from `prev`.
    ///
    /// This is the **only** wall-clock read in the pure crates, and it happens only at
    /// record-creation time (rule D). Everything downstream is a function of the recorded
    /// value, never of the current time.
    pub fn now(prev: &Hlc, agent: &AgentId) -> Hlc {
        Hlc::advance(prev, agent, system_time_ms())
    }

    /// The pure core of [`Hlc::now`], with the clock read passed in. Tests and replays
    /// use this; nothing else should.
    pub fn advance(prev: &Hlc, agent: &AgentId, wall: u64) -> Hlc {
        if wall > prev.wall_ms {
            Hlc {
                wall_ms: wall,
                counter: 0,
                agent: agent.clone(),
            }
        } else {
            Hlc {
                wall_ms: prev.wall_ms,
                counter: prev.counter + 1,
                agent: agent.clone(),
            }
        }
    }

    /// Merge two observed clocks, as a receiver does on delivery.
    pub fn observe(local: &Hlc, remote: &Hlc, agent: &AgentId, wall: u64) -> Hlc {
        let hi = local.wall_ms.max(remote.wall_ms).max(wall);
        let counter = if hi == local.wall_ms && hi == remote.wall_ms {
            local.counter.max(remote.counter) + 1
        } else if hi == local.wall_ms {
            local.counter + 1
        } else if hi == remote.wall_ms {
            remote.counter + 1
        } else {
            0
        };
        Hlc {
            wall_ms: hi,
            counter,
            agent: agent.clone(),
        }
    }
}

fn system_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl fmt::Display for Hlc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}@{}", self.wall_ms, self.counter, self.agent)
    }
}

// ---------------------------------------------------------------------------
// Op and Rung
// ---------------------------------------------------------------------------

/// What an agent did to produce a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Op {
    Authored = 0,
    Transformed = 1,
    Imported = 2,
    Attested = 3,
}

impl Op {
    pub const ALL: &'static [Op] = &[Op::Authored, Op::Transformed, Op::Imported, Op::Attested];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Op> {
        match v {
            0 => Some(Op::Authored),
            1 => Some(Op::Transformed),
            2 => Some(Op::Imported),
            3 => Some(Op::Attested),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Op::Authored => "authored",
            Op::Transformed => "transformed",
            Op::Imported => "imported",
            Op::Attested => "attested",
        }
    }

    pub fn parse(s: &str) -> Option<Op> {
        Op::ALL.iter().copied().find(|o| o.as_str() == s)
    }

    /// Only an import may claim `measured` (rule T). An `Authored` unit at `measured` is
    /// `SMY-W035` at best and a laundering attempt at worst.
    pub const fn may_assign_measured(self) -> bool {
        matches!(self, Op::Imported)
    }
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Trust-ladder position (rule T, §9.3). Bounds the status an agent may assign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Rung {
    /// Deterministic tool, calculation, parser.
    Computed = 0,
    /// User-supplied document or dataset.
    Document = 1,
    /// Fetched content, gated.
    Web = 2,
    /// The model's own parametric knowledge.
    Model = 3,
}

impl Rung {
    pub const ALL: &'static [Rung] = &[Rung::Computed, Rung::Document, Rung::Web, Rung::Model];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Rung> {
        match v {
            0 => Some(Rung::Computed),
            1 => Some(Rung::Document),
            2 => Some(Rung::Web),
            3 => Some(Rung::Model),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Rung::Computed => "computed",
            Rung::Document => "document",
            Rung::Web => "web",
            Rung::Model => "model",
        }
    }

    pub fn parse(s: &str) -> Option<Rung> {
        Rung::ALL.iter().copied().find(|r| r.as_str() == s)
    }

    /// The maximum status assignable at this rung (rule T).
    ///
    /// Rule M prevents laundering inside the graph; this prevents it at entry. A model
    /// asserting from its own priors is capped at `inferred` however confidently it
    /// phrases the claim.
    pub const fn ceiling(self) -> Status {
        match self {
            Rung::Computed => Status::Derived,
            Rung::Document => Status::Cited,
            Rung::Web => Status::Cited,
            Rung::Model => Status::Inferred,
        }
    }

    /// Whether a source reference is required to reach this rung's ceiling.
    pub const fn ceiling_requires_source(self) -> bool {
        self.ceiling().requires_source()
    }
}

impl fmt::Display for Rung {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Attestation
// ---------------------------------------------------------------------------

/// One agent's assertion about one unit (§1.1).
///
/// Not hashed. Attestations accrete: a unit's identity is fixed by its content, and its
/// support grows as agents corroborate it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Attestation {
    pub uid: Uid,
    pub agent: AgentId,
    pub hop: u32,
    pub parents: BTreeSet<Uid>,
    pub ts: Hlc,
    pub op: Op,
    pub rung: Rung,
    /// Hash of the full conditions of the model call that produced this (D-8).
    pub recipe: Option<[u8; 32]>,
    /// Provider- and model-free recipe hash, so the same logical ingest aggregates
    /// across vendors (D-8).
    pub family: Option<[u8; 32]>,
    /// COSE_Sign1, reserved. Not implemented in 0.1 (N9).
    pub sig: Option<Vec<u8>>,
    /// Unknown keys from a future minor version, preserved verbatim.
    pub extra: crate::types::unit::Extra,
}

impl Attestation {
    pub fn new(uid: Uid, agent: AgentId, op: Op, rung: Rung, ts: Hlc) -> Attestation {
        Attestation {
            uid,
            agent,
            hop: 0,
            parents: BTreeSet::new(),
            ts,
            op,
            rung,
            recipe: None,
            family: None,
            sig: None,
            extra: Default::default(),
        }
    }

    pub fn at_hop(mut self, hop: u32) -> Attestation {
        self.hop = hop;
        self
    }

    pub fn with_parents(mut self, parents: BTreeSet<Uid>) -> Attestation {
        self.parents = parents;
        self
    }

    pub fn with_recipe(mut self, recipe: [u8; 32], family: [u8; 32]) -> Attestation {
        self.recipe = Some(recipe);
        self.family = Some(family);
        self
    }

    /// The corroboration group key of §16.4: `(provider, model, recipe)`.
    ///
    /// Two units from the same provider under the same recipe are not independent, so
    /// grouping is correctness rather than an optimisation.
    pub fn corroboration_key(&self) -> (String, Option<[u8; 32]>) {
        (self.agent.as_str().to_string(), self.recipe)
    }

    /// The status ceiling this attestation permits (rule T).
    pub const fn ceiling(&self) -> Status {
        self.rung.ceiling()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(s: &str) -> AgentId {
        AgentId::new(s).unwrap()
    }

    // --- HLC ---------------------------------------------------------------

    #[test]
    fn advancing_past_the_previous_wall_resets_the_counter() {
        let a = agent("model:ollama/llama3.1");
        let prev = Hlc::new(1000, 7, a.clone());
        let next = Hlc::advance(&prev, &a, 2000);
        assert_eq!((next.wall_ms, next.counter), (2000, 0));
    }

    #[test]
    fn a_stalled_clock_increments_the_counter() {
        let a = agent("model:ollama/llama3.1");
        let prev = Hlc::new(1000, 7, a.clone());
        for (wall, expect) in [(1000u64, 8u32), (999, 8), (0, 8)] {
            let next = Hlc::advance(&prev, &a, wall);
            assert_eq!(
                (next.wall_ms, next.counter),
                (1000, expect),
                "a clock that goes backwards must not go backwards"
            );
        }
    }

    #[test]
    fn advance_is_strictly_increasing() {
        let a = agent("human:vladimir");
        let mut h = Hlc::zero(a.clone());
        let mut prev = h.clone();
        for wall in [0u64, 0, 0, 5, 5, 9] {
            h = Hlc::advance(&h, &a, wall);
            assert!(h > prev, "{h} must follow {prev}");
            prev = h.clone();
        }
    }

    #[test]
    fn observe_takes_the_maximum_of_both_clocks() {
        let a = agent("human:a");
        let local = Hlc::new(100, 3, a.clone());
        let remote = Hlc::new(200, 1, agent("human:b"));
        let merged = Hlc::observe(&local, &remote, &a, 50);
        assert_eq!(merged.wall_ms, 200);
        assert_eq!(merged.counter, 2);
        assert_eq!(merged.agent, a);
    }

    #[test]
    fn observe_breaks_ties_by_taking_the_larger_counter() {
        let a = agent("human:a");
        let local = Hlc::new(100, 3, a.clone());
        let remote = Hlc::new(100, 9, agent("human:b"));
        let merged = Hlc::observe(&local, &remote, &a, 100);
        assert_eq!((merged.wall_ms, merged.counter), (100, 10));
    }

    #[test]
    fn hlc_orders_by_wall_then_counter_then_agent() {
        let a = agent("human:a");
        let b = agent("human:b");
        assert!(Hlc::new(1, 0, a.clone()) < Hlc::new(2, 0, a.clone()));
        assert!(Hlc::new(1, 0, a.clone()) < Hlc::new(1, 1, a.clone()));
        assert!(Hlc::new(1, 0, a) < Hlc::new(1, 0, b));
    }

    #[test]
    fn now_reads_the_clock_only_once_and_still_advances() {
        let a = agent("human:vladimir");
        let prev = Hlc::new(u64::MAX, 0, a.clone());
        // The system clock cannot exceed u64::MAX ms, so this exercises the stalled path
        // without depending on the actual time.
        let next = Hlc::now(&prev, &a);
        assert_eq!(next.wall_ms, u64::MAX);
        assert_eq!(next.counter, 1);
    }

    // --- Op and Rung -------------------------------------------------------

    #[test]
    fn op_round_trips() {
        assert_eq!(Op::ALL.len(), 4);
        for &o in Op::ALL {
            assert_eq!(Op::from_u8(o.as_u8()), Some(o));
            assert_eq!(Op::parse(o.as_str()), Some(o));
        }
        assert_eq!(Op::from_u8(4), None);
    }

    #[test]
    fn only_imported_may_assign_measured() {
        let allowed: Vec<&str> = Op::ALL
            .iter()
            .filter(|o| o.may_assign_measured())
            .map(|o| o.as_str())
            .collect();
        assert_eq!(allowed, ["imported"]);
    }

    #[test]
    fn rung_round_trips() {
        assert_eq!(Rung::ALL.len(), 4);
        for &r in Rung::ALL {
            assert_eq!(Rung::from_u8(r.as_u8()), Some(r));
            assert_eq!(Rung::parse(r.as_str()), Some(r));
        }
        assert_eq!(Rung::from_u8(4), None);
    }

    /// The table of §9.3, asserted directly. This is the entry-side half of the
    /// anti-laundering guarantee; the graph-side half is rule M.
    #[test]
    fn trust_ceilings_match_the_rule_t_table() {
        assert_eq!(Rung::Computed.ceiling(), Status::Derived);
        assert_eq!(Rung::Document.ceiling(), Status::Cited);
        assert_eq!(Rung::Web.ceiling(), Status::Cited);
        assert_eq!(Rung::Model.ceiling(), Status::Inferred);
    }

    #[test]
    fn no_rung_reaches_measured() {
        for &r in Rung::ALL {
            assert!(
                r.ceiling() < Status::Measured,
                "{r} must not be able to assign measured - only an instrument may"
            );
        }
    }

    #[test]
    fn document_and_web_ceilings_demand_a_source() {
        assert!(Rung::Document.ceiling_requires_source());
        assert!(Rung::Web.ceiling_requires_source());
        assert!(!Rung::Model.ceiling_requires_source());
        assert!(!Rung::Computed.ceiling_requires_source());
    }

    // --- Attestation -------------------------------------------------------

    #[test]
    fn attestations_default_to_hop_zero_with_no_parents() {
        let a = Attestation::new(
            Uid::from_bytes([1; 32]),
            agent("human:vladimir"),
            Op::Authored,
            Rung::Document,
            Hlc::zero(agent("human:vladimir")),
        );
        assert_eq!(a.hop, 0);
        assert!(a.parents.is_empty());
        assert!(a.recipe.is_none() && a.family.is_none() && a.sig.is_none());
        assert_eq!(a.ceiling(), Status::Cited);
    }

    #[test]
    fn recipe_and_family_are_set_together() {
        let a = Attestation::new(
            Uid::from_bytes([1; 32]),
            agent("model:anthropic/claude-opus-5"),
            Op::Transformed,
            Rung::Model,
            Hlc::zero(agent("model:anthropic/claude-opus-5")),
        )
        .at_hop(3)
        .with_recipe([9; 32], [8; 32]);
        assert_eq!(a.hop, 3);
        assert_eq!(a.recipe, Some([9; 32]));
        assert_eq!(a.family, Some([8; 32]));
    }

    /// Two attestations from the same agent under the same recipe are one corroboration
    /// group, not two - otherwise a single model could corroborate itself.
    #[test]
    fn corroboration_groups_collapse_same_agent_same_recipe() {
        let mk = |ag: &str, r: Option<[u8; 32]>| {
            let mut a = Attestation::new(
                Uid::from_bytes([1; 32]),
                agent(ag),
                Op::Authored,
                Rung::Model,
                Hlc::zero(agent(ag)),
            );
            a.recipe = r;
            a
        };
        let a = mk("model:anthropic/claude-opus-5", Some([1; 32]));
        let b = mk("model:anthropic/claude-opus-5", Some([1; 32]));
        let c = mk("model:anthropic/claude-opus-5", Some([2; 32]));
        let d = mk("model:openai/gpt", Some([1; 32]));
        assert_eq!(a.corroboration_key(), b.corroboration_key());
        assert_ne!(a.corroboration_key(), c.corroboration_key());
        assert_ne!(a.corroboration_key(), d.corroboration_key());
    }

    #[test]
    fn attestations_sort_deterministically_in_a_set() {
        let ag = agent("human:v");
        let mut set = BTreeSet::new();
        for i in [3u8, 1, 2] {
            set.insert(Attestation::new(
                Uid::from_bytes([i; 32]),
                ag.clone(),
                Op::Authored,
                Rung::Document,
                Hlc::zero(ag.clone()),
            ));
        }
        let order: Vec<u8> = set.iter().map(|a| a.uid.as_bytes()[0]).collect();
        assert_eq!(order, [1, 2, 3]);
    }
}
