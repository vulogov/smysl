//! Samples, their order, and the window digest (RFC SMYSL-3 §9.1).

use core::cmp::Ordering;
use core::fmt;

use smysl_core::cbor::writer::Enc;
use smysl_core::Decimal;

/// The sample flags (§9.1): a bit set.
pub mod flags {
    /// The series went stale at this instant (Prometheus' stale NaN).
    pub const STALE: u8 = 1;
    /// A counter reset: the start time changed.
    pub const RESET: u8 = 2;
    /// No recorded value (OTLP `FLAG_NO_RECORDED_VALUE`).
    pub const NO_VALUE: u8 = 4;
    /// The sub-millisecond remainder is present.
    pub const NS: u8 = 8;
    /// Every defined bit.
    pub const ALL: u8 = STALE | RESET | NO_VALUE | NS;
}

/// The domain separator of the window digest.
pub const DIGEST_DOMAIN: &[u8] = b"smysl/tel-window/1";

/// One sample: (`ts_ms`, `ns`, value, flags) of one (series, instrument) stream (§9.1).
///
/// Every value of this type is well formed, so two equal samples are one sample:
/// - `ns` is below 1 000 000, and flag 8 is set exactly when it is present;
/// - a stale or no-recorded-value sample has value `[0, 0]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sample {
    ts_ms: u64,
    ns: Option<u32>,
    value: Decimal,
    flags: u8,
}

/// Why a tuple is not a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SampleError {
    /// The sub-millisecond remainder is 1 000 000 or more.
    Ns(u32),
    /// A flag bit outside §9.1's four.
    Flags(u8),
}

impl fmt::Display for SampleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SampleError::Ns(ns) => write!(f, "a sub-millisecond remainder of {ns} ns"),
            SampleError::Flags(b) => write!(f, "flag bits {b:#04x} outside 0x0f"),
        }
    }
}

impl std::error::Error for SampleError {}

impl Sample {
    /// A sample. Flag 8 follows `ns` whatever `flags` says, and a stale or no-recorded-value
    /// sample's value becomes `[0, 0]`: both are functions of the rest, so they are set rather
    /// than checked.
    pub fn new(
        ts_ms: u64,
        ns: Option<u32>,
        value: Decimal,
        flags: u8,
    ) -> Result<Sample, SampleError> {
        if let Some(ns) = ns {
            if ns >= 1_000_000 {
                return Err(SampleError::Ns(ns));
            }
        }
        if flags & !flags::ALL != 0 {
            return Err(SampleError::Flags(flags));
        }
        let flags = (flags & !flags::NS) | if ns.is_some() { flags::NS } else { 0 };
        let value = if flags & (flags::STALE | flags::NO_VALUE) != 0 {
            Decimal::ZERO
        } else {
            value
        };
        Ok(Sample {
            ts_ms,
            ns,
            value,
            flags,
        })
    }

    /// A plain value at a millisecond instant.
    pub fn value(ts_ms: u64, value: Decimal) -> Sample {
        Sample {
            ts_ms,
            ns: None,
            value,
            flags: 0,
        }
    }

    /// A stale marker at a millisecond instant.
    pub fn stale(ts_ms: u64) -> Sample {
        Sample {
            ts_ms,
            ns: None,
            value: Decimal::ZERO,
            flags: flags::STALE,
        }
    }

    pub const fn ts_ms(&self) -> u64 {
        self.ts_ms
    }

    pub const fn ns(&self) -> Option<u32> {
        self.ns
    }

    pub const fn value_dec(&self) -> Decimal {
        self.value
    }

    pub const fn flags(&self) -> u8 {
        self.flags
    }

    /// Whether this sample is counted by a summary (§9.3): neither stale nor without a value.
    pub const fn is_counted(&self) -> bool {
        self.flags & (flags::STALE | flags::NO_VALUE) == 0
    }

    /// The instant, `(ts_ms, ns)` with an absent `ns` as 0: two samples at one instant that
    /// differ are a data error (§9.1, `SMY-W615`).
    pub fn instant(&self) -> (u64, u32) {
        (self.ts_ms, self.ns.unwrap_or(0))
    }

    /// `[ts_ms, ns_or_0, value, flags]`, the digest's element (§9.1).
    pub fn encode(&self, e: &mut Enc) {
        e.array_head(4);
        e.uint(self.ts_ms);
        e.uint(u64::from(self.ns.unwrap_or(0)));
        self.value.encode(e);
        e.uint(u64::from(self.flags));
    }
}

/// Sample order (§9.1): `ts_ms`, then `ns` (absent as 0), then the CBOR encoding of the value,
/// then flags. The value's encoding is computed only when the instant ties.
impl Ord for Sample {
    fn cmp(&self, other: &Sample) -> Ordering {
        self.instant()
            .cmp(&other.instant())
            .then_with(|| {
                if self.value == other.value {
                    Ordering::Equal
                } else {
                    self.value.to_cbor().cmp(&other.value.to_cbor())
                }
            })
            .then_with(|| self.flags.cmp(&other.flags))
    }
}

impl PartialOrd for Sample {
    fn partial_cmp(&self, other: &Sample) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The digest of a window's samples (§9.1):
/// `BLAKE3-256("smysl/tel-window/1" ‖ 0x00 ‖ cbor_array(s₁, s₂, …))`.
///
/// `samples` must be in sample order and without repeats, as a window read gives them; the
/// digest is of the set, and a caller holding anything else sorts and dedups first.
pub fn digest(samples: &[Sample]) -> [u8; 32] {
    debug_assert!(samples.windows(2).all(|w| w[0] < w[1]), "a sorted set");
    let mut e = Enc::with_capacity(DIGEST_DOMAIN.len() + 1 + samples.len() * 16);
    e.raw(DIGEST_DOMAIN);
    e.raw(&[0]);
    e.array_head(samples.len());
    for s in samples {
        s.encode(&mut e);
    }
    smysl_core::hash_bytes(e.as_bytes())
}

/// The instants at which `samples` (in sample order) hold more than one sample: §9.1's data
/// error, reported `SMY-W615`. Both samples are kept and both enter the digest.
pub fn conflicts(samples: &[Sample]) -> Vec<(u64, u32)> {
    let mut out: Vec<(u64, u32)> = Vec::new();
    for w in samples.windows(2) {
        if w[0].instant() == w[1].instant() && out.last() != Some(&w[0].instant()) {
            out.push(w[0].instant());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Decimal {
        Decimal::parse_token(s).unwrap()
    }

    #[test]
    fn flag_8_follows_ns_and_a_stale_value_is_zero() {
        let s = Sample::new(5, Some(7), d("1.5"), 0).unwrap();
        assert_eq!(s.flags(), flags::NS);
        let s = Sample::new(5, None, d("1.5"), flags::NS | flags::STALE).unwrap();
        assert_eq!((s.flags(), s.value_dec()), (flags::STALE, Decimal::ZERO));
        assert!(!s.is_counted());
        assert_eq!(
            Sample::new(0, Some(1_000_000), d("1"), 0),
            Err(SampleError::Ns(1_000_000))
        );
        assert_eq!(
            Sample::new(0, None, d("1"), 16),
            Err(SampleError::Flags(16))
        );
    }

    #[test]
    fn sample_order_is_time_then_ns_then_value_bytes_then_flags() {
        let a = Sample::value(1, d("9"));
        let b = Sample::new(1, Some(1), d("0"), 0).unwrap();
        let c = Sample::value(2, d("0"));
        assert!(a < b && b < c);
        // At one instant: the CBOR of the value decides, so 1 (0x01) precedes -1 (0x20).
        assert!(Sample::value(3, d("1")) < Sample::value(3, d("-1")));
        assert!(Sample::value(3, d("0")) < Sample::stale(3));
    }

    #[test]
    fn the_digest_is_of_the_set_and_its_order() {
        let s = [Sample::value(1, d("41.2")), Sample::value(2, d("41.20"))];
        assert_eq!(
            digest(&s),
            digest(&[Sample::value(1, d("412e-1")), Sample::value(2, d("41.2"))]),
            "a dec is its value, not its spelling"
        );
        assert_ne!(digest(&s), digest(&s[..1]));
        assert_ne!(digest(&[]), [0; 32]);
        // The preimage, spelled out: domain, a zero byte, then the CBOR array.
        let mut pre = b"smysl/tel-window/1\0".to_vec();
        pre.extend_from_slice(&[0x81, 0x84, 0x01, 0x00, 0x82, 0x19, 0x01, 0x9c, 0x20, 0x00]);
        assert_eq!(
            digest(&s[..1]),
            smysl_core::hash_bytes(&pre),
            "[[1, 0, [412, -1], 0]]"
        );
    }

    #[test]
    fn two_values_at_one_instant_are_a_conflict() {
        let mut s = vec![
            Sample::value(1, d("1")),
            Sample::value(2, d("1")),
            Sample::value(2, d("2")),
            Sample::stale(2),
            Sample::value(3, d("1")),
        ];
        s.sort();
        assert_eq!(conflicts(&s), vec![(2, 0)]);
    }
}
