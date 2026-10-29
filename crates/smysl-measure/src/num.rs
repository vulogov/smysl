//! The arithmetic a measure is allowed (SMYSL-2.6 §3.5).
//!
//! **Transcendentals go through `libm`.** IEEE 754 rounds `+ − × ÷ √` correctly and does not
//! promise anything about `ln` or `exp`: the platform's library chooses, and on the machine
//! the RFC was probed on glibc's `ln` and the `libm` crate disagree in the last bit for 2.0% of
//! a million inputs. A measure's value becomes a payload integer and the payload is part of a
//! uid, so a last bit that differs between Linux and macOS is two different units for one
//! computation. The four wrappers here are the only way into those functions from this crate,
//! and `clippy.toml` beside `Cargo.toml` refuses the `f64` methods that would bypass them.
//!
//! **Sums run in one order.** [`sum`] is a left fold over whatever order the caller gives,
//! which is always a fixed one — `BTreeMap` order, span order, term-id order. No compensation:
//! Kahan summation is deterministic too, but it is a second algorithm a port must reproduce
//! exactly, and the plain fold is the one every language already has.
//!
//! **Quantisation happens once, at the end.** [`q20`] turns an `f64` into the integer a
//! payload stores, `round_half_even(v · 2²⁰)`; nothing before it rounds.

/// Natural logarithm, by `libm`.
pub fn ln(x: f64) -> f64 {
    libm::log(x)
}

/// Base-2 logarithm, by `libm`.
pub fn log2(x: f64) -> f64 {
    libm::log2(x)
}

/// `eˣ`, by `libm`.
pub fn exp(x: f64) -> f64 {
    libm::exp(x)
}

/// `xʸ`, by `libm`.
pub fn powf(x: f64, y: f64) -> f64 {
    libm::pow(x, y)
}

/// `√x`, by `libm`. Correctly rounded under IEEE 754, so no port has a choice to make.
pub fn sqrt(x: f64) -> f64 {
    libm::sqrt(x)
}

/// A left fold in the order given. See the module note on why not compensated.
pub fn sum(xs: impl IntoIterator<Item = f64>) -> f64 {
    xs.into_iter().fold(0.0, |acc, x| acc + x)
}

/// The scale of a fixed-point payload number: 2²⁰.
pub const Q20_SCALE: f64 = 1_048_576.0;

/// The largest magnitude [`q20`] accepts, `2⁶³ / 2²⁰` ≈ 8.8·10¹², exclusive.
pub const Q20_LIMIT: f64 = 8_796_093_022_208.0;

/// `v` as a fixed-point payload number: `round_half_even(v · 2²⁰)`.
///
/// `None` for a value that is not finite or does not fit, so that a NaN can never become a
/// stored zero the way `smysl_core::quantise` makes it one — a measure that produced NaN has
/// failed, and the caller reports it not run.
pub fn q20(v: f64) -> Option<i64> {
    if !v.is_finite() || v.abs() >= Q20_LIMIT {
        return None;
    }
    // Exact: multiplying by a power of two only moves the exponent, so the product is the
    // value itself and the one rounding is the one `round_ties_even` does.
    Some((v * Q20_SCALE).round_ties_even() as i64)
}

/// A fixed-point payload number as an `f64`. Exact: every `i64` whose magnitude is below 2⁵³
/// divides by 2²⁰ without rounding, and [`Q20_LIMIT`] keeps stored values there.
pub fn from_q20(i: i64) -> f64 {
    i as f64 / Q20_SCALE
}

/// A fixed-point payload number in decimal, to `places` places, rounded half to even —
/// computed from the integer and never from a float, so the gist a unit carries is the same
/// text on every machine that holds the same payload.
pub fn render_q20(i: i64, places: u32) -> String {
    render_ratio(i128::from(i), 1_i128 << 20, places)
}

/// `num / den` in decimal to `places` places, rounded half to even, in integer arithmetic.
///
/// `den` must be positive; a zero denominator renders as `undefined`, which is what a ratio
/// with nothing under it is (SMYSL-2.6 §2.1: a measure refuses one before it gets here).
pub fn render_ratio(num: i128, den: i128, places: u32) -> String {
    if den <= 0 {
        return "undefined".to_string();
    }
    let scale = 10_i128.pow(places);
    let neg = num < 0;
    let n = num.unsigned_abs() * scale.unsigned_abs();
    let d = den.unsigned_abs();
    let (mut q, r) = (n / d, n % d);
    // Half to even: up when the remainder is over half, or exactly half and q is odd.
    if 2 * r > d || (2 * r == d && q % 2 == 1) {
        q += 1;
    }
    let s = scale.unsigned_abs();
    let (whole, frac) = (q / s, q % s);
    let sign = if neg && q != 0 { "-" } else { "" };
    if places == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole}.{frac:0width$}", width = places as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn q20_rounds_half_to_even_and_refuses_what_it_cannot_store() {
        assert_eq!(q20(1.0), Some(1 << 20));
        assert_eq!(q20(-1.0), Some(-(1 << 20)));
        // Exactly half a quantum, either side of an even and an odd integer.
        let half = 0.5 / Q20_SCALE;
        assert_eq!(q20(half), Some(0));
        assert_eq!(q20(3.0 * half), Some(2));
        assert_eq!(q20(5.0 * half), Some(2));
        assert_eq!(q20(f64::NAN), None);
        assert_eq!(q20(f64::INFINITY), None);
        assert_eq!(q20(Q20_LIMIT), None);
        assert!(q20(Q20_LIMIT - 1.0).is_some());
        for i in [-5_000_000_i64, -1, 0, 1, 2_393_899, 10_727_114] {
            assert_eq!(q20(from_q20(i)), Some(i), "{i} survives a round trip");
        }
    }

    #[test]
    fn rendering_is_integer_arithmetic_rounded_half_to_even() {
        assert_eq!(render_q20(2_393_899, 4), "2.2830");
        assert_eq!(render_q20(-(1 << 19), 2), "-0.50");
        assert_eq!(render_q20(1 << 20, 0), "1");
        assert_eq!(render_ratio(1, 8, 2), "0.12");
        assert_eq!(render_ratio(3, 8, 2), "0.38");
        assert_eq!(render_ratio(5, 8, 2), "0.62");
        assert_eq!(render_ratio(-1, 3, 4), "-0.3333");
        assert_eq!(render_ratio(-1, 1_000_000, 2), "0.00");
        assert_eq!(render_ratio(7, 0, 2), "undefined");
    }
}
