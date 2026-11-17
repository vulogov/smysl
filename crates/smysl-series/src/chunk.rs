//! Chunks: one (series, instrument) stream over one epoch-aligned span (RFC SMYSL-3 §19.2).
//!
//! **Local, not an identity.** A chunk's bytes depend on the span chosen and on this encoder's
//! version, so they are never hashed into anything and never compared across peers. What is
//! compared is samples, through the window digest. The format carries a version byte so a later
//! encoder can read what an earlier one wrote.
//!
//! Layout, version 1, every integer an unsigned LEB128 varint unless said otherwise:
//!
//! | Column | Encoding |
//! |---|---|
//! | header | version `1`, sample count |
//! | timestamp ms | the first, then delta-of-delta, zig-zag |
//! | ns remainder | a byte: `0` absent from every sample, `1` then one varint per sample (0 where absent) |
//! | value | a byte: `0` the first value then deltas, zig-zag, every value an integral `dec` within `i64`; `1` each `dec` as its CBOR pair |
//! | flags | runs: a flags byte and a run length, until the count is covered |
//!
//! §19.2 also names a mantissa XOR stream for decimals whose exponents repeat. Version 1 writes
//! the plain pairs; the stream is an encoder improvement for a later version, and changes no
//! sample and no digest.

use smysl_core::cbor::reader::Dec;
use smysl_core::Decimal;

use crate::sample::{flags, Sample};
use crate::Error;

/// The chunk format this encoder writes.
pub const VERSION: u8 = 1;

/// The spans a series may be chunked by (§19.2): 1 min, 15 min, 2 h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Span {
    M1,
    M15,
    H2,
}

impl Span {
    pub const ALL: [Span; 3] = [Span::M1, Span::M15, Span::H2];

    pub const fn ms(self) -> u64 {
        match self {
            Span::M1 => 60_000,
            Span::M15 => 900_000,
            Span::H2 => 7_200_000,
        }
    }

    pub fn from_ms(ms: u64) -> Option<Span> {
        Span::ALL.into_iter().find(|s| s.ms() == ms)
    }

    /// The start of the span holding `ts_ms`: `floor(ts_ms / span) × span`.
    pub const fn start(self, ts_ms: u64) -> u64 {
        ts_ms - ts_ms % self.ms()
    }

    /// The span for a series sampled every `interval_ms`: the shortest that holds at least
    /// [`Span::TARGET`] samples, else the longest. 1 s → 1 min, 15 s → 15 min, 1 min → 2 h.
    pub fn for_interval(interval_ms: u64) -> Span {
        Span::ALL
            .into_iter()
            .find(|s| s.ms() / interval_ms.max(1) >= Span::TARGET)
            .unwrap_or(Span::H2)
    }

    /// The samples a chunk aims to hold.
    pub const TARGET: u64 = 60;

    /// The span for a series first seen with `samples` (in sample order): by the median interval
    /// between distinct instants, or 15 min when there is only one.
    pub fn choose(samples: &[Sample]) -> Span {
        let mut gaps: Vec<u64> = samples
            .windows(2)
            .map(|w| w[1].ts_ms() - w[0].ts_ms())
            .filter(|g| *g > 0)
            .collect();
        if gaps.is_empty() {
            return Span::M15;
        }
        gaps.sort_unstable();
        Span::for_interval(gaps[gaps.len() / 2])
    }
}

/// Encode `samples`, in sample order and without repeats.
pub fn encode(samples: &[Sample]) -> Vec<u8> {
    let mut out = vec![VERSION];
    put(&mut out, samples.len() as u64);

    let (mut prev, mut prev_delta) = (0u64, 0i128);
    for (i, s) in samples.iter().enumerate() {
        if i == 0 {
            put(&mut out, s.ts_ms());
        } else {
            let delta = i128::from(s.ts_ms() - prev);
            put_signed(&mut out, delta - prev_delta);
            prev_delta = delta;
        }
        prev = s.ts_ms();
    }

    if samples.iter().any(|s| s.ns().is_some()) {
        out.push(1);
        for s in samples {
            put(&mut out, u64::from(s.ns().unwrap_or(0)));
        }
    } else {
        out.push(0);
    }

    let ints: Option<Vec<i64>> = samples.iter().map(|s| as_i64(s.value_dec())).collect();
    match ints {
        Some(ints) => {
            out.push(0);
            let mut prev = 0i128;
            for v in ints {
                put_signed(&mut out, i128::from(v) - prev);
                prev = i128::from(v);
            }
        }
        None => {
            out.push(1);
            for s in samples {
                out.extend_from_slice(&s.value_dec().to_cbor());
            }
        }
    }

    let mut i = 0;
    while i < samples.len() {
        let f = samples[i].flags();
        let run = samples[i..].iter().take_while(|s| s.flags() == f).count();
        out.push(f);
        put(&mut out, run as u64);
        i += run;
    }
    out
}

/// Decode a chunk into its samples, in sample order.
pub fn decode(bytes: &[u8]) -> Result<Vec<Sample>, Error> {
    let mut r = Reader { b: bytes, at: 0 };
    let version = r.byte()?;
    if version != VERSION {
        return Err(corrupt(format!("chunk format {version}, not {VERSION}")));
    }
    let n = usize::try_from(r.uint()?).map_err(|_| corrupt("a count beyond usize"))?;
    if n > bytes.len() {
        return Err(corrupt("more samples than bytes"));
    }

    let mut ts = Vec::with_capacity(n);
    let (mut prev, mut delta) = (0i128, 0i128);
    for i in 0..n {
        let t = if i == 0 {
            i128::from(r.uint()?)
        } else {
            delta += r.signed()?;
            prev + delta
        };
        let t = u64::try_from(t).map_err(|_| corrupt("a timestamp outside u64"))?;
        if i > 0 && i128::from(t) < prev {
            return Err(corrupt("timestamps out of order"));
        }
        ts.push(t);
        prev = i128::from(t);
    }

    let ns: Option<Vec<u32>> = match r.byte()? {
        0 => None,
        1 => Some(
            (0..n)
                .map(|_| {
                    r.uint().and_then(|v| {
                        u32::try_from(v).map_err(|_| corrupt("a remainder beyond u32"))
                    })
                })
                .collect::<Result<_, _>>()?,
        ),
        m => return Err(corrupt(format!("ns column mode {m}"))),
    };

    let values: Vec<Decimal> = match r.byte()? {
        0 => {
            let mut prev = 0i128;
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                prev += r.signed()?;
                let v = i64::try_from(prev).map_err(|_| corrupt("an integer beyond i64"))?;
                out.push(Decimal::from_i64(v));
            }
            out
        }
        1 => {
            let mut d = Dec::new(&bytes[r.at..]);
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                out.push(Decimal::decode(&mut d).map_err(|e| corrupt(e.to_string()))?);
            }
            r.at += d.position();
            out
        }
        m => return Err(corrupt(format!("value column mode {m}"))),
    };

    let mut fl = Vec::with_capacity(n);
    while fl.len() < n {
        let f = r.byte()?;
        let run = usize::try_from(r.uint()?).map_err(|_| corrupt("a run beyond usize"))?;
        if run == 0 || fl.len() + run > n {
            return Err(corrupt("a flags run that does not fit"));
        }
        fl.extend(std::iter::repeat_n(f, run));
    }
    if r.at != bytes.len() {
        return Err(corrupt("bytes after the flags"));
    }

    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let has_ns = fl[i] & flags::NS != 0;
        let ns = match (&ns, has_ns) {
            (Some(col), true) => Some(col[i]),
            (Some(col), false) if col[i] != 0 => {
                return Err(corrupt("a remainder on a sample without flag 8"))
            }
            (None, true) => return Err(corrupt("flag 8 with no ns column")),
            _ => None,
        };
        let s = Sample::new(ts[i], ns, values[i], fl[i]).map_err(|e| corrupt(e.to_string()))?;
        if s.value_dec() != values[i] || s.flags() != fl[i] {
            return Err(corrupt("a sample that is not well formed"));
        }
        out.push(s);
    }
    if out.windows(2).any(|w| w[0] >= w[1]) {
        return Err(corrupt("samples out of sample order or repeated"));
    }
    Ok(out)
}

/// An integral `dec` whose value fits `i64`.
fn as_i64(d: Decimal) -> Option<i64> {
    let mut m = d.mantissa();
    let e = d.exponent();
    if e < 0 {
        return None;
    }
    for _ in 0..e {
        m = m.checked_mul(10)?;
        if m.unsigned_abs() > u128::from(u64::MAX) {
            return None;
        }
    }
    i64::try_from(m).ok()
}

fn corrupt(msg: impl Into<String>) -> Error {
    Error::Corrupt(msg.into())
}

fn put(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Zig-zag over `i128`, for differences of `u64`s and `i64`s, which need 65 bits.
fn put_signed(out: &mut Vec<u8>, v: i128) {
    let z = ((v << 1) ^ (v >> 127)) as u128;
    let mut z = z;
    while z >= 0x80 {
        out.push((z as u8) | 0x80);
        z >>= 7;
    }
    out.push(z as u8);
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> Result<u8, Error> {
        let b = *self.b.get(self.at).ok_or_else(|| corrupt("truncated"))?;
        self.at += 1;
        Ok(b)
    }

    /// At most eleven bytes, 77 bits: every value this format writes needs at most 66.
    fn varint(&mut self) -> Result<u128, Error> {
        let mut v = 0u128;
        let mut shift = 0;
        loop {
            let b = self.byte()?;
            if shift > 70 {
                return Err(corrupt("a varint too long"));
            }
            v |= u128::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    fn uint(&mut self) -> Result<u64, Error> {
        u64::try_from(self.varint()?).map_err(|_| corrupt("a varint beyond u64"))
    }

    fn signed(&mut self) -> Result<i128, Error> {
        let z = self.varint()?;
        Ok(((z >> 1) as i128) ^ -((z & 1) as i128))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse_token(s).unwrap()
    }

    fn round_trip(samples: &[Sample]) -> Vec<u8> {
        let bytes = encode(samples);
        assert_eq!(decode(&bytes).unwrap(), samples);
        bytes
    }

    #[test]
    fn integers_ride_the_delta_column_and_decimals_the_pairs() {
        let ints: Vec<Sample> = (0..60)
            .map(|i| Sample::value(1_000 + i * 15_000, Decimal::from_i64(1_000 + i as i64 * 7)))
            .collect();
        let bytes = round_trip(&ints);
        assert!(bytes.len() < 60 * 3, "{} bytes", bytes.len());

        round_trip(&[
            Sample::value(1, d("0.25")),
            Sample::value(2, d("1e300")),
            Sample::stale(3),
        ]);
        // 10^19 is integral but beyond i64: the pairs.
        round_trip(&[
            Sample::value(1, d("1e19")),
            Sample::value(2, d("-9223372036854775808")),
        ]);
        round_trip(&[
            Sample::value(0, Decimal::from_u64(u64::MAX)),
            Sample::value(u64::MAX, Decimal::from_i64(i64::MIN)),
        ]);
    }

    #[test]
    fn ns_and_flags_round_trip() {
        round_trip(&[
            Sample::value(5, d("1")),
            Sample::new(5, Some(250_000), d("2"), 0).unwrap(),
            Sample::new(6, Some(0), d("2"), flags::RESET).unwrap(),
            Sample::new(7, None, d("0"), flags::NO_VALUE).unwrap(),
            Sample::stale(9),
        ]);
        round_trip(&[]);
    }

    #[test]
    fn a_corrupt_chunk_is_refused_not_misread() {
        let good = encode(&[Sample::value(1, d("1")), Sample::value(2, d("2"))]);
        for cut in 0..good.len() {
            assert!(decode(&good[..cut]).is_err(), "cut at {cut}");
        }
        let mut extra = good.clone();
        extra.push(0);
        assert!(decode(&extra).is_err());
        let mut v2 = good;
        v2[0] = 2;
        assert!(decode(&v2).is_err());
    }

    #[test]
    fn spans_by_rate() {
        assert_eq!(Span::for_interval(1_000), Span::M1);
        assert_eq!(Span::for_interval(15_000), Span::M15);
        assert_eq!(Span::for_interval(60_000), Span::H2);
        assert_eq!(Span::for_interval(0), Span::M1);
        assert_eq!(Span::choose(&[Sample::value(1, d("1"))]), Span::M15);
        assert_eq!(Span::H2.start(7_200_001), 7_200_000);
    }
}
