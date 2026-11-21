//! `ent/1` and `perm/1` (RFC SMYSL-5 §6.1): the plug-in entropy of a symbol sequence, and its
//! Miller–Madow value.
//!
//! Both are functions of integer counts in ascending symbol order, which is what makes them
//! replayable: `value = log₂ N − (1/N)·Σ c·log₂ c`, the sum a left fold over the counts in
//! symbol order, and `mm = value + (K − 1) / (2·N·ln 2)`. One observed symbol is exactly 0
//! (§6.0 step 1), computed from no logarithm at all.

use std::collections::BTreeMap;

use super::num;

/// The counts of a symbol sequence, by symbol, ascending.
pub fn counts(symbols: &[Option<i64>]) -> BTreeMap<i64, u64> {
    let mut c = BTreeMap::new();
    for s in symbols.iter().flatten() {
        *c.entry(*s).or_insert(0u64) += 1;
    }
    c
}

/// A plug-in entropy, before it is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plugin {
    /// Symbols counted.
    pub n: u64,
    /// Distinct symbols observed.
    pub k: u64,
    /// Bits.
    pub value: f64,
    /// Miller–Madow, bits.
    pub mm: f64,
}

/// The plug-in entropy of `counts` and its Miller–Madow value. `None` for no symbols.
pub fn plugin(counts: &BTreeMap<i64, u64>) -> Option<Plugin> {
    let n: u64 = counts.values().sum();
    let k = counts.len() as u64;
    if n == 0 {
        return None;
    }
    if k == 1 {
        return Some(Plugin {
            n,
            k,
            value: 0.0,
            mm: 0.0,
        });
    }
    let nf = n as f64;
    let sum = num::sum(counts.values().map(|&c| {
        let c = c as f64;
        c * num::log2(c)
    }));
    let value = num::log2(nf) - sum / nf;
    let mm = value + (k - 1) as f64 / (2.0 * nf * num::ln(2.0));
    Some(Plugin { n, k, value, mm })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn of(cs: &[(i64, u64)]) -> BTreeMap<i64, u64> {
        cs.iter().copied().collect()
    }

    #[test]
    fn uniform_counts_give_log2_k() {
        let p = plugin(&of(&[(0, 5), (1, 5), (2, 5), (3, 5)])).unwrap();
        assert_eq!((p.n, p.k), (20, 4));
        assert!((p.value - 2.0).abs() < 1e-15, "{}", p.value);
        let bias = 3.0 / (2.0 * 20.0 * std::f64::consts::LN_2);
        assert!((p.mm - (p.value + bias)).abs() < 1e-15);
    }

    #[test]
    fn one_symbol_is_exactly_zero_and_none_is_nothing() {
        assert_eq!(
            plugin(&of(&[(7, 300)])),
            Some(Plugin {
                n: 300,
                k: 1,
                value: 0.0,
                mm: 0.0
            })
        );
        assert_eq!(plugin(&BTreeMap::new()), None);
    }

    #[test]
    fn counts_are_by_symbol_ascending() {
        let c = counts(&[Some(3), None, Some(-1), Some(3), Some(0)]);
        assert_eq!(c.into_iter().collect::<Vec<_>>(), [(-1, 1), (0, 1), (3, 2)]);
    }

    /// A skewed source: p = (1/2, 1/4, 1/4) has 1.5 bits, exactly so in the counts 2:1:1.
    #[test]
    fn a_skewed_source_has_its_entropy() {
        let p = plugin(&of(&[(0, 200), (1, 100), (2, 100)])).unwrap();
        assert!((p.value - 1.5).abs() < 1e-15, "{}", p.value);
    }
}
