//! Exact decimal arithmetic over `dec` values (RFC SMYSL-3 §5; RFC SMYSL-5 §5, TI-P0).
//!
//! `smysl_core::Decimal` is a canonical encoding and deliberately has no arithmetic and no
//! numeric order. Two parts of this crate need both without binary64 in the way: window
//! summaries, whose `sum` is exact (§9.3), and the information measures' symbols, which
//! compare `dec` values, subtract them and take the floor of a quotient (SMYSL-5 §5). This is
//! the one place both call.
//!
//! - [`numeric_cmp`]: the numeric order of two `dec`s.
//! - [`BigDec`]: a signed decimal of any length — an exact sum, a difference, a numeric order
//!   ([`BigDec::cmp_value`]), the floor of a quotient by a positive `dec`
//!   ([`BigDec::floor_div`]), and rounding to binary64 once, half to even.
//!
//! Nothing here passes a value through binary64 except the two conversions that say so.

use std::cmp::Ordering;

use smysl_core::Decimal;

/// Numeric order of two decimals, exactly: `Decimal`'s own `Ord` is not numeric.
pub fn numeric_cmp(a: &Decimal, b: &Decimal) -> Ordering {
    let sign = |d: &Decimal| d.mantissa().signum();
    match sign(a).cmp(&sign(b)) {
        Ordering::Equal => {}
        o => return o,
    }
    if sign(a) == 0 {
        return Ordering::Equal;
    }
    // Same sign: compare magnitudes by their leading digit's power, then digit by digit.
    let mag = |d: &Decimal| {
        let digits = d.mantissa().unsigned_abs().to_string();
        let lead = i32::from(d.exponent()) + digits.len() as i32 - 1;
        (lead, digits)
    };
    let ((la, da), (lb, db)) = (mag(a), mag(b));
    let by_mag = la.cmp(&lb).then_with(|| {
        let n = da.len().max(db.len());
        format!("{da:0<n$}").cmp(&format!("{db:0<n$}"))
    });
    if sign(a) < 0 {
        by_mag.reverse()
    } else {
        by_mag
    }
}

/// A signed decimal `± digits × 10^exp`, digits least significant first.
///
/// Its derived equality is of representation; [`BigDec::cmp_value`] is numeric order, which is
/// what every caller outside this module wants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BigDec {
    negative: bool,
    digits: Vec<u8>,
    exp: i32,
}

impl BigDec {
    /// The exact sum of `values`.
    pub(crate) fn sum(values: &[Decimal]) -> BigDec {
        let exp = values
            .iter()
            .map(|v| i32::from(v.exponent()))
            .min()
            .unwrap_or(0);
        // Most windows fit 128 bits: a counter, a gauge with two decimals. Checked, so the first
        // overflow falls through to the digits, and the result is the same either way.
        let fast = values.iter().try_fold(0i128, |acc, v| {
            let mut m = v.mantissa();
            for _ in 0..(i32::from(v.exponent()) - exp) {
                m = m.checked_mul(10)?;
            }
            acc.checked_add(m)
        });
        if let Some(total) = fast {
            let mut digits = Vec::new();
            let mut m = total.unsigned_abs();
            while m > 0 {
                digits.push((m % 10) as u8);
                m /= 10;
            }
            return BigDec {
                negative: total < 0,
                digits,
                exp,
            };
        }
        let mut acc = BigDec {
            negative: false,
            digits: Vec::new(),
            exp,
        };
        for v in values {
            let shift = (i32::from(v.exponent()) - exp) as usize;
            let mut digits = vec![0u8; shift];
            let mut m = v.mantissa().unsigned_abs();
            while m > 0 {
                digits.push((m % 10) as u8);
                m /= 10;
            }
            acc.add(v.mantissa() < 0, &digits);
        }
        acc.trim();
        acc
    }

    fn add(&mut self, negative: bool, digits: &[u8]) {
        if self.negative == negative || self.digits.is_empty() {
            if self.digits.is_empty() {
                self.negative = negative;
            }
            let mut carry = 0u8;
            let n = self.digits.len().max(digits.len());
            self.digits.resize(n, 0);
            for (i, slot) in self.digits.iter_mut().enumerate() {
                let d = *slot + digits.get(i).copied().unwrap_or(0) + carry;
                *slot = d % 10;
                carry = d / 10;
            }
            if carry > 0 {
                self.digits.push(carry);
            }
        } else {
            // Opposite signs: the larger magnitude minus the smaller, with its sign.
            let (big, small, sign) = match cmp_mag(&self.digits, digits) {
                Ordering::Less => (digits.to_vec(), self.digits.clone(), negative),
                _ => (self.digits.clone(), digits.to_vec(), self.negative),
            };
            let mut out = big;
            let mut borrow = 0i8;
            for (i, slot) in out.iter_mut().enumerate() {
                let mut d = *slot as i8 - small.get(i).copied().unwrap_or(0) as i8 - borrow;
                borrow = 0;
                if d < 0 {
                    d += 10;
                    borrow = 1;
                }
                *slot = d as u8;
            }
            self.digits = out;
            self.negative = sign;
        }
        self.trim();
    }

    /// Drop leading zeros; zero is non-negative.
    fn trim(&mut self) {
        while self.digits.last() == Some(&0) {
            self.digits.pop();
        }
        if self.digits.is_empty() {
            self.negative = false;
        }
    }

    /// The digits, most significant first, as text; `"0"` for zero.
    fn text(&self) -> String {
        if self.digits.is_empty() {
            return "0".into();
        }
        self.digits
            .iter()
            .rev()
            .map(|d| (b'0' + d) as char)
            .collect()
    }

    /// As a `dec`, if it is one (§5.2's range).
    pub(crate) fn to_dec(&self) -> Option<Decimal> {
        if self.digits.is_empty() {
            return Some(Decimal::ZERO);
        }
        let zeros = self.digits.iter().take_while(|d| **d == 0).count();
        let m: i128 = self.digits[zeros..]
            .iter()
            .rev()
            .try_fold(0i128, |m, d| m.checked_mul(10)?.checked_add(i128::from(*d)))?;
        let m = if self.negative { -m } else { m };
        Decimal::new(m, i64::from(self.exp) + zeros as i64).ok()
    }

    /// Rounded once to binary64, half to even.
    pub(crate) fn to_f64(&self) -> f64 {
        let sign = if self.negative { "-" } else { "" };
        format!("{sign}{}e{}", self.text(), self.exp)
            .parse()
            .unwrap_or(f64::NAN)
    }

    /// `self / n`, rounded once to binary64, half to even: the quotient to 800 significant
    /// digits and a sticky digit (see the module comment). `NaN` for `n = 0`.
    pub(crate) fn div_to_f64(&self, n: u64) -> f64 {
        if n == 0 {
            return f64::NAN;
        }
        const SIGNIFICANT: usize = 800;
        let n = u128::from(n);
        let mut quotient = String::new();
        let mut rem: u128 = 0;
        let mut significant = 0usize;
        let mut frac = 0i64;
        let mut digits = self.digits.iter().rev().copied().peekable();
        loop {
            let (d, integer) = match digits.next() {
                Some(d) => (d, true),
                None if rem == 0 || significant >= SIGNIFICANT => break,
                None => (0, false),
            };
            if !integer {
                frac += 1;
            }
            rem = rem * 10 + u128::from(d);
            let q = (rem / n) as u8;
            rem %= n;
            if q != 0 || significant > 0 {
                quotient.push((b'0' + q) as char);
                significant += 1;
            } else if !integer {
                quotient.push('0');
            }
        }
        if rem != 0 {
            quotient.push('1');
            frac += 1;
        }
        if quotient.is_empty() {
            quotient.push('0');
        }
        let sign = if self.negative { "-" } else { "" };
        format!("{sign}{quotient}e{}", i64::from(self.exp) - frac)
            .parse()
            .unwrap_or(f64::NAN)
    }
}

fn cmp_mag(a: &[u8], b: &[u8]) -> Ordering {
    let len = |d: &[u8]| d.len() - d.iter().rev().take_while(|x| **x == 0).count();
    let (la, lb) = (len(a), len(b));
    la.cmp(&lb)
        .then_with(|| a[..la].iter().rev().cmp(b[..lb].iter().rev()))
}

/// The longest quotient [`BigDec::floor_div`] attempts, in digits of its dividend after scaling:
/// past it the quotient is beyond `i64` whatever the divisor, since a `dec` mantissa is under
/// 2⁶⁴ < 10²⁰.
const FLOOR_DIV_DIGITS: usize = 40;

// What SMYSL-5 §5 needs: `info::grid` and `info::symbols` call it.
impl BigDec {
    /// `v`, exactly.
    pub(crate) fn from_dec(v: &Decimal) -> BigDec {
        let mut digits = Vec::new();
        let mut m = v.mantissa().unsigned_abs();
        while m > 0 {
            digits.push((m % 10) as u8);
            m /= 10;
        }
        let mut out = BigDec {
            negative: v.mantissa() < 0,
            digits,
            exp: i32::from(v.exponent()),
        };
        out.trim();
        out
    }

    /// −1, 0 or 1.
    pub(crate) fn signum(&self) -> i8 {
        match (self.digits.is_empty(), self.negative) {
            (true, _) => 0,
            (false, true) => -1,
            (false, false) => 1,
        }
    }

    /// The digits scaled to `exp`, which is at most `self.exp`: zeros below.
    fn scaled_to(&self, exp: i32) -> Vec<u8> {
        let shift = (self.exp - exp) as usize;
        let mut d = vec![0u8; shift];
        d.extend_from_slice(&self.digits);
        d
    }

    /// `self − other`, exactly.
    pub(crate) fn sub(&self, other: &BigDec) -> BigDec {
        let exp = self.exp.min(other.exp);
        let mut out = BigDec {
            negative: self.negative,
            digits: self.scaled_to(exp),
            exp,
        };
        out.trim();
        if other.signum() != 0 {
            out.add(!other.negative, &other.scaled_to(exp));
        }
        out
    }

    /// Numeric order. Representation does not enter: `10·10⁰` and `1·10¹` are equal.
    pub(crate) fn cmp_value(&self, other: &BigDec) -> Ordering {
        match self.signum().cmp(&other.signum()) {
            Ordering::Equal if self.signum() == 0 => Ordering::Equal,
            Ordering::Equal => {
                let exp = self.exp.min(other.exp);
                let by_mag = cmp_mag(&self.scaled_to(exp), &other.scaled_to(exp));
                if self.negative {
                    by_mag.reverse()
                } else {
                    by_mag
                }
            }
            o => o,
        }
    }

    /// `floor(self / w)`, exactly, for `w > 0`. `None` when `w` is not positive or the
    /// quotient is outside `i64`. This is SMYSL-5 §5.3's `width:w` symbol.
    pub(crate) fn floor_div(&self, w: &Decimal) -> Option<i64> {
        if w.mantissa() <= 0 {
            return None;
        }
        if self.signum() == 0 {
            return Some(0);
        }
        let divisor = w.mantissa().unsigned_abs();
        // self / w = ±D·10^k / divisor, D the digits as an integer.
        let k = self.exp - i32::from(w.exponent());
        // The integer part of D·10^k, most significant first, and whether a non-zero digit was
        // dropped below the point.
        let (whole, mut inexact): (Vec<u8>, bool) = if k >= 0 {
            if self.digits.len() + k as usize > FLOOR_DIV_DIGITS {
                return None;
            }
            let mut d = vec![0u8; k as usize];
            d.extend_from_slice(&self.digits);
            (d.into_iter().rev().collect(), false)
        } else {
            let drop = (-k) as usize;
            if drop >= self.digits.len() {
                (Vec::new(), true)
            } else {
                if self.digits.len() - drop > FLOOR_DIV_DIGITS {
                    return None;
                }
                let inexact = self.digits[..drop].iter().any(|d| *d != 0);
                (self.digits[drop..].iter().rev().copied().collect(), inexact)
            }
        };
        // Long division by the divisor: the remainder stays below it, so `rem·10 + 9` fits u128.
        let mut q: u128 = 0;
        let mut rem: u128 = 0;
        for d in whole {
            rem = rem * 10 + u128::from(d);
            q = q.checked_mul(10)?.checked_add(rem / divisor)?;
            rem %= divisor;
        }
        inexact |= rem != 0;
        let q = i128::try_from(q).ok()?;
        let q = match (self.negative, inexact) {
            (false, _) => q,
            (true, false) => -q,
            (true, true) => -q - 1,
        };
        i64::try_from(q).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(t: &str) -> Decimal {
        Decimal::parse_token(t).unwrap()
    }

    fn b(t: &str) -> BigDec {
        BigDec::from_dec(&d(t))
    }

    #[test]
    fn a_difference_is_exact_at_any_scale() {
        for (x, y, want) in [
            ("5", "3", "2"),
            ("3", "5", "-2"),
            ("0.1", "0.3", "-0.2"),
            ("-2.5", "-2.5", "0"),
            ("1e19", "1", "9999999999999999999"),
            (
                "9223372036854775807",
                "-9223372036854775808",
                "18446744073709551615",
            ),
            ("1e-300", "0", "1e-300"),
            ("0", "7.25", "-7.25"),
        ] {
            let got = b(x).sub(&b(y));
            assert_eq!(
                got.cmp_value(&b(want)),
                Ordering::Equal,
                "{x} - {y}: {got:?}"
            );
        }
    }

    /// A difference past `dec`'s range is still a value: `1e300 − 1e-300` has 601 digits.
    #[test]
    fn a_difference_past_dec_is_kept_and_ordered() {
        let wide = b("18446744073709551615").sub(&b("-18446744073709551613"));
        assert!(wide.to_dec().is_none(), "a mantissa over 2^64 - 1");
        assert_eq!(
            wide.cmp_value(&b("3.6893488147419103e19")),
            Ordering::Greater
        );
        assert_eq!(wide.cmp_value(&b("3.6893488147419104e19")), Ordering::Less);
        let big = b("1e300").sub(&b("1e-300"));
        assert!(big.to_dec().is_none());
        assert_eq!(big.cmp_value(&b("1e300")), Ordering::Less);
        assert_eq!(big.cmp_value(&b("9.99e299")), Ordering::Greater);
    }

    #[test]
    fn numeric_order_agrees_with_numeric_cmp() {
        let xs = [
            "-1e5",
            "-12.5",
            "-1",
            "-0.001",
            "0",
            "1e-9",
            "0.1",
            "0.10000001",
            "1",
            "10",
            "1e1",
            "99.999",
            "100",
            "18446744073709551615",
        ];
        for x in xs {
            for y in xs {
                assert_eq!(
                    b(x).cmp_value(&b(y)),
                    numeric_cmp(&d(x), &d(y)),
                    "{x} against {y}"
                );
            }
        }
    }

    #[test]
    fn the_floor_of_a_quotient_rounds_toward_negative_infinity() {
        for (x, w, want) in [
            ("7", "2", 3),
            ("-7", "2", -4),
            ("6", "2", 3),
            ("-6", "2", -3),
            ("0.75", "0.25", 3),
            ("-0.75", "0.25", -3),
            ("-0.7", "0.25", -3),
            ("0.0001", "1000", 0),
            ("-0.0001", "1000", -1),
            ("1e10", "0.001", 10_000_000_000_000),
            ("123.456", "1e-3", 123_456),
            ("123.4567", "1e-3", 123_456),
            ("-123.4567", "1e-3", -123_457),
            ("0", "0.5", 0),
            ("18446744073709551615", "18446744073709551615", 1),
            ("9223372036854775807", "1", i64::MAX),
        ] {
            assert_eq!(b(x).floor_div(&d(w)), Some(want), "floor({x} / {w})");
        }
    }

    #[test]
    fn a_quotient_outside_i64_or_a_divisor_not_positive_is_none() {
        assert_eq!(b("9223372036854775808").floor_div(&d("1")), None);
        assert_eq!(b("1e300").floor_div(&d("1")), None);
        assert_eq!(b("1").floor_div(&d("0")), None);
        assert_eq!(b("1").floor_div(&d("-1")), None);
        assert_eq!(b("-9223372036854775808").floor_div(&d("1")), Some(i64::MIN));
    }

    /// The window sum still rounds once: the module only moved.
    #[test]
    fn the_exact_sum_is_unchanged_by_the_move() {
        let s = BigDec::sum(&[d("0.1"), d("0.2"), d("-0.3")]);
        assert_eq!(s.signum(), 0);
        let s = BigDec::sum(&[d("1e20"), d("1")]);
        assert_eq!(s.to_dec(), None, "beyond dec's mantissa");
        assert_eq!(BigDec::sum(&[d("2"), d("3")]).div_to_f64(2), 2.5);
    }
}
