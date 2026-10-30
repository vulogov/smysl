//! Krippendorff's α, nominal, for GE-T9 (SMYSL-2.6 §3.11, §5.5): how far annotators agree, the
//! ceiling every model figure is read against.
//!
//! The coincidence-matrix form, tolerant of missing values (Krippendorff 2011, "Computing
//! Krippendorff's Alpha-Reliability"): a unit with fewer than two values is not pairable and
//! adds nothing; each ordered pair of values within a unit of m values adds `1/(m − 1)` to its
//! cell; `α = 1 − (n − 1) · D_o / Σ_{c≠k} n_c n_k`, with `D_o` the off-diagonal mass and `n_c`
//! the margins. It is computed **in integers**: every cell is scaled by the least common
//! multiple of the `m − 1`, which cancels, so α is an exact fraction (`Alpha`). There is no α
//! when nothing is pairable or every value is the same — no disagreement was possible, and
//! 1 would claim an agreement that was never tested.
//!
//! [`alpha`] reads the matrix from attestations: an annotator is an agent, and its value for
//! an item — a unit, or a `same-as` edge by its relation uid — is whether it attested the item
//! at rung `document`. Every annotator has a value for every item, present or absent.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{AgentId, Rung, Uid};
use smysl_graph::Store;

use crate::graph::ratio_q20;
use crate::Num;

/// An exact α, `num / den` with `den > 0`; `num` can be negative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Alpha {
    pub num: i128,
    pub den: i128,
}

impl Alpha {
    /// α in fixed point, rounded half to even once.
    pub fn to_num(&self) -> Option<Num> {
        ratio_q20(self.num, self.den).map(Num::Fixed)
    }

    /// Whether this α equals `n / d`, compared as fractions.
    pub fn is(&self, n: i128, d: i128) -> bool {
        self.num * d == n * self.den
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// α over `units`, each the values its coders gave it (`None` for a coder who gave none).
pub fn alpha_nominal<V: Ord>(units: &[Vec<Option<V>>]) -> Option<Alpha> {
    let pairable: Vec<Vec<&V>> = units
        .iter()
        .map(|u| u.iter().flatten().collect::<Vec<&V>>())
        .filter(|v| v.len() >= 2)
        .collect();
    let l = pairable
        .iter()
        .map(|v| v.len() as u64 - 1)
        .fold(1u64, |l, m| l / gcd(l, m) * m);
    // Cells scaled by l: a unit of m values adds l/(m − 1) per ordered pair.
    let mut cells: BTreeMap<(&V, &V), i128> = BTreeMap::new();
    for v in &pairable {
        let w = i128::from(l / (v.len() as u64 - 1));
        for (i, a) in v.iter().enumerate() {
            for (j, b) in v.iter().enumerate() {
                if i != j {
                    *cells.entry((*a, *b)).or_default() += w;
                }
            }
        }
    }
    let mut margin: BTreeMap<&V, i128> = BTreeMap::new();
    for ((c, _), x) in &cells {
        *margin.entry(*c).or_default() += x;
    }
    let n: i128 = margin.values().sum();
    if n == 0 {
        return None;
    }
    let off: i128 = cells
        .iter()
        .filter(|((c, k), _)| c != k)
        .map(|(_, x)| x)
        .sum();
    let total: i128 = margin.values().sum::<i128>();
    let same: i128 = margin.values().map(|m| m * m).sum();
    let expected = total * total - same;
    if expected == 0 {
        return None;
    }
    // α = 1 − (n/l − 1)(off/l) / (expected/l²) = 1 − (n − l)·off / expected.
    let l = i128::from(l);
    Some(Alpha {
        num: expected - (n - l) * off,
        den: expected,
    })
}

/// α over `items` among `agents`, from their `document`-rung attestations.
pub fn alpha(store: &Store, items: &[Uid], agents: &[AgentId]) -> Option<Alpha> {
    let units: Vec<Vec<Option<bool>>> = items
        .iter()
        .map(|item| {
            let by: BTreeSet<&AgentId> = store
                .attestations_of(item)
                .iter()
                .filter(|a| a.rung == Rung::Document)
                .map(|a| &a.agent)
                .collect();
            agents.iter().map(|g| Some(by.contains(g))).collect()
        })
        .collect();
    alpha_nominal(&units)
}
