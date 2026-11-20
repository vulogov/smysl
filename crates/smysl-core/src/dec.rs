//! `dec`, the canonical decimal (RFC SMYSL-3 §5, TL-P1).
//!
//! **Why the kernel float will not do.** A kernel float is binary32 after quantisation to 1/1024
//! (`crate::cbor`, rule 4). That is right for weights and gaps, and it loses a latency of
//! 0.0004 s and every counter above 2²⁴. Changing it would move every uid that has a float, so
//! telemetry values take a type of their own instead, defined by `x.tel/v1` for its payloads and
//! proposed for A-7.
//!
//! **What a `dec` is.** A pair `[m, e]` with value m × 10^e, on the wire a CBOR array of two
//! shortest-form integers. One value has one pair: `m` carries no trailing zero, and zero is
//! `[0, 0]`. So `41`, `41.0` and `4.10e1` are one value, one encoding and one uid.
//!
//! **Where one comes from.** A decimal token is read exactly as written, never through binary64
//! ([`Decimal::parse_token`]). A binary64 is the decimal ECMA-262's `Number::toString` selects
//! ([`Decimal::from_f64`]). A typed integer is itself ([`Decimal::from_i64`],
//! [`Decimal::from_u64`]). Each has exactly one text spelling ([`Decimal`]'s `Display`), and
//! parsing it back gives the same pair.
//!
//! **Not in a surface payload beyond `i64`.** A `dec` is an ordinary array of two integers, so
//! a payload holding one round-trips through surface text as `[412, -1]`. The surface value
//! model's integers are `i64`, and a mantissa beyond that has no surface spelling; such a unit
//! has no surface form, as a saved query has none, and lives in CBOR.

use core::fmt;

use crate::cbor::major;
use crate::cbor::reader::Dec;
use crate::cbor::writer::Enc;
use crate::diag::Code;
use crate::surface::hjson::{HValue, Spanned};

/// The largest mantissa magnitude: the sign is carried by the CBOR major type, so the magnitude
/// is any `u64` (§5.2 rule 2).
const M_MAX: i128 = u64::MAX as i128;

/// The exponent bounds (§5.2 rule 3).
pub const E_MIN: i16 = -400;
pub const E_MAX: i16 = 400;

/// A canonical decimal, m × 10^e (RFC SMYSL-3 §5).
///
/// Every value of this type is canonical: the constructors normalise, and the decoder refuses
/// what is not already normal. `Eq` is therefore value equality, and `Ord` is **not** numeric
/// order, so it is not derived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    m: i128,
    e: i16,
}

/// Why a value is not a `dec`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecError {
    /// The text is not a decimal token.
    Syntax(String),
    /// NaN or an infinity. These are not values: a stale marker is a fact (`x.tel/stale`), not
    /// a number (§5.2 rule 5).
    NotAValue,
    /// The normalised mantissa's magnitude exceeds 2⁶⁴ − 1, or the exponent leaves [−400, 400].
    /// A change unit keeps such a token as text (`SMY-W613`).
    OutOfRange,
    /// CBOR that is not a canonical `dec` (`SMY-E602`): not a two-element array of shortest-form
    /// integers, a trailing-zero mantissa, a non-zero exponent on zero, −2⁶⁴, or bytes after it.
    NonCanonical(&'static str),
}

impl DecError {
    /// The diagnostic a decoder reports. Only a refusal of CBOR is one: a source token that is
    /// not a `dec` is a reader's to report, in its own terms.
    pub const fn code(&self) -> Option<Code> {
        match self {
            DecError::NonCanonical(_) => Some(Code::E602),
            DecError::Syntax(_) | DecError::NotAValue | DecError::OutOfRange => None,
        }
    }
}

impl fmt::Display for DecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecError::Syntax(t) => write!(f, "`{t}` is not a decimal number"),
            DecError::NotAValue => f.write_str("NaN and the infinities are not values"),
            DecError::OutOfRange => f.write_str(
                "out of dec's range: a mantissa of at most 2^64 - 1 and an exponent in [-400, 400]",
            ),
            DecError::NonCanonical(why) => write!(f, "{}: not a canonical dec: {why}", Code::E602),
        }
    }
}

impl std::error::Error for DecError {}

impl Decimal {
    /// Zero, `[0, 0]`.
    pub const ZERO: Decimal = Decimal { m: 0, e: 0 };

    /// m × 10^e, normalised: trailing zeros move from the mantissa into the exponent, and zero
    /// is `[0, 0]` whatever exponent it was given.
    pub fn new(m: i128, e: i64) -> Result<Decimal, DecError> {
        if m == 0 {
            return Ok(Decimal::ZERO);
        }
        let (mut m, mut e) = (m, e);
        while m % 10 == 0 {
            m /= 10;
            e = e.saturating_add(1);
        }
        if !(-M_MAX..=M_MAX).contains(&m) || !(i64::from(E_MIN)..=i64::from(E_MAX)).contains(&e) {
            return Err(DecError::OutOfRange);
        }
        Ok(Decimal { m, e: e as i16 })
    }

    /// The mantissa, carrying the sign.
    pub const fn mantissa(&self) -> i128 {
        self.m
    }

    /// The exponent.
    pub const fn exponent(&self) -> i16 {
        self.e
    }

    pub const fn is_zero(&self) -> bool {
        self.m == 0
    }

    /// Whether the value is an integer: every value with `e ≥ 0` is (§9.3's integer series).
    pub const fn is_integral(&self) -> bool {
        self.e >= 0
    }

    /// A typed integer, never routed through a double (§5.3).
    pub fn from_i64(n: i64) -> Decimal {
        // In range by construction: |i64| < 2⁶⁴, and normalising only raises `e` to at most 18.
        Decimal::new(i128::from(n), 0).unwrap_or(Decimal::ZERO)
    }

    /// A typed unsigned integer, never routed through a double (§5.3).
    pub fn from_u64(n: u64) -> Decimal {
        Decimal::new(i128::from(n), 0).unwrap_or(Decimal::ZERO)
    }

    /// A decimal token, read **exactly as written** (§5.3): its digits become the mantissa and
    /// its exponent the exponent, never through binary64.
    ///
    /// The grammar is a sign (`-` or `+`), digits with at most one `.` and at least one digit,
    /// and an optional exponent (`e` or `E`, a sign, digits). That is every JSON number, every
    /// Prometheus sample value that is a value, and a Zabbix value string. `NaN`, `Inf` and
    /// `+Inf` are refused as [`DecError::NotAValue`] rather than as syntax, because they are
    /// what a Prometheus stale marker or an overflow is spelled as.
    pub fn parse_token(token: &str) -> Result<Decimal, DecError> {
        let syntax = || DecError::Syntax(token.to_string());
        let lowered = token.trim_start_matches(['+', '-']).to_ascii_lowercase();
        if matches!(lowered.as_str(), "nan" | "inf" | "infinity") {
            return Err(DecError::NotAValue);
        }
        let b = token.as_bytes();
        let mut i = 0;
        let negative = match b.first() {
            Some(b'-') => {
                i = 1;
                true
            }
            Some(b'+') => {
                i = 1;
                false
            }
            _ => false,
        };
        // The significant digits, with the point's position folded into the exponent.
        let mut digits = String::new();
        let mut frac_len: i64 = 0;
        let mut seen_digit = false;
        let mut seen_point = false;
        while i < b.len() {
            match b[i] {
                c @ b'0'..=b'9' => {
                    seen_digit = true;
                    if seen_point {
                        frac_len += 1;
                    }
                    // Leading zeros carry nothing.
                    if !(digits.is_empty() && c == b'0') {
                        digits.push(c as char);
                    }
                }
                b'.' if !seen_point => seen_point = true,
                b'e' | b'E' => break,
                _ => return Err(syntax()),
            }
            i += 1;
        }
        if !seen_digit {
            return Err(syntax());
        }
        let mut exp: i64 = 0;
        if i < b.len() {
            // At an `e`.
            i += 1;
            let neg_exp = match b.get(i) {
                Some(b'-') => {
                    i += 1;
                    true
                }
                Some(b'+') => {
                    i += 1;
                    false
                }
                _ => false,
            };
            if i >= b.len() {
                return Err(syntax());
            }
            for &c in &b[i..] {
                if !c.is_ascii_digit() {
                    return Err(syntax());
                }
                // Saturating: an exponent too large to hold is out of range, unless the value
                // is zero, which any exponent leaves zero.
                exp = exp.saturating_mul(10).saturating_add(i64::from(c - b'0'));
            }
            if neg_exp {
                exp = -exp;
            }
        }
        // Zero, whatever the exponent said.
        let trimmed = digits.trim_end_matches('0');
        if trimmed.is_empty() {
            return Ok(Decimal::ZERO);
        }
        let trailing = (digits.len() - trimmed.len()) as i64;
        // 2⁶⁴ − 1 has twenty digits; anything longer is out of range before it is parsed.
        if trimmed.len() > 20 {
            return Err(DecError::OutOfRange);
        }
        let magnitude: i128 = trimmed.parse().map_err(|_| syntax())?;
        let m = if negative { -magnitude } else { magnitude };
        let e = exp.saturating_sub(frac_len).saturating_add(trailing);
        Decimal::new(m, e)
    }

    /// A binary64, as the decimal ECMA-262's `Number::toString` selects (§5.3): the fewest
    /// significant digits that round-trip, and among those the closest to the exact value.
    ///
    /// Rust's float formatting produces the shortest round-tripping digits and, among those,
    /// the closest — the same choice. `fixtures/tel/dec/` carries cases generated by a
    /// JavaScript engine's own `String(x)`, and the tests compare against them, so a
    /// disagreement is a failing test rather than an assumption.
    pub fn from_f64(x: f64) -> Result<Decimal, DecError> {
        if !x.is_finite() {
            return Err(DecError::NotAValue);
        }
        if x == 0.0 {
            // `-0` too (§5.3).
            return Ok(Decimal::ZERO);
        }
        Decimal::parse_token(&format!("{x:e}"))
    }

    /// The canonical CBOR: `[m, e]`, two shortest-form integers.
    pub fn encode(&self, e: &mut Enc) {
        e.array_head(2);
        put_int(e, self.m);
        put_int(e, i128::from(self.e));
    }

    /// [`Decimal::encode`] into bytes of its own.
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut e = Enc::new();
        self.encode(&mut e);
        e.into_bytes()
    }

    /// Read a `dec` at the decoder's position, refusing anything not canonical (§5.2).
    pub fn decode(d: &mut Dec<'_>) -> Result<Decimal, DecError> {
        let (maj, n) = d
            .head()
            .map_err(|_| DecError::NonCanonical("not deterministic CBOR"))?;
        if maj != major::ARRAY || n != 2 {
            return Err(DecError::NonCanonical("not an array of two integers"));
        }
        let m = get_int(d)?;
        let e = get_int(d)?;
        if !(-M_MAX..=M_MAX).contains(&m) {
            return Err(DecError::NonCanonical("the mantissa is -2^64"));
        }
        if !(i128::from(E_MIN)..=i128::from(E_MAX)).contains(&e) {
            return Err(DecError::NonCanonical(
                "the exponent is outside [-400, 400]",
            ));
        }
        if m == 0 && e != 0 {
            return Err(DecError::NonCanonical("zero is [0, 0]"));
        }
        if m != 0 && m % 10 == 0 {
            return Err(DecError::NonCanonical("the mantissa has a trailing zero"));
        }
        Ok(Decimal { m, e: e as i16 })
    }

    /// [`Decimal::decode`] over the whole of `bytes`: trailing bytes are refused.
    pub fn from_cbor(bytes: &[u8]) -> Result<Decimal, DecError> {
        let mut d = Dec::new(bytes);
        let v = Decimal::decode(&mut d)?;
        if !d.is_done() {
            return Err(DecError::NonCanonical("bytes after the dec"));
        }
        Ok(v)
    }
}

impl Decimal {
    /// The value as a payload value: `[m, e]`, the array of two integers it is on the wire.
    ///
    /// `None` when the mantissa is beyond `i64`, which the payload value model cannot hold; a
    /// payload carrying such a `dec` is written as CBOR directly and has no surface form. The
    /// payload encoder writes this array to exactly [`Decimal::to_cbor`]'s bytes, so a `dec`
    /// put in a payload this way and one encoded directly are the same bytes and the same uid.
    pub fn to_payload_value(&self) -> Option<HValue> {
        let m = i64::try_from(self.m).ok()?;
        let at = |v| Spanned::new(v, crate::diag::Span::new(0, 0));
        Some(HValue::Array(vec![
            at(HValue::Int(m)),
            at(HValue::Int(i64::from(self.e))),
        ]))
    }

    /// A payload value read as a `dec`, refusing what is not canonical (`SMY-E602`) rather
    /// than normalising it: the payload is inside the uid, so `[410, -1]` and `[41, 0]` are two
    /// units, and only one of them carries a `dec`.
    pub fn from_payload_value(v: &HValue) -> Result<Decimal, DecError> {
        let HValue::Array(items) = v else {
            return Err(DecError::NonCanonical("not an array of two integers"));
        };
        let [m, e] = items.as_slice() else {
            return Err(DecError::NonCanonical("not an array of two integers"));
        };
        let (HValue::Int(m), HValue::Int(e)) = (&m.value, &e.value) else {
            return Err(DecError::NonCanonical("not an array of two integers"));
        };
        let (m, e) = (i128::from(*m), *e);
        if !(i64::from(E_MIN)..=i64::from(E_MAX)).contains(&e) {
            return Err(DecError::NonCanonical(
                "the exponent is outside [-400, 400]",
            ));
        }
        if m == 0 && e != 0 {
            return Err(DecError::NonCanonical("zero is [0, 0]"));
        }
        if m != 0 && m % 10 == 0 {
            return Err(DecError::NonCanonical("the mantissa has a trailing zero"));
        }
        Ok(Decimal { m, e: e as i16 })
    }
}

/// The one text spelling (§5.3): a plain decimal, no exponent, no `+`, `-` only if negative, and
/// at least one digit before any point.
impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        if self.m < 0 {
            out.push('-');
        }
        let digits = self.m.unsigned_abs().to_string();
        if self.e >= 0 {
            out.push_str(&digits);
            if self.m != 0 {
                out.extend(core::iter::repeat_n('0', self.e as usize));
            }
        } else {
            let places = usize::from(self.e.unsigned_abs());
            if digits.len() > places {
                let (int, frac) = digits.split_at(digits.len() - places);
                out.push_str(int);
                out.push('.');
                out.push_str(frac);
            } else {
                out.push_str("0.");
                out.extend(core::iter::repeat_n('0', places - digits.len()));
                out.push_str(&digits);
            }
        }
        f.pad(&out)
    }
}

impl core::str::FromStr for Decimal {
    type Err = DecError;

    /// Any decimal token, as [`Decimal::parse_token`] reads it. The canonical spelling is one
    /// such token, so `s.parse::<Decimal>()?.to_string() == s` holds for every spelling
    /// `Display` writes.
    fn from_str(s: &str) -> Result<Decimal, DecError> {
        Decimal::parse_token(s)
    }
}

fn put_int(e: &mut Enc, v: i128) {
    if v >= 0 {
        e.head(major::UINT, v as u64);
    } else {
        e.head(major::NEGINT, (-1 - v) as u64);
    }
}

fn get_int(d: &mut Dec<'_>) -> Result<i128, DecError> {
    let (maj, arg) = d
        .head()
        .map_err(|_| DecError::NonCanonical("not a shortest-form integer"))?;
    match maj {
        major::UINT => Ok(i128::from(arg)),
        major::NEGINT => Ok(-1 - i128::from(arg)),
        _ => Err(DecError::NonCanonical("not an array of two integers")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(m: i128, e: i64) -> Decimal {
        Decimal::new(m, e).expect("in range")
    }

    #[test]
    fn one_value_has_one_pair() {
        for t in ["41", "41.0", "4.10e1", "+41", "0041.000", "410e-1"] {
            assert_eq!(Decimal::parse_token(t), Ok(d(41, 0)), "{t}");
        }
        assert_eq!(d(41, 0), Decimal { m: 41, e: 0 });
        assert_eq!(d(4100, -2), Decimal { m: 41, e: 0 });
        assert_eq!(d(0, 7), Decimal::ZERO);
        for t in ["0", "-0", "0.000", "0e999999999999999999999", "-0.0e-5"] {
            assert_eq!(Decimal::parse_token(t), Ok(Decimal::ZERO), "{t}");
        }
    }

    /// §5.3's worked spellings, and the round trip it promises.
    #[test]
    fn the_spelling_is_plain_and_round_trips() {
        let cases = [
            (d(412, -1), "41.2"),
            (d(1, 3), "1000"),
            (d(15, -10), "0.0000000015"),
            (Decimal::ZERO, "0"),
            (d(-5, -1), "-0.5"),
            (d(-123, 0), "-123"),
            (d(123456, -3), "123.456"),
            (d(u64::MAX as i128, 0), "18446744073709551615"),
        ];
        for (v, s) in cases {
            assert_eq!(v.to_string(), s);
            assert_eq!(s.parse::<Decimal>(), Ok(v), "{s}");
        }
    }

    #[test]
    fn a_token_is_read_exactly_never_through_binary64() {
        // 0.1 + 0.2 in binary64 is 0.30000000000000004; read as written it is 3 × 10⁻¹.
        assert_eq!(Decimal::parse_token("0.3"), Ok(d(3, -1)));
        // Above 2⁵³, where binary64 would round.
        assert_eq!(
            Decimal::parse_token("9007199254740993"),
            Ok(d(9_007_199_254_740_993, 0))
        );
        assert_eq!(Decimal::parse_token("0.0004"), Ok(d(4, -4)));
    }

    #[test]
    fn the_range_is_a_u64_magnitude_and_an_exponent_of_400() {
        assert!(Decimal::parse_token("18446744073709551615").is_ok());
        assert!(Decimal::parse_token("-18446744073709551615").is_ok());
        assert_eq!(
            Decimal::parse_token("18446744073709551616"),
            Err(DecError::OutOfRange)
        );
        // Trailing zeros move into the exponent, so a long number can still be in range.
        assert_eq!(
            Decimal::parse_token("18446744073709551615000"),
            Ok(d(u64::MAX as i128, 3))
        );
        assert_eq!(Decimal::parse_token("1e400"), Ok(d(1, 400)));
        assert_eq!(Decimal::parse_token("1e401"), Err(DecError::OutOfRange));
        assert_eq!(Decimal::parse_token("1e-400"), Ok(d(1, -400)));
        assert_eq!(Decimal::parse_token("1e-401"), Err(DecError::OutOfRange));
        assert_eq!(
            Decimal::parse_token("1e99999999999999999999"),
            Err(DecError::OutOfRange)
        );
    }

    #[test]
    fn nan_and_the_infinities_are_not_values() {
        for t in ["NaN", "nan", "Inf", "+Inf", "-Inf", "infinity"] {
            assert_eq!(Decimal::parse_token(t), Err(DecError::NotAValue), "{t}");
        }
        assert_eq!(Decimal::from_f64(f64::NAN), Err(DecError::NotAValue));
        assert_eq!(Decimal::from_f64(f64::INFINITY), Err(DecError::NotAValue));
    }

    #[test]
    fn what_is_not_a_token_is_syntax() {
        for t in [
            "", "-", ".", "1.2.3", "1e", "1e+", "0x10", "1_000", "12a", " 1", "1 ",
        ] {
            assert!(
                matches!(Decimal::parse_token(t), Err(DecError::Syntax(_))),
                "{t:?}"
            );
        }
        // A point with digits on one side only is a token: Prometheus and Go both read them.
        assert_eq!(Decimal::parse_token(".5"), Ok(d(5, -1)));
        assert_eq!(Decimal::parse_token("5."), Ok(d(5, 0)));
    }

    #[test]
    fn a_binary64_is_its_shortest_round_tripping_decimal() {
        assert_eq!(Decimal::from_f64(0.1), Ok(d(1, -1)));
        assert_eq!(
            Decimal::from_f64(0.1 + 0.2),
            Ok(d(30_000_000_000_000_004, -17))
        );
        assert_eq!(Decimal::from_f64(-0.0), Ok(Decimal::ZERO));
        assert_eq!(Decimal::from_f64(41.0), Ok(d(41, 0)));
        assert_eq!(Decimal::from_f64(1e21), Ok(d(1, 21)));
        assert_eq!(Decimal::from_f64(5e-324), Ok(d(5, -324)));
        assert_eq!(
            Decimal::from_f64(f64::MAX),
            Ok(d(17_976_931_348_623_157, 292))
        );
    }

    #[test]
    fn typed_integers_skip_the_double() {
        assert_eq!(
            Decimal::from_i64(i64::MIN).to_string(),
            i64::MIN.to_string()
        );
        assert_eq!(
            Decimal::from_u64(u64::MAX).to_string(),
            u64::MAX.to_string()
        );
        assert_eq!(Decimal::from_u64(1000), d(1, 3));
    }

    #[test]
    fn the_cbor_is_two_shortest_integers() {
        assert_eq!(d(412, -1).to_cbor(), [0x82, 0x19, 0x01, 0x9c, 0x20]);
        assert_eq!(Decimal::ZERO.to_cbor(), [0x82, 0x00, 0x00]);
        assert_eq!(d(-1, 0).to_cbor(), [0x82, 0x20, 0x00]);
        for v in [
            d(412, -1),
            Decimal::ZERO,
            d(u64::MAX as i128, 0),
            d(-(u64::MAX as i128), -400),
            d(7, 400),
        ] {
            assert_eq!(Decimal::from_cbor(&v.to_cbor()), Ok(v));
        }
    }

    /// The generic payload path and the direct encoding agree byte for byte, so a `dec` in a
    /// payload has one uid however it was put there.
    #[test]
    fn a_payload_dec_is_the_same_bytes_as_a_direct_one() {
        for v in [
            d(412, -1),
            Decimal::ZERO,
            d(-7, 400),
            d(i64::MAX as i128, -3),
        ] {
            let pv = v.to_payload_value().expect("within i64");
            assert_eq!(crate::surface::payload::value_bytes(&pv), v.to_cbor());
            assert_eq!(Decimal::from_payload_value(&pv), Ok(v));
        }
        assert_eq!(
            d(u64::MAX as i128, 0).to_payload_value(),
            None,
            "beyond i64"
        );
        // Not normalised on the way in: `[410, -1]` is a payload, not a `dec`.
        let at = |v| Spanned::new(v, crate::diag::Span::new(0, 0));
        let wide = HValue::Array(vec![at(HValue::Int(410)), at(HValue::Int(-1))]);
        assert_eq!(
            Decimal::from_payload_value(&wide).map_err(|e| e.code()),
            Err(Some(Code::E602))
        );
        assert!(Decimal::from_payload_value(&HValue::Str("41.2".into())).is_err());
    }

    #[test]
    fn a_non_canonical_dec_is_e602() {
        let refused: &[(&[u8], &str)] = &[
            (&[0x82, 0x18, 0x0a, 0x00], "trailing zero"), // [10, 0]
            (&[0x82, 0x00, 0x01], "zero with an exponent"), // [0, 1]
            (&[0x82, 0x18, 0x05, 0x00], "non-shortest"),  // [24-form 5, 0]
            (&[0x83, 0x01, 0x00, 0x00], "three elements"),
            (&[0x81, 0x01], "one element"),
            (
                &[0x82, 0xfa, 0x3f, 0x80, 0x00, 0x00, 0x00],
                "a float mantissa",
            ),
            (&[0x82, 0x01, 0x00, 0x00], "bytes after it"),
            (&[0x9f, 0x01, 0x00, 0xff], "indefinite length"),
            (&[0x82, 0x01, 0x19, 0x01, 0x91], "exponent 401"),
            // −2⁶⁴: major type 1 with argument 2⁶⁴ − 1.
            (
                &[
                    0x82, 0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00,
                ],
                "-2^64",
            ),
        ];
        for (bytes, why) in refused {
            let err = Decimal::from_cbor(bytes).expect_err(why);
            assert_eq!(err.code(), Some(Code::E602), "{why}: {err}");
        }
        // −(2⁶⁴ − 1) is the one below it, and is a dec.
        let min = [
            0x82, 0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xfe, 0x00,
        ];
        assert_eq!(Decimal::from_cbor(&min), Ok(d(-(u64::MAX as i128), 0)));
    }
}
