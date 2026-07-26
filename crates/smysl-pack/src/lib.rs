//! `smysl-pack` - budget-bounded, closure-complete selection (§8, §18).
//!
//! This is what a consuming agent calls instead of asking a model to summarise: same
//! graph, same budget, same thread yields identical bytes. Packing is pure, which is why
//! the cost model is a bundled deterministic estimator rather than a provider tokeniser
//! (D-2).
//!
//! Filled by SM-P9 (estimator, closure expansion, greedy, local improvement) and
//! SM-P10 (exact mode).

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub use smysl_core::error::PackError;

/// Identifier of the cost model a pack was produced under. Recorded in `packinfo` so a
/// truncation is self-describing and budgets are honest about being approximate against
/// any specific model (D-2, N11).
pub const DEFAULT_ESTIMATOR: &str = "smysl/utf8-div4";

/// `cost(t) = ceil(utf8_len(t)/4) + 2` (D-2).
///
/// The token count itself lives in `smysl-core`, because granularity bounds are expressed
/// in the same unit; the `+ 2` here is the per-item framing overhead a pack pays.
pub fn cost(text: &str) -> u32 {
    smysl_core::tokens(text) + 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimator_is_the_documented_formula() {
        assert_eq!(cost(""), 2);
        assert_eq!(cost("a"), 3);
        assert_eq!(cost("abcd"), 3);
        assert_eq!(cost("abcde"), 4);
        // utf8_len, not char count: the estimator is over bytes.
        assert_eq!(cost("\u{00e9}"), 3);
    }

    #[test]
    fn estimator_is_monotone_in_length() {
        let mut prev = cost("");
        let mut s = String::new();
        for _ in 0..64 {
            s.push('x');
            let c = cost(&s);
            assert!(c >= prev);
            prev = c;
        }
    }

    #[test]
    fn estimator_id_is_recorded_verbatim() {
        assert_eq!(DEFAULT_ESTIMATOR, "smysl/utf8-div4");
    }
}
