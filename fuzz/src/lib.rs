//! Store generation driven by fuzzer bytes rather than a seeded PRNG.
//!
//! `merge`, `pack` and `thread` already assert the properties that matter — the
//! join-semilattice laws, constraints C1-C7, rule L — over generated stores. What drives
//! that generation is a fixed-seed xorshift running a fixed number of blind rounds: the
//! same 200 cases, every run, forever. There is no coverage feedback, so a shape the
//! generator is unlikely to reach is a shape it will never reach.
//!
//! 0.4.0 made the difference concrete. Eight defects, all found in minutes, every one of
//! them in `surface` or `cbor` — the only two subsystems with a fuzz target. That is not a
//! fact about where bugs live. It is a fact about where anyone was looking.
//!
//! So the properties stay exactly as they are and the *search* changes. [`Choices`] has the
//! same shape as that xorshift, but every decision comes from the fuzzer's input, which
//! makes each one something coverage feedback can steer. Reaching a rebuttal that closes a
//! cycle stops being a matter of luck.
//!
//! The seeded tests stay where they are. They run in a second under `cargo test` and pin
//! the laws for anyone without a nightly toolchain; these targets go looking.

use smysl_core::{
    canonical_uid, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind, Relation, Role,
    Rung, SourceKind, SourceRef, Status, Step, Thread, ThreadId, ThreadSchema, Uid,
    UnitCoreBuilder,
};
use smysl_graph::Store;

/// A decision source backed by the fuzzer's input.
///
/// Deliberately the same surface as the xorshift in `merge_algebra.rs`, so a generator
/// reads the same either way and the two can be compared.
///
/// Past the end of the input every byte reads as zero. That is safe here *only* because
/// [`generate`] draws its counts before its details: the size of a store is fixed by the
/// first few bytes, so a spent stream can flatten the details but cannot make the store
/// grow without bound.
///
/// The first version of this drained the stream in order and stopped generating when it ran
/// out. Units consumed the whole input, so the relation and thread loops never ran once, and
/// three fuzz targets spent two minutes each asserting the join-semilattice laws over stores
/// that contained no rebuttals, no supersessions and no contentions — the entire class the
/// laws are about. They reported no crashes, which was true and meant nothing. Drawing the
/// plan first is what makes the budget reach the parts that matter.
pub struct Choices<'a> {
    bytes: &'a [u8],
    i: usize,
}

impl<'a> Choices<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Choices { bytes, i: 0 }
    }

    fn byte(&mut self) -> u8 {
        let b = self.bytes.get(self.i).copied().unwrap_or(0);
        self.i += 1;
        b
    }

    /// A value in `0..n`. `n == 0` yields 0 rather than dividing by zero.
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        self.byte() as usize % n
    }

    pub fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in.max(1)) == 0
    }
}

fn agent(n: usize) -> AgentId {
    AgentId::new(format!("model:vendor{}/m", n % 4)).unwrap()
}

/// Generate a store whose units genuinely reference one another.
///
/// Units are built in dependency order with their uids fed forward, because content
/// addressing means a reference can only point at something that already exists. A store
/// generated any other way is a store of dangling references, which exercises the error
/// paths and nothing else.
///
/// Statuses are chosen freely, so some units violate rule M. That is deliberate: `merge`
/// must be a join-semilattice over stores it did not author, including invalid ones.
pub fn generate(c: &mut Choices<'_>, max_units: usize) -> Store {
    let mut records: Vec<Record> = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();

    // The plan, drawn before any detail, so every phase gets a share of the input.
    let unit_count = 1 + c.below(max_units);
    let relation_count = c.below(max_units);
    let thread_count = c.below(4);

    for i in 0..unit_count {
        let gist = format!("unit {} of a generated store", c.below(1000));
        let core = if uids.is_empty() || c.chance(3) {
            if c.chance(2) {
                UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
                    .source(SourceRef::new(SourceKind::Metric, "m"))
                    .build()
            } else {
                UnitCoreBuilder::new(KernelType::Hypothesis, gist, Status::Speculative).build()
            }
        } else {
            let n = 1 + c.below(uids.len().min(3));
            let grounds: Vec<Uid> = (0..n).map(|_| uids[c.below(uids.len())]).collect();
            let status = if c.chance(2) {
                Status::Inferred
            } else {
                Status::Derived
            };
            UnitCoreBuilder::new(KernelType::Claim, gist, status)
                .grounds(grounds)
                .build()
        };
        // A builder can refuse — an empty gist, say. Skip rather than unwrap: a generator
        // that panics on its own input turns every such case into a false crash.
        let Ok(core) = core else { continue };

        let uid = canonical_uid(&core);
        uids.push(uid);
        records.push(Record::Unit(core));

        if c.chance(2) {
            let a = agent(i);
            records.push(Record::Attestation(
                Attestation::new(
                    uid,
                    a.clone(),
                    if c.chance(3) { Op::Imported } else { Op::Authored },
                    [Rung::Computed, Rung::Document, Rung::Web, Rung::Model][c.below(4)],
                    Hlc::new(c.below(100) as u64, c.below(4) as u32, a),
                )
                .at_hop(c.below(5) as u32),
            ));
        }
    }

    // Relations, including the rebuttals that turn into contentions.
    let kinds = [
        RelKind::Rebuts,
        RelKind::Supersedes,
        RelKind::Causes,
        RelKind::Elaborates,
        RelKind::Retracts,
    ];
    for _ in 0..relation_count {
        if uids.len() < 2 {
            break;
        }
        let from = uids[c.below(uids.len())];
        let to = uids[c.below(uids.len())];
        if from == to {
            continue;
        }
        records.push(Record::Relation(Relation::new(
            kinds[c.below(kinds.len())].clone(),
            from,
            to,
        )));
    }

    // Threads, which is what promotes a rebuttal to a live contention.
    for i in 0..thread_count {
        if uids.is_empty() {
            break;
        }
        let a = agent(i);
        let steps: Vec<Step> = (0..1 + c.below(3))
            .map(|_| Step::new(Role::BottomLine, uids[c.below(uids.len())]))
            .collect();
        records.push(Record::Thread(
            Thread::new(
                ThreadId::new(format!("t/x{}", i % 3)).unwrap(),
                ThreadSchema::Brief,
                a.clone(),
                format!("thread {i}"),
                Hlc::new(c.below(10) as u64, c.below(3) as u32, a),
            )
            .with_steps(steps),
        ));
    }

    Store::from_records(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generator must produce stores worth testing.
    ///
    /// A target that runs a million cases against an empty store reports "no crashes" just
    /// as confidently as one that found nothing wrong — and 0.4.0 already produced one
    /// sweep that probed with the wrong value shapes, never entered the decoder it claimed
    /// to cover, and pronounced a whole class clean while a second instance of the bug sat
    /// in the tree. So this asserts the shape of what the fuzzer is actually being handed.
    #[test]
    fn generated_stores_are_not_trivial() {
        let mut sizes = Vec::new();
        let mut with_relations = 0usize;
        let mut with_threads = 0usize;
        // Inputs of the length libFuzzer actually settles on, varied so this measures the
        // generator rather than one lucky byte string.
        for seed in 0u16..600 {
            let bytes: Vec<u8> = (0..64)
                .map(|i| (seed.wrapping_mul(31).wrapping_add(i * 7) % 251) as u8)
                .collect();
            let mut c = Choices::new(&bytes);
            let store = generate(&mut c, 12);
            sizes.push(store.units().count());
            if store.relations().next().is_some() {
                with_relations += 1;
            }
            if store.threads().next().is_some() {
                with_threads += 1;
            }
        }
        let non_empty = sizes.iter().filter(|n| **n > 0).count();
        let biggest = sizes.iter().copied().max().unwrap_or(0);
        assert!(
            non_empty * 2 >= sizes.len(),
            "over half the generated stores were empty ({non_empty} of {} had a unit)",
            sizes.len()
        );
        assert!(biggest >= 4, "the largest generated store held {biggest} units");
        assert!(
            with_relations * 4 >= sizes.len(),
            "only {with_relations} of {} stores carried a relation, so rebuttals and \
             contentions — the cases the join-semilattice laws are actually about — are \
             barely reached",
            sizes.len()
        );
        assert!(
            with_threads > 0,
            "no generated store carried a thread, and a rebuttal only becomes a live \
             contention once a thread selects it"
        );
    }
}
