//! Events (RFC SMYSL-3 §19.6, TL-P9): spans and log records, the readings that are not numbers.
//!
//! An [`Event`] is (`ts_ms`, `ns`, body) within one (stream, instrument): the body is the
//! reader's canonical record in the kernel's deterministic CBOR, and holds everything the reader
//! keeps. The substrate is a **set** of events, so equal events are one, and their digest is a
//! function of the set:
//!
//! ```text
//! digest = BLAKE3-256( "smysl/tel-events/1" ‖ 0x00 ‖ cbor_array( e₁, e₂, … ) )
//! eᵢ     = cbor_array( ts_ms, ns, body )        body as a CBOR byte string
//! ```
//!
//! Events are stored by the hour, a local layout as chunks are (§19.2): an hour is the CBOR
//! sequence of its event arrays in event order.

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::Enc;

use crate::Error;

/// The digest's domain.
pub const DIGEST_DOMAIN: &[u8] = b"smysl/tel-events/1";
/// An hour, the span events are stored by.
pub const HOUR_MS: u64 = 3_600_000;

/// One event. Event order is the derived order: `ts_ms`, then `ns`, then the body's bytes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Event {
    pub ts_ms: u64,
    /// The sub-millisecond remainder, 0–999,999.
    pub ns: u32,
    /// The reader's canonical record.
    pub body: Vec<u8>,
}

impl Event {
    /// An event; `None` when `ns` is not a sub-millisecond remainder.
    pub fn new(ts_ms: u64, ns: u32, body: Vec<u8>) -> Option<Event> {
        (ns < 1_000_000).then_some(Event { ts_ms, ns, body })
    }

    /// An event at a nanosecond instant.
    pub fn at_nanos(unix_nano: u64, body: Vec<u8>) -> Event {
        Event {
            ts_ms: unix_nano / 1_000_000,
            ns: (unix_nano % 1_000_000) as u32,
            body,
        }
    }

    /// `[ts_ms, ns, body]`, the digest's element.
    pub fn encode(&self, e: &mut Enc) {
        e.array_head(3);
        e.uint(self.ts_ms);
        e.uint(u64::from(self.ns));
        e.bytes(&self.body);
    }

    /// The start of the hour it is stored in.
    pub fn hour(&self) -> u64 {
        self.ts_ms / HOUR_MS * HOUR_MS
    }
}

/// The digest of `events`, which are in event order and without repeats (§19.6).
pub fn digest(events: &[Event]) -> [u8; 32] {
    debug_assert!(events.windows(2).all(|w| w[0] < w[1]), "a sorted set");
    let mut e = Enc::with_capacity(DIGEST_DOMAIN.len() + 1 + events.len() * 64);
    e.raw(DIGEST_DOMAIN);
    e.raw(&[0]);
    e.array_head(events.len());
    for ev in events {
        ev.encode(&mut e);
    }
    smysl_core::hash_bytes(e.as_bytes())
}

/// The bytes of a run of events in event order: their arrays, one after another. An hour's
/// stored form, and (TL-P9 step 3) an exchanged day's.
pub fn encode_run(events: &[Event]) -> Vec<u8> {
    let mut e = Enc::with_capacity(events.len() * 64);
    for ev in events {
        ev.encode(&mut e);
    }
    e.into_bytes()
}

/// The events of a run [`encode_run`] wrote, with every `ts_ms` in `[lo, hi)`. Anything else is
/// refused: out of order, repeated, outside the range, or not canonical.
pub fn decode_run(bytes: &[u8], lo: u64, hi: u64) -> Result<Vec<Event>, Error> {
    let bad = |what: String| Error::Corrupt(format!("an event run: {what}"));
    let mut d = Dec::new(bytes);
    let mut out: Vec<Event> = Vec::new();
    while !d.is_done() {
        let n = d.array_head().map_err(|e| bad(e.to_string()))?;
        if n != 3 {
            return Err(bad(format!("an event of {n} elements, not 3")));
        }
        let ts = d.uint().map_err(|e| bad(e.to_string()))?;
        let ns = d.uint().map_err(|e| bad(e.to_string()))?;
        let body = d.bytes().map_err(|e| bad(e.to_string()))?.to_vec();
        let ns = u32::try_from(ns)
            .ok()
            .filter(|n| *n < 1_000_000)
            .ok_or_else(|| bad(format!("a remainder of {ns} ns")))?;
        if !(lo..hi).contains(&ts) {
            return Err(bad(format!("an event at {ts}, outside [{lo}, {hi})")));
        }
        let ev = Event {
            ts_ms: ts,
            ns,
            body,
        };
        if out.last().is_some_and(|p| *p >= ev) {
            return Err(bad("events not in event order, or repeated".to_string()));
        }
        out.push(ev);
    }
    if encode_run(&out) != bytes {
        return Err(bad("not in canonical form".to_string()));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_is_canonical_and_the_digest_is_the_set() {
        let h = 7 * HOUR_MS;
        let a = Event::at_nanos(h * 1_000_000 + 5, b"a".to_vec());
        let b = Event::new(h, 6, b"a".to_vec()).unwrap();
        let c = Event::new(h, 6, b"b".to_vec()).unwrap();
        assert!(a < b && b < c, "instant, then body");
        let run = [a.clone(), b.clone(), c.clone()];
        let bytes = encode_run(&run);
        assert_eq!(decode_run(&bytes, h, h + HOUR_MS).unwrap(), run);
        assert!(decode_run(&bytes, h + 1, h + HOUR_MS).is_err(), "outside");
        let swapped = [encode_run(&run[1..2]), encode_run(&run[0..1])].concat();
        assert!(decode_run(&swapped, h, h + HOUR_MS).is_err());
        assert_ne!(digest(&run), digest(&run[..2]));
        assert_eq!(digest(&run), digest(&run.clone()));
        assert!(Event::new(0, 1_000_000, vec![]).is_none());
    }
}
