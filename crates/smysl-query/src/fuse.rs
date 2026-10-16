//! Reciprocal rank fusion (SMYSL-2.5 §3.4, TX-P6 step 3).
//!
//! §3.4 pins the whole of it: `score(u) = Σ_p 1 / (k + rank_p(u))`, `k = 60`, ranks 1-based,
//! partitions summed in sorted partition-key order, `f64`, ties by uid.
//!
//! # Why every one of those clauses is load-bearing
//!
//! Floating-point addition is not associative, so "summed in sorted partition-key order" is
//! not a tidiness rule — it is what makes the fused score a function of the inputs rather than
//! of the order a map happened to iterate. The same reasoning put the summation order in
//! `smysl-retrieve`'s scorer, and it is the same hazard: two runs over one corpus that differ
//! in the last bit of a score produce two different rankings wherever two units were close,
//! and nothing about the output says which run was which.
//!
//! **Ranks and not scores** is the other half. A BM25 score is comparable within one partition
//! and meaningless across two — `idf` is counted over that partition's documents — so a fusion
//! that added scores would be adding numbers measured against different corpora. A rank is
//! ordinal and has no units, which is the property RRF exists to exploit.
//!
//! Ties by uid, because two units with the same fused score have to come back in *some* order
//! and the only one that is a function of the data is the identifier's.

use std::collections::BTreeMap;

use smysl_core::ids::Uid;
use smysl_retrieve::Hit;

use crate::ports::PartitionKey;

/// RRF's `k`, from SMYSL-2.5 §3.4 (OQ-45).
///
/// Recorded in the recipe when there is a recipe to record it in — §3.6 hashes it with seven
/// other inputs, and TX-P8 is where saving happens. Named here so the constant and the clause
/// that fixes it are in one place.
pub const RRF_K: f64 = 60.0;

/// One partition's ranked list, as its own BM25 produced it.
#[derive(Debug, Clone)]
pub struct Ranked {
    pub key: PartitionKey,
    pub hits: Vec<Hit>,
}

/// Where one partition placed a unit, and why.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Placement {
    pub key: PartitionKey,
    /// 1-based, as §3.4 specifies. A rank of 0 would make the first hit's contribution `1/k`
    /// rather than `1/(k+1)` and silently change every fused score.
    pub rank: usize,
    pub score: f32,
    /// What each query term contributed inside this partition (`Hit::terms`), for `--why`.
    pub terms: Vec<(String, f32)>,
}

/// A unit's fused result.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Fused {
    pub uid: Uid,
    pub score: f64,
    /// Every partition that ranked this unit, in sorted partition-key order — which is also
    /// the order the score was summed in.
    pub placements: Vec<Placement>,
}

/// Fuse per-partition rankings into one.
///
/// `lists` is sorted by partition key here rather than trusted to arrive sorted: the summation
/// order is part of the specification, and a function that depends on its caller having got it
/// right is a function whose determinism is somebody else's property.
pub fn rrf(lists: &[Ranked]) -> Vec<Fused> {
    let mut order: Vec<&Ranked> = lists.iter().collect();
    order.sort_by(|a, b| a.key.cmp(&b.key));

    let mut placements: BTreeMap<Uid, Vec<Placement>> = BTreeMap::new();
    for list in order {
        for (at, hit) in list.hits.iter().enumerate() {
            placements.entry(hit.uid).or_default().push(Placement {
                key: list.key.clone(),
                rank: at + 1,
                score: hit.score,
                terms: hit.terms.clone(),
            });
        }
    }

    let mut fused: Vec<Fused> = placements
        .into_iter()
        .map(|(uid, placements)| {
            // Summed in the order the placements were pushed, which is sorted partition-key
            // order because the outer loop was.
            let score = placements
                .iter()
                .map(|p| 1.0 / (RRF_K + p.rank as f64))
                .sum();
            Fused {
                uid,
                score,
                placements,
            }
        })
        .collect();

    fused.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.uid.cmp(&b.uid))
    });
    fused
}
