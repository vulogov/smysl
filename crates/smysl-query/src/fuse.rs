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

/// RRF's `k`, from SMYSL-2.5 §3.4 — fixed, and recorded in every recipe (OQ-45, decided).
///
/// Not a parameter, because over the fusion this module does it cannot change an order: a unit
/// belongs to one partition, so its fused score is `1 / (k + rank)` for its one rank, which is
/// monotone in the rank for every `k`, and two units at one rank tie whatever `k` is and fall
/// to the uid. `k` moves an order only for a unit ranked by two partitions — one cited from two
/// parts of different languages or manifests — and there a parameter would be one more number
/// a rerun depends on, tuned on whatever corpus it was tuned on. `tests` below holds the first
/// half; `smysl-embed/tests/fusion.rs` measured the other, over lexical-with-semantic fusion,
/// which is the fusion `~hybrid` does not do (OQ-46).
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
    rrf_at(lists, RRF_K)
}

fn rrf_at(lists: &[Ranked], k: f64) -> Vec<Fused> {
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
            let score = placements.iter().map(|p| 1.0 / (k + p.rank as f64)).sum();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(n: u8, score: f32) -> Hit {
        Hit::new(Uid::from_bytes([n; 32]), score)
    }

    fn key(lang: &str) -> PartitionKey {
        PartitionKey::Catalog {
            manifest: "m".to_string(),
            lang: lang.to_string(),
        }
    }

    fn order(fused: &[Fused]) -> Vec<Uid> {
        fused.iter().map(|f| f.uid).collect()
    }

    /// OQ-45: over disjoint partitions, which is what a scope partitions into unless a unit is
    /// cited from two of them, no `k` moves the fused order.
    #[test]
    fn k_cannot_reorder_units_ranked_by_one_partition() {
        let lists = vec![
            Ranked {
                key: key("de"),
                hits: (1..=7).map(|n| hit(n, 9.0 - n as f32)).collect(),
            },
            Ranked {
                key: key("en"),
                hits: (20..=22).map(|n| hit(n, 3.0)).collect(),
            },
            Ranked {
                key: key("ru"),
                hits: (40..=49).rev().map(|n| hit(n, 1.0)).collect(),
            },
        ];
        let at_60 = order(&rrf(&lists));
        for k in [0.5, 1.0, 10.0, 30.0, 100.0, 1000.0, 1e9] {
            assert_eq!(order(&rrf_at(&lists, k)), at_60, "k = {k}");
        }
    }

    /// And the other half, so the first is not true by accident: a unit two partitions rank
    /// trades one good rank against two middling ones, and `k` sets the rate.
    #[test]
    fn k_reorders_a_unit_ranked_twice() {
        let twice = hit(9, 1.0);
        let lists = vec![
            Ranked {
                key: key("de"),
                hits: vec![hit(1, 5.0), hit(2, 4.0), hit(5, 3.0), twice.clone()],
            },
            Ranked {
                key: key("en"),
                hits: vec![hit(3, 5.0), hit(4, 4.0), hit(6, 3.0), twice],
            },
        ];
        // Fourth in both: 2/5 under the first's 1/2 at k = 1, 2/64 over 1/61 at k = 60.
        let first = |k: f64| rrf_at(&lists, k)[0].uid;
        assert_eq!(first(1.0), Uid::from_bytes([1; 32]));
        assert_eq!(first(60.0), Uid::from_bytes([9; 32]));
    }
}
