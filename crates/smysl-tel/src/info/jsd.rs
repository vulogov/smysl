//! `jsd/1` (RFC SMYSL-5 §6.2): the Jensen–Shannon divergence between a window's symbol
//! distribution and a reference window's, in bits.
//!
//! `value = H(M) − ½·H(P) − ½·H(Q)`, `M` the equal mixture, each `H` §6.1's plug-in. The
//! mixture is itself a plug-in over integers: `M(s) = (c_P(s)·N_Q + c_Q(s)·N_P) / (2·N_P·N_Q)`,
//! so `H(M)` is the entropy of the weights `c_P(s)·N_Q + c_Q(s)·N_P`, which sum to
//! `2·N_P·N_Q`, and no probability is ever formed.

use std::collections::BTreeMap;

use super::entropy;

/// The divergence between the counts `p` and `q`, before it is stored. `None` when either is
/// empty or a weight would overflow.
pub fn divergence(p: &BTreeMap<i64, u64>, q: &BTreeMap<i64, u64>) -> Option<f64> {
    let (np, nq): (u64, u64) = (p.values().sum(), q.values().sum());
    let hp = entropy::plugin(p)?.value;
    let hq = entropy::plugin(q)?.value;
    let mut w: BTreeMap<i64, u64> = BTreeMap::new();
    for (s, c) in p {
        *w.entry(*s).or_insert(0) += c.checked_mul(nq)?;
    }
    for (s, c) in q {
        let e = w.entry(*s).or_insert(0);
        *e = e.checked_add(c.checked_mul(np)?)?;
    }
    let hm = entropy::plugin(&w)?.value;
    Some(hm - 0.5 * hp - 0.5 * hq)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(cs: &[(i64, u64)]) -> BTreeMap<i64, u64> {
        cs.iter().copied().collect()
    }

    #[test]
    fn equal_distributions_are_zero_and_disjoint_ones_one_bit() {
        let p = of(&[(0, 10), (1, 30)]);
        let q = of(&[(0, 5), (1, 15)]);
        assert!(divergence(&p, &q).unwrap().abs() < 1e-12);
        let d = divergence(&of(&[(0, 7)]), &of(&[(1, 300)])).unwrap();
        assert!((d - 1.0).abs() < 1e-12, "{d}");
        assert_eq!(divergence(&p, &BTreeMap::new()), None);
    }

    /// P = (1/2, 1/2, 0), Q = (0, 1/2, 1/2): M = (1/4, 1/2, 1/4), H(M) = 1.5, so 0.5 bits.
    #[test]
    fn half_overlap_is_half_a_bit() {
        let d = divergence(&of(&[(0, 50), (1, 50)]), &of(&[(1, 20), (2, 20)])).unwrap();
        assert!((d - 0.5).abs() < 1e-12, "{d}");
    }
}
