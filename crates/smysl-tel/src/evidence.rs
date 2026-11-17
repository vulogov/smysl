//! Window evidence (RFC SMYSL-3 §9, TL-P4): what a window of samples contained, as an
//! `x.tel/window` unit.
//!
//! **The unit is a function of its samples.** Its payload names the window (series,
//! instrument, `[from_ms, to_ms)`, recipe), carries the §9.1 digest of the samples in it, and
//! summarises them (§9.3). Materialising the same window over the same samples twice gives one
//! uid; over a different sample set (late data) another, and both are true (§9.2).
//!
//! **Exact means exact** (§9.3):
//! - `count` is the samples without flag 1 or 4; `min`, `max`, `first`, `last` are over them.
//! - `sum` of an integer series (every counted value has `e ≥ 0`) is the exact integer, and is
//!   omitted with `SMY-W616` when that is outside `dec`'s range. Of any other series it is the
//!   exact decimal sum rounded **once** to binary64, half to even, then spelt by §5.3.
//! - `mean` is the exact sum divided by `count`, rounded once the same way, never computed from
//!   a rounded `sum`.
//!
//! The exact sum is a big decimal here, so no order of addition can change it. Rounding a
//! decimal that may have hundreds of digits is `str::parse::<f64>`'s job, which rounds correctly
//! at any length. A quotient is not a finite decimal, so the mean is written to 800 significant
//! digits and a sticky digit: a binary64 halfway point has at most 767, so the truncated
//! quotient is on the same side of every one as the exact quotient, and the parse rounds it as
//! it would the exact value.
//!
//! **Status is rule TW's** (§9.4): a window of the substrate with exact fields only is
//! `measured`; one with any approximate field is `derived`, grounded on its series; a
//! platform's own aggregate is `cited` and has no digest.
//!
//! **No surface form.** `digest` is a 32-byte CBOR byte string (`bytes32`, §4), which the
//! surface syntax has no spelling for. A window unit therefore travels as CBOR only, as a
//! payload with a `dec` beyond `i64` already does: it is committed, exported and merged, and a
//! surface rendering omits it and says so.

use std::cmp::Ordering;

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::{
    canonical_uid, Decimal, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_series::{flags, Backend, Sample, Substrate};

use crate::scrape::numeric_cmp;

/// The recipe of an exact summary.
pub const RECIPE_EXACT: &str = "tel-window/exact/1";

/// Where a window's numbers came from (§4.1 `basis`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Basis {
    /// Computed here, from the substrate's samples.
    Substrate,
    /// A platform's own aggregate, transcribed (a Zabbix trend).
    Platform,
}

impl Basis {
    pub fn as_str(self) -> &'static str {
        match self {
            Basis::Substrate => "substrate",
            Basis::Platform => "platform",
        }
    }
}

/// §9.3's exact summary of a window's samples.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// Samples without flag 1 (stale) or 4 (no value).
    pub count: u64,
    pub min: Option<Decimal>,
    pub max: Option<Decimal>,
    pub first: Option<Decimal>,
    pub last: Option<Decimal>,
    pub sum: Option<Decimal>,
    pub mean: Option<Decimal>,
    /// The exact integer sum was outside `dec`'s range, so `sum` is absent (`SMY-W616`).
    pub sum_out_of_range: bool,
}

/// Whether a sample enters a summary.
pub fn counted(s: &Sample) -> bool {
    s.flags() & (flags::STALE | flags::NO_VALUE) == 0
}

/// Summarise `samples`, which are in sample order (as a window read returns them).
pub fn summarize(samples: &[Sample]) -> Summary {
    let values: Vec<Decimal> = samples
        .iter()
        .filter(|s| counted(s))
        .map(Sample::value_dec)
        .collect();
    let mut out = Summary {
        count: values.len() as u64,
        ..Summary::default()
    };
    let (Some(first), Some(last)) = (values.first(), values.last()) else {
        return out;
    };
    out.first = Some(*first);
    out.last = Some(*last);
    out.min = values.iter().copied().min_by(numeric_cmp);
    out.max = values.iter().copied().max_by(numeric_cmp);

    let exact = BigDec::sum(&values);
    let integer = values.iter().all(|v| v.exponent() >= 0);
    if integer {
        match exact.to_dec() {
            Some(d) => out.sum = Some(d),
            None => out.sum_out_of_range = true,
        }
    } else {
        out.sum = Decimal::from_f64(exact.to_f64()).ok();
        out.sum_out_of_range = out.sum.is_none();
    }
    out.mean = Decimal::from_f64(exact.div_to_f64(out.count)).ok();
    out
}

// ---------------------------------------------------------------------------------------------
// The exact sum
// ---------------------------------------------------------------------------------------------

/// A signed decimal `± digits × 10^exp`, digits least significant first.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BigDec {
    negative: bool,
    digits: Vec<u8>,
    exp: i32,
}

impl BigDec {
    /// The exact sum of `values`.
    fn sum(values: &[Decimal]) -> BigDec {
        let exp = values
            .iter()
            .map(|v| i32::from(v.exponent()))
            .min()
            .unwrap_or(0);
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
    fn to_dec(&self) -> Option<Decimal> {
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
    fn to_f64(&self) -> f64 {
        let sign = if self.negative { "-" } else { "" };
        format!("{sign}{}e{}", self.text(), self.exp)
            .parse()
            .unwrap_or(f64::NAN)
    }

    /// `self / n`, rounded once to binary64, half to even: the quotient to 800 significant
    /// digits and a sticky digit (see the module comment). `NaN` for `n = 0`.
    fn div_to_f64(&self, n: u64) -> f64 {
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

// ---------------------------------------------------------------------------------------------
// The unit
// ---------------------------------------------------------------------------------------------

/// Everything an `x.tel/window` unit says (§4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Window {
    pub series: Uid,
    /// The series' scheme and key, for the unit's source reference.
    pub scheme: String,
    pub key: String,
    pub instrument: Uid,
    pub from_ms: u64,
    pub to_ms: u64,
    pub recipe: String,
    pub basis: Basis,
    /// Required with `basis: substrate`, absent with `platform`.
    pub digest: Option<[u8; 32]>,
    pub summary: Summary,
    /// A sketch's quantile ranks, values and relative accuracy. Any of them makes the window
    /// approximate.
    pub sketch: Option<Sketched>,
}

/// The approximate fields of a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sketched {
    pub qs: Vec<Decimal>,
    pub q: Vec<Decimal>,
    pub alpha: Decimal,
}

impl Window {
    /// Rule TW's status (§9.4).
    pub fn status(&self) -> Status {
        match (self.basis, &self.sketch) {
            (Basis::Platform, _) => Status::Cited,
            (Basis::Substrate, None) => Status::Measured,
            (Basis::Substrate, Some(_)) => Status::Derived,
        }
    }

    /// The window key (§9.2): what several windows over one question share.
    pub fn window_key(&self) -> (Uid, Uid, u64, u64, &str) {
        (
            self.series,
            self.instrument,
            self.from_ms,
            self.to_ms,
            &self.recipe,
        )
    }

    /// The canonical payload: a CBOR map with keys in the kernel's order, `dec` fields as
    /// their `[m, e]` pairs and `digest` as a byte string.
    pub fn payload(&self) -> Vec<u8> {
        let text = |s: &str| {
            let mut e = Enc::new();
            e.text(s);
            e.into_bytes()
        };
        let uint = |n: u64| {
            let mut e = Enc::new();
            e.uint(n);
            e.into_bytes()
        };
        let dec = |d: &Decimal| d.to_cbor();
        let mut entries: Vec<(&str, Vec<u8>)> = vec![
            ("tel:series", text(&self.series.canonical())),
            ("tel:instrument", text(&self.instrument.canonical())),
            ("tel:from_ms", uint(self.from_ms)),
            ("tel:to_ms", uint(self.to_ms)),
            ("tel:recipe", text(&self.recipe)),
            ("tel:basis", text(self.basis.as_str())),
            ("tel:count", uint(self.summary.count)),
        ];
        if let Some(d) = &self.digest {
            let mut e = Enc::new();
            e.bytes(d);
            entries.push(("tel:digest", e.into_bytes()));
        }
        let s = &self.summary;
        for (k, v) in [
            ("tel:min", &s.min),
            ("tel:max", &s.max),
            ("tel:sum", &s.sum),
            ("tel:mean", &s.mean),
            ("tel:first", &s.first),
            ("tel:last", &s.last),
        ] {
            if let Some(v) = v {
                entries.push((k, dec(v)));
            }
        }
        if let Some(sk) = &self.sketch {
            let list = |xs: &[Decimal]| {
                let mut e = Enc::new();
                e.head(major::ARRAY, xs.len() as u64);
                for x in xs {
                    e.raw(&x.to_cbor());
                }
                e.into_bytes()
            };
            entries.push(("tel:qs", list(&sk.qs)));
            entries.push(("tel:q", list(&sk.q)));
            entries.push(("tel:alpha", dec(&sk.alpha)));
        }
        let mut keyed: Vec<(Vec<u8>, Vec<u8>)> =
            entries.into_iter().map(|(k, v)| (text(k), v)).collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0));
        let mut e = Enc::new();
        e.head(major::MAP, keyed.len() as u64);
        for (k, v) in keyed {
            e.raw(&k);
            e.raw(&v);
        }
        e.into_bytes()
    }

    /// The `x.tel/window` unit. Its source is the series, observed at the window's end (§4.1);
    /// a `derived` window is grounded on its series.
    pub fn unit(&self) -> UnitCore {
        let status = self.status();
        let grounds = if status == Status::Derived {
            vec![self.series]
        } else {
            Vec::new()
        };
        UnitCoreBuilder::new(
            SchemaId::Extension(tel::WINDOW.to_string()),
            tel::gist_of(&format!(
                "window {} [{}, {}) {}",
                self.key, self.from_ms, self.to_ms, self.recipe
            )),
            status,
        )
        .source(
            SourceRef::new(
                SourceKind::Metric,
                format!("{}{}:{}", tel::SERIES_REF, self.scheme, self.key),
            )
            .observed_at(self.to_ms),
        )
        .grounds(grounds)
        .payload(self.payload())
        .build()
        .expect("a window unit is well formed by construction")
    }
}

/// The preferred window of several over one key (§9.2): the largest `count`, then the
/// lexicographically smallest digest. A function of the set, not of arrival.
pub fn preferred(windows: &[Window]) -> Option<&Window> {
    windows.iter().min_by(|a, b| {
        b.summary
            .count
            .cmp(&a.summary.count)
            .then_with(|| a.digest.cmp(&b.digest))
    })
}

/// Whether the windows of one key disagree (`SMY-W614`): more than one digest among them.
pub fn disagree(windows: &[Window]) -> bool {
    windows
        .iter()
        .filter_map(|w| w.digest)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        > 1
}

/// What materialising a window found.
#[derive(Debug, Clone)]
pub struct Materialised {
    pub window: Window,
    pub unit: UnitCore,
    /// Instants holding more than one sample (`SMY-W615`): both are in the digest.
    pub conflicts: Vec<(u64, u32)>,
}

/// Why a window could not be materialised.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceError {
    /// The substrate has no sample of this series at all.
    NoSeries(Uid),
    /// `from_ms ≥ to_ms`.
    Empty,
    Series(smysl_series::Error),
}

impl core::fmt::Display for EvidenceError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            EvidenceError::NoSeries(u) => {
                write!(
                    f,
                    "the substrate holds no sample of series {}",
                    u.canonical()
                )
            }
            EvidenceError::Empty => f.write_str("a window is [from, to) with from < to"),
            EvidenceError::Series(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for EvidenceError {}

impl From<smysl_series::Error> for EvidenceError {
    fn from(e: smysl_series::Error) -> EvidenceError {
        EvidenceError::Series(e)
    }
}

/// Materialise the exact window `[from_ms, to_ms)` of `(series, instrument)` from `substrate`.
pub fn materialise<B: Backend>(
    substrate: &Substrate<B>,
    series: &Uid,
    instrument: &Uid,
    from_ms: u64,
    to_ms: u64,
) -> Result<Materialised, EvidenceError> {
    if from_ms >= to_ms {
        return Err(EvidenceError::Empty);
    }
    let row = substrate
        .row(series)?
        .ok_or(EvidenceError::NoSeries(*series))?;
    let read = substrate.window(series, instrument, from_ms, to_ms)?;
    let window = Window {
        series: *series,
        scheme: row.scheme,
        key: row.key,
        instrument: *instrument,
        from_ms,
        to_ms,
        recipe: RECIPE_EXACT.to_string(),
        basis: Basis::Substrate,
        digest: Some(read.digest),
        summary: summarize(&read.samples),
        sketch: None,
    };
    let unit = window.unit();
    Ok(Materialised {
        window,
        unit,
        conflicts: read.conflicts,
    })
}

/// The uid a window unit has, for callers that only need to compare.
pub fn window_uid(w: &Window) -> Uid {
    canonical_uid(&w.unit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_series::{MemBackend, SeriesInfo};

    fn d(s: &str) -> Decimal {
        Decimal::parse_token(s).unwrap()
    }

    fn v(ts: u64, s: &str) -> Sample {
        Sample::new(ts, None, d(s), 0).unwrap()
    }

    #[test]
    fn an_integer_series_sums_exactly_and_says_when_it_cannot() {
        let s = summarize(&[v(1, "5"), v(2, "1e3"), v(3, "-2"), Sample::stale(4)]);
        assert_eq!(s.count, 3);
        assert_eq!(s.sum, Some(d("1003")));
        assert_eq!(s.min, Some(d("-2")));
        assert_eq!(s.max, Some(d("1e3")));
        assert_eq!((s.first, s.last), (Some(d("5")), Some(d("-2"))));
        assert_eq!(s.mean, Some(d("334.3333333333333")));

        let big = d("18446744073709551615");
        let max = "18446744073709551615";
        // Twice it ends in a zero, so it is [3689348814741910323, 1]: a dec.
        let s = summarize(&[v(1, max), v(2, max)]);
        assert_eq!(s.sum, Some(d("36893488147419103230")));
        // Three times is not.
        let s = summarize(&[v(1, max), v(2, max), v(3, max)]);
        assert!(
            s.sum.is_none() && s.sum_out_of_range,
            "3 (2^64 - 1) is not a dec"
        );
        assert_eq!(s.max, Some(big));
        // Its mean is, rounded through binary64.
        assert_eq!(s.mean, Some(d("18446744073709552000")));
    }

    #[test]
    fn a_decimal_sum_is_exact_then_rounded_once() {
        // 0.1 + 0.2 as decimals is 0.3, not binary64's 0.30000000000000004.
        let s = summarize(&[v(1, "0.1"), v(2, "0.2")]);
        assert_eq!(s.sum, Some(d("0.3")));
        assert_eq!(s.mean, Some(d("0.15")));
        // Cancellation that a running binary64 sum gets wrong.
        let s = summarize(&[v(1, "1e300"), v(2, "1.5"), v(3, "-1e300")]);
        assert_eq!(s.sum, Some(d("1.5")));
        assert_eq!(s.mean, Some(d("0.5")));
    }

    #[test]
    fn the_sum_does_not_depend_on_order() {
        let a = [v(1, "0.1"), v(2, "1e20"), v(3, "-1e20"), v(4, "3.333")];
        let mut b = a;
        b.reverse();
        assert_eq!(summarize(&a).sum, summarize(&b).sum);
        assert_eq!(summarize(&a).mean, summarize(&b).mean);
    }

    #[test]
    fn a_mean_is_rounded_from_the_exact_quotient() {
        // 1/3, 2/3: the nearest doubles, as JavaScript writes them.
        assert_eq!(
            summarize(&[v(1, "1"), v(2, "0"), v(3, "0")]).mean,
            Some(d("0.3333333333333333"))
        );
        assert_eq!(
            summarize(&[v(1, "1"), v(2, "1"), v(3, "0")]).mean,
            Some(d("0.6666666666666666"))
        );
        // Exactly halfway between two doubles: ties to even. 2^53 + 1 is between 2^53 and
        // 2^53 + 2, and the even one is 2^53.
        let s = summarize(&[v(1, "18014398509481986"), v(2, "0")]);
        assert_eq!(s.mean, Some(d("9007199254740992")));
        // Just above halfway: up.
        let s = summarize(&[v(1, "18014398509481987"), v(2, "0")]);
        assert_eq!(s.mean, Some(d("9007199254740994")));
    }

    #[test]
    fn nothing_counted_is_a_count_of_zero_and_nothing_else() {
        let s = summarize(&[Sample::stale(1)]);
        assert_eq!(s, Summary::default());
    }

    #[test]
    fn a_window_is_one_uid_per_sample_set_and_measured() {
        let mut sub = Substrate::new(MemBackend::default());
        let (series, inst) = (Uid::from_bytes([1; 32]), Uid::from_bytes([2; 32]));
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: "up".into(),
            class: "gauge".into(),
        };
        sub.add(&series, &inst, &info, &[v(1_000, "1"), v(2_000, "0.5")])
            .unwrap();
        let a = materialise(&sub, &series, &inst, 0, 10_000).unwrap();
        let b = materialise(&sub, &series, &inst, 0, 10_000).unwrap();
        assert_eq!(canonical_uid(&a.unit), canonical_uid(&b.unit));
        assert_eq!(a.unit.status, Status::Measured);
        assert!(a.unit.grounds.is_empty());
        // Late data: another true window, not a replacement.
        sub.add(&series, &inst, &info, &[v(3_000, "2")]).unwrap();
        let c = materialise(&sub, &series, &inst, 0, 10_000).unwrap();
        assert_ne!(canonical_uid(&a.unit), canonical_uid(&c.unit));
        let both = [a.window.clone(), c.window.clone()];
        assert!(disagree(&both));
        assert_eq!(preferred(&both).unwrap().summary.count, 3);
        // The preference is a function of the set.
        let rev = [c.window.clone(), a.window.clone()];
        assert_eq!(preferred(&rev), preferred(&both));

        assert_eq!(
            materialise(&sub, &series, &inst, 5, 5).unwrap_err(),
            EvidenceError::Empty
        );
        assert!(matches!(
            materialise(&sub, &Uid::from_bytes([9; 32]), &inst, 0, 1),
            Err(EvidenceError::NoSeries(_))
        ));
    }

    #[test]
    fn the_payload_is_canonical_cbor_with_a_byte_string_digest() {
        let w = Window {
            series: Uid::from_bytes([1; 32]),
            scheme: "prom".into(),
            key: "up".into(),
            instrument: Uid::from_bytes([2; 32]),
            from_ms: 0,
            to_ms: 10,
            recipe: RECIPE_EXACT.into(),
            basis: Basis::Substrate,
            digest: Some([7; 32]),
            summary: summarize(&[v(1, "41.0")]),
            sketch: None,
        };
        let p = w.payload();
        // A map, keys in length-then-bytewise order: the shortest key is `tel:max`/`tel:min`.
        assert_eq!(p[0] >> 5, major::MAP);
        let digest_at = p.windows(34).position(|x| x[0] == 0x58 && x[1] == 32);
        assert!(digest_at.is_some(), "a 32-byte byte string");
        // 41.0 is [41, 0].
        assert!(p.windows(3).any(|x| x == [0x82, 0x18, 41]));
    }
}
