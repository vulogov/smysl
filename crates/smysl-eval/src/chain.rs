//! The multi-hop chain (§28).
//!
//! A hop is one handoff. The smysl arm's hop is `pack`: the receiving system is given a
//! budget-bounded selection of the graph rather than a re-summarised paragraph, and what it
//! passes on is what it was given. Five hops is where the prose baseline's losses are
//! expected to have compounded past arguing about.
//!
//! **The prose arm is not simulated.** Every hop of it is a model call, and a baseline
//! produced by guessing what a model would have dropped would be a number about this file
//! rather than about prose. [`Arm::Prose`] is therefore declared here and driven from
//! outside; a run that had no provider reports the smysl arm and says the baseline did not
//! run, which is the honest shape for a comparison with one side missing.

use std::collections::BTreeSet;

use smysl_core::{Lod, Uid};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, Estimator, PackRequest, Selection};

/// Which side of the comparison a run belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Arm {
    /// Each hop reads prose and emits prose. Requires a model at every hop.
    Prose,
    /// Each hop reads a pack and emits units. Deterministic; no model anywhere.
    Smysl,
}

impl Arm {
    pub const fn as_str(self) -> &'static str {
        match self {
            Arm::Prose => "prose",
            Arm::Smysl => "smysl",
        }
    }

    /// Whether running this arm needs a provider. E1 and E2 are only a *comparison* when
    /// both arms ran, and this is what lets a report say so rather than imply it.
    pub const fn needs_model(self) -> bool {
        matches!(self, Arm::Prose)
    }
}

/// What each hop is allowed to spend.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Budget {
    /// A fixed number of estimator tokens.
    Tokens(u64),
    /// A fraction of what handing the whole graph over at full detail would cost.
    ///
    /// The default, and not merely a convenience: an absolute budget large enough for one
    /// fixture is not binding on a smaller one, and a chain whose budget never binds
    /// reports E1 = 1.0 and E2 = 1.0 on every input. Those are the numbers a harness
    /// measuring nothing produces, and they are indistinguishable from a spectacular
    /// result. A fraction binds on any input by construction.
    Fraction(f64),
}

impl Budget {
    /// Resolve against the cost of the whole input.
    pub fn tokens(&self, full: u64) -> u64 {
        match self {
            Budget::Tokens(t) => *t,
            Budget::Fraction(f) => ((full as f64) * f.clamp(0.0, 1.0)).round() as u64,
        }
    }
}

/// How to run a chain.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ChainOptions {
    /// Handoffs to perform. §28 measures survival across five.
    pub hops: usize,
    /// What each hop may spend.
    pub budget: Budget,
    /// Units the chain must carry if it carries anything. Empty means no focus.
    pub focus: BTreeSet<Uid>,
}

impl Default for ChainOptions {
    fn default() -> ChainOptions {
        ChainOptions {
            hops: 5,
            // Enough to keep every unit and not every body, which is the regime the format
            // is built for: shed detail, keep the claim and its rebuttal.
            budget: Budget::Fraction(0.6),
            focus: BTreeSet::new(),
        }
    }
}

/// What one handoff produced.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Hop {
    /// 1-based: hop 0 is the input, not a handoff.
    pub index: usize,
    /// Estimator tokens the receiving system is handed, at the levels selected.
    pub tokens: u64,
    /// What survived, and at which level of detail each unit survived.
    pub selection: Selection,
    /// Units that survived this hop, without their levels.
    pub surviving: BTreeSet<Uid>,
    /// Units carrying a rebuttal that survived, whose rebuttal also survived. Rule R is
    /// about exactly this pair travelling together.
    pub rebuttals_honoured: usize,
    /// Surviving units that carry a rebuttal at all - the denominator for the above, and
    /// the number that says whether rule R had anything to bind on.
    pub rebuttals_possible: usize,
    /// True when the budget could not hold the mandatory floor. Packing fails rather than
    /// emitting a claim without its rebuttal, so this is a refusal, not a loss.
    pub refused: bool,
}

/// A whole chain.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Run {
    pub arm: Arm,
    /// Units present before any handoff.
    pub initial: BTreeSet<Uid>,
    /// Estimator tokens of the whole input at full detail, before any budget applied.
    pub initial_tokens: u64,
    pub hops: Vec<Hop>,
}

impl Run {
    /// Units still present after the final hop.
    pub fn survivors(&self) -> &BTreeSet<Uid> {
        match self.hops.last() {
            Some(h) => &h.surviving,
            None => &self.initial,
        }
    }
}

/// Estimator tokens for one unit **at the level it was selected at**.
///
/// The level is the whole point: a pack that keeps a unit at `L0` ships its gist and not
/// its body, so counting gist-plus-body regardless would charge the chain for text it never
/// sent - and would report no saving on a corpus where the saving is exactly the bodies
/// that were left behind. Measured with the same estimator `pack` budgets against, so E1 is
/// in the units the budget was spent in.
pub fn unit_tokens(store: &Store, uid: &Uid, level: Lod, est: &Estimator) -> u64 {
    store
        .get(uid)
        .map(|u| est.unit(&u.core, level))
        .unwrap_or(0)
}

/// Tokens for a whole selection, at each unit's own level.
fn selection_tokens(store: &Store, sel: &Selection, est: &Estimator) -> u64 {
    sel.iter()
        .map(|(uid, level)| unit_tokens(store, uid, *level, est))
        .sum()
}

/// The cheapest way to carry every unit: all of them at `L0`, gist only.
///
/// This is the line below which units *have* to be dropped. Above it, a budget can be met
/// by shedding detail alone, so a unit going missing means something other than arithmetic
/// decided it - which is what makes this worth exposing rather than inlining.
pub fn floor_tokens(store: &Store, uids: &BTreeSet<Uid>, est: &Estimator) -> u64 {
    uids.iter()
        .filter_map(|u| store.get(u))
        .map(|u| est.unit(&u.core, Lod::L0))
        .sum()
}

/// The input's cost with nothing left behind: every unit at its richest available level.
/// This is the denominator E1 is a fraction of - what handing the whole graph over costs.
pub fn full_tokens(store: &Store, uids: &BTreeSet<Uid>, est: &Estimator) -> u64 {
    uids.iter()
        .filter_map(|u| store.get(u))
        .map(|u| {
            let level = smysl_pack::available_levels(&u.core)
                .last()
                .copied()
                .unwrap_or(Lod::L0);
            est.unit(&u.core, level)
        })
        .sum()
}

/// Count the rebuttal pairs among a surviving set.
///
/// `possible` counts survivors that are rebutted by anything in the original graph;
/// `honoured` counts those whose rebutter survived with them. Rule R is the claim that
/// these two numbers are equal, so measuring them apart is what makes the claim falsifiable
/// rather than assumed.
fn rebuttal_pairs(store: &Store, surviving: &BTreeSet<Uid>) -> (usize, usize) {
    let mut possible = 0;
    let mut honoured = 0;
    for uid in surviving {
        let rebutters = store.rebuttals_of(uid);
        if rebutters.is_empty() {
            continue;
        }
        possible += 1;
        if rebutters.iter().all(|r| surviving.contains(r)) {
            honoured += 1;
        }
    }
    (possible, honoured)
}

/// Run the smysl arm: `hops` successive packs, each over what the last one passed on.
///
/// No model is consulted, so the whole run is a pure function of the store and the options
/// - which is the property that makes E1 a cost rather than a price.
pub fn run_smysl_arm(store: &Store, opts: &ChainOptions) -> Run {
    let est = Estimator::default();
    let initial: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
    let mut run = Run {
        arm: Arm::Smysl,
        initial_tokens: full_tokens(store, &initial, &est),
        initial: initial.clone(),
        hops: Vec::with_capacity(opts.hops),
    };

    // Resolved once, against the whole input: a fraction recomputed per hop would shrink
    // with what it is measuring and drive the chain to nothing regardless of the format.
    let budget = opts.budget.tokens(run.initial_tokens);

    let mut carried = initial;
    for index in 1..=opts.hops {
        let mut req = PackRequest::default();
        req.budget = budget;
        req.estimator = est.clone();
        req.scope = carried.clone();
        req.focus = opts
            .focus
            .iter()
            .copied()
            .filter(|u| carried.contains(u))
            .collect();
        let sal = salience(store, &SalienceRequest::default());

        let (selection, refused) = match pack(store, &sal, &req) {
            Ok(p) => (p.selection, false),
            // A budget that cannot hold the mandatory floor is a refusal. Recording it as
            // an empty hop would read as total loss, which is the opposite of what
            // happened: nothing was shipped *because* shipping it would have been lossy.
            Err(_) => (Selection::new(), true),
        };

        let surviving: BTreeSet<Uid> = selection.keys().copied().collect();
        let (rebuttals_possible, rebuttals_honoured) = rebuttal_pairs(store, &surviving);
        run.hops.push(Hop {
            index,
            tokens: selection_tokens(store, &selection, &est),
            rebuttals_possible,
            rebuttals_honoured,
            selection,
            surviving: surviving.clone(),
            refused,
        });

        if refused {
            break;
        }
        carried = surviving;
    }

    run
}
