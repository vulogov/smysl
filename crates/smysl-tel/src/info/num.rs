//! The floating-point arithmetic an information measure is allowed (RFC SMYSL-5 §6, D-I4).
//!
//! **Transcendentals go through `libm`,** pinned by the workspace at the version
//! `smysl-measure` uses. The platform's `ln` and `log2` may differ in the last bit between
//! machines (SMYSL-2.6 §2.9), and a stored number is part of a uid. These wrappers are the only
//! way into those functions from this crate: `clippy.toml` refuses the `f64` methods.
//!
//! **Sums run in one order.** [`sum`] is a left fold over the order the caller gives, which is
//! always ascending symbol order (pairs lexicographic). No compensation, for the reason
//! `smysl-measure`'s `num::sum` gives: it is a second algorithm every port must reproduce.
//!
//! **A result is stored once, at the end** (§6.0): a negative statistic is clamped to zero
//! ([`stored_statistic`]), and every stored number is rounded to [`PLACES`] decimal places, half
//! to even on the binary64's exact value, then written as a `dec` ([`stored`]).

use smysl_core::Decimal;

/// The decimal places a stored statistic keeps (D-I13).
pub const PLACES: usize = 6;

/// Natural logarithm, by `libm`.
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// Base-2 logarithm, by `libm`.
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

/// A left fold in the order given.
pub fn sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    xs.into_iter().fold(0.0, |acc, x| acc + x)
}

/// `v` rounded to six places, half to even, as a `dec` (§6.0 step 3). `None` for a value that is
/// not finite: a statistic that produced one has failed, and is reported, never stored as zero.
///
/// The rounding is Rust's fixed-precision formatting, which works on the binary64's exact
/// decimal expansion and breaks a tie to even. So `0.0078125`, exactly halfway, is `0.007812`,
/// and `0.0000005`, which binary64 holds a little below halfway, is `0`.
pub fn stored(v: f64) -> Option<Decimal> {
    if !v.is_finite() {
        return None;
    }
    let text = format!("{v:.PLACES$}");
    // `-0.000000`, a tiny negative rounded away, is zero.
    let text = text
        .strip_prefix('-')
        .filter(|t| t.bytes().all(|b| b == b'0' || b == b'.'))
        .unwrap_or(&text);
    Decimal::parse_token(text).ok()
}

/// A statistic's `value`, or an element of `values` (§6.0 steps 2 and 3): every statistic here
/// is non-negative in exact arithmetic, so a negative binary64 result is rounding and is
/// clamped to zero before it is stored. `bias` and `mm` go through [`stored`] alone.
pub fn stored_statistic(v: f64) -> Option<Decimal> {
    stored(if v < 0.0 { 0.0 } else { v })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(t: &str) -> Decimal {
        Decimal::parse_token(t).unwrap()
    }

    #[test]
    fn logarithms_are_libm_and_exact_where_they_should_be() {
        assert_eq!(log2(1.0), 0.0);
        assert_eq!(log2(8.0), 3.0);
        assert_eq!(log2(1024.0), 10.0);
        assert_eq!(ln(1.0), 0.0);
        assert_eq!(log2(3.0).to_bits(), libm::log2(3.0).to_bits());
        assert_eq!(sum([0.1, 0.2, 0.3]), (0.1 + 0.2) + 0.3);
    }

    #[test]
    fn a_stored_number_has_six_places_and_ties_go_to_even() {
        assert_eq!(stored(0.831_294_12), Some(d("0.831294")));
        assert_eq!(
            stored(0.831_294_5),
            Some(d("0.831295")),
            "above the tie in binary64"
        );
        // Exact binary ties: 1/128 and 3/128 have seven decimals ending in 5.
        assert_eq!(stored(0.007_812_5), Some(d("0.007812")));
        assert_eq!(stored(0.023_437_5), Some(d("0.023438")));
        assert_eq!(stored(2.0), Some(d("2")));
        assert_eq!(stored(1.0e-7), Some(Decimal::ZERO));
        assert_eq!(stored(-1.0e-17), Some(Decimal::ZERO));
        assert_eq!(stored(-0.0), Some(Decimal::ZERO));
        assert_eq!(stored(-0.25), Some(d("-0.25")));
        assert_eq!(stored(123_456.789_012_3), Some(d("123456.789012")));
        assert_eq!(stored(f64::NAN), None);
        assert_eq!(stored(f64::INFINITY), None);
    }

    /// The canonical text is what rule TQ compares, so a quote of six places verifies.
    #[test]
    fn the_stored_text_is_what_a_quote_gives() {
        assert_eq!(stored(0.831_294_12).unwrap().to_string(), "0.831294");
        assert_eq!(stored(0.5).unwrap().to_string(), "0.5");
    }

    #[test]
    fn a_negative_statistic_is_clamped_to_zero() {
        assert_eq!(stored_statistic(-1.0e-17), Some(Decimal::ZERO));
        assert_eq!(stored_statistic(-0.3), Some(Decimal::ZERO));
        assert_eq!(stored_statistic(0.4128713), Some(d("0.412871")));
        assert_eq!(stored_statistic(f64::NAN), None);
    }
}
