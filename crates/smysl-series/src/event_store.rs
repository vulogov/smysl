//! The event substrate (RFC SMYSL-3 §19.6, TL-P9): [`Substrate`]'s reads and writes of events.
//!
//! The tables are the samples' shapes again: event hours keyed (stream, instrument, hour start),
//! eviction records with the same key, a row per stream, and a tag index from a trace id to the
//! hours holding its spans, so that `tel trace` reads a trace without reading every stream.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::Enc;
use smysl_core::Uid;

use crate::event::{self, Event, HOUR_MS};
use crate::store::{
    chunk_key, key_inst, key_start, tag_key, Backend, ChunkKey, Coverage, EvictReport,
};
use crate::{Error, Substrate};

/// What [`Substrate::add_events`] needs to know of a stream it has not seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamInfo {
    pub scheme: String,
    pub key: String,
    /// `span`, `log` or `event` (§4.3).
    pub kind: String,
}

/// One row of the stream table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamRow {
    pub kind: String,
    pub scheme: String,
    pub key: String,
    pub first_ms: u64,
    pub last_ms: u64,
    pub count: u64,
}

/// What one [`Substrate::add_events`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventAddReport {
    /// Events the substrate did not already hold.
    pub added: u64,
    /// Events it already held: the add is a set union.
    pub already: u64,
}

/// The events of a range and their digest (§19.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsRead {
    /// In event order.
    pub events: Vec<Event>,
    pub digest: [u8; 32],
}

/// A function naming an event's tag (a span's trace id), or none.
pub type Tagger<'a> = &'a dyn Fn(&Event) -> Option<[u8; 16]>;

impl<B: Backend> Substrate<B> {
    /// Add `events` of `(stream, inst)` as a set (§19.6), indexing each under `tag`'s tag.
    pub fn add_events(
        &mut self,
        stream: &Uid,
        inst: &Uid,
        info: &StreamInfo,
        events: &[Event],
        tag: Tagger<'_>,
    ) -> Result<EventAddReport, Error> {
        let mut report = EventAddReport::default();
        if events.is_empty() {
            return Ok(report);
        }
        let incoming: BTreeSet<&Event> = events.iter().collect();
        report.already = (events.len() - incoming.len()) as u64;
        let mut row = self.stream(stream)?.unwrap_or_else(|| StreamRow {
            kind: info.kind.clone(),
            scheme: info.scheme.clone(),
            key: info.key.clone(),
            first_ms: u64::MAX,
            last_ms: 0,
            count: 0,
        });
        let mut by_hour: BTreeMap<u64, Vec<&Event>> = BTreeMap::new();
        for e in incoming {
            by_hour.entry(e.hour()).or_default().push(e);
        }
        let (mut writes, mut tags) = (Vec::new(), Vec::new());
        for (hour, new) in by_hour {
            let key = chunk_key(stream, inst, hour);
            let mut set: BTreeSet<Event> = match self.backend().event_hour(&key)? {
                Some(bytes) => event::decode_run(&bytes, hour, hour + HOUR_MS)?
                    .into_iter()
                    .collect(),
                None => BTreeSet::new(),
            };
            let before = set.len();
            let mut hour_tags = BTreeSet::new();
            for e in &new {
                if !set.contains(*e) {
                    if let Some(t) = tag(e) {
                        hour_tags.insert(t);
                    }
                    set.insert((*e).clone());
                }
            }
            let added = (set.len() - before) as u64;
            report.added += added;
            report.already += new.len() as u64 - added;
            if added == 0 {
                continue;
            }
            let merged: Vec<Event> = set.into_iter().collect();
            row.first_ms = row.first_ms.min(merged[0].ts_ms);
            row.last_ms = row.last_ms.max(merged[merged.len() - 1].ts_ms);
            tags.extend(hour_tags.iter().map(|t| tag_key(t, &key)));
            writes.push((key, event::encode_run(&merged)));
        }
        if writes.is_empty() {
            return Ok(report);
        }
        row.count += report.added;
        self.backend_mut()
            .write_events(writes, vec![(*stream, encode_stream(&row))], tags)?;
        Ok(report)
    }

    /// The stream row, if the substrate holds any event of the stream.
    pub fn stream(&self, stream: &Uid) -> Result<Option<StreamRow>, Error> {
        self.backend()
            .stream_row(stream)?
            .map(|b| decode_stream(&b))
            .transpose()
    }

    /// The streams the substrate holds, with their rows, in uid order.
    pub fn streams(&self) -> Result<Vec<(Uid, StreamRow)>, Error> {
        self.backend()
            .stream_rows()?
            .into_iter()
            .map(|(u, b)| Ok((u, decode_stream(&b)?)))
            .collect()
    }

    /// The events of `(stream, inst)` with `ts_ms` in `[from, to)`, in event order, and their
    /// digest.
    pub fn events(
        &self,
        stream: &Uid,
        inst: &Uid,
        from: u64,
        to: u64,
    ) -> Result<EventsRead, Error> {
        let mut events = Vec::new();
        if from < to {
            let (lo, hi) = (from / HOUR_MS * HOUR_MS, (to - 1) / HOUR_MS * HOUR_MS);
            for (start, bytes) in self.backend().event_hours(stream, inst, lo, hi)? {
                events.extend(
                    event::decode_run(&bytes, start, start + HOUR_MS)?
                        .into_iter()
                        .filter(|e| (from..to).contains(&e.ts_ms)),
                );
            }
        }
        Ok(EventsRead {
            digest: event::digest(&events),
            events,
        })
    }

    /// The instruments `stream` has events from, in uid order.
    pub fn event_instruments(&self, stream: &Uid) -> Result<Vec<Uid>, Error> {
        let lo = chunk_key(stream, &Uid::from_bytes([0; 32]), 0);
        let hi = chunk_key(stream, &Uid::from_bytes([0xff; 32]), u64::MAX);
        let mut out: Vec<Uid> = self
            .backend()
            .event_keys()?
            .into_iter()
            .filter(|k| (lo..=hi).contains(k))
            .map(|k| key_inst(&k))
            .collect();
        out.dedup();
        Ok(out)
    }

    /// The hours holding events tagged `tag`, as `(stream, instrument, hour start)`; an hour
    /// since evicted is still named, so a caller can tell an evicted trace from one never read.
    pub fn tagged(&self, tag: &[u8; 16]) -> Result<Vec<(Uid, Uid, u64)>, Error> {
        Ok(self
            .backend()
            .tagged(tag)?
            .into_iter()
            .map(|k| (stream_of(&k), key_inst(&k), key_start(&k)))
            .collect())
    }

    /// Which hours of `[from, to)` the substrate holds for `(stream, inst)`, for `SMY-W605`.
    pub fn events_present(
        &self,
        stream: &Uid,
        inst: &Uid,
        from: u64,
        to: u64,
    ) -> Result<Coverage, Error> {
        if from >= to {
            return Ok(Coverage::default());
        }
        let (lo, hi) = (from / HOUR_MS * HOUR_MS, (to - 1) / HOUR_MS * HOUR_MS);
        Ok(Coverage {
            present: self
                .backend()
                .event_hours(stream, inst, lo, hi)?
                .into_iter()
                .map(|(s, _)| s)
                .collect(),
            evicted: self.backend().events_evicted(stream, inst, lo, hi)?,
        })
    }

    /// Every event eviction record: `(stream, instrument, hour start, evicted at)`.
    pub fn event_evictions(&self) -> Result<Vec<(Uid, Uid, u64, u64)>, Error> {
        Ok(self
            .backend()
            .event_evictions()?
            .into_iter()
            .map(|(k, at)| (stream_of(&k), key_inst(&k), key_start(&k), at))
            .collect())
    }

    /// The event hours rule TL removes at `bound_ms` (every hour that ends at or before it),
    /// and the stream rows as they will be after; counted into `report`.
    #[allow(clippy::type_complexity)]
    pub(crate) fn plan_event_eviction(
        &self,
        bound_ms: u64,
        report: &mut EvictReport,
    ) -> Result<(Vec<ChunkKey>, Vec<(Uid, Vec<u8>)>), Error> {
        let mut gone: BTreeMap<Uid, Vec<ChunkKey>> = BTreeMap::new();
        for key in self.backend().event_keys()? {
            if key_start(&key).saturating_add(HOUR_MS) <= bound_ms {
                gone.entry(stream_of(&key)).or_default().push(key);
            }
        }
        let (mut keys, mut rows) = (Vec::new(), Vec::new());
        for (stream, hours) in gone {
            let Some(mut row) = self.stream(&stream)? else {
                continue;
            };
            let mut removed = 0u64;
            let mut last_gone = 0u64;
            for k in &hours {
                if let Some(bytes) = self.backend().event_hour(k)? {
                    let start = key_start(k);
                    removed += event::decode_run(&bytes, start, start + HOUR_MS)?.len() as u64;
                    last_gone = last_gone.max(start + HOUR_MS);
                }
            }
            row.count = row.count.saturating_sub(removed);
            row.first_ms = if row.count == 0 {
                u64::MAX
            } else {
                row.first_ms.max(last_gone)
            };
            if row.count == 0 {
                row.last_ms = 0;
            }
            report.event_hours += hours.len() as u64;
            report.events += removed;
            report.streams += 1;
            keys.extend(hours);
            rows.push((stream, encode_stream(&row)));
        }
        Ok((keys, rows))
    }
}

fn stream_of(k: &ChunkKey) -> Uid {
    Uid::from_bytes(k[..32].try_into().expect("thirty-two bytes"))
}

/// `[kind, scheme, key, first, last, count]`.
fn encode_stream(r: &StreamRow) -> Vec<u8> {
    let mut e = Enc::new();
    e.array_head(6);
    e.text(&r.kind);
    e.text(&r.scheme);
    e.text(&r.key);
    e.uint(r.first_ms);
    e.uint(r.last_ms);
    e.uint(r.count);
    e.into_bytes()
}

fn decode_stream(b: &[u8]) -> Result<StreamRow, Error> {
    let bad = |what: &str| Error::Corrupt(format!("a stream row: {what}"));
    let mut d = Dec::new(b);
    if d.array_head().map_err(|_| bad("not CBOR"))? != 6 {
        return Err(bad("not an array of six"));
    }
    let text = |d: &mut Dec<'_>| d.text().map(str::to_owned).map_err(|_| bad("not text"));
    let (kind, scheme, key) = (text(&mut d)?, text(&mut d)?, text(&mut d)?);
    let uint = |d: &mut Dec<'_>| d.uint().map_err(|_| bad("not an integer"));
    let (first_ms, last_ms, count) = (uint(&mut d)?, uint(&mut d)?, uint(&mut d)?);
    if !d.is_done() {
        return Err(bad("trailing bytes"));
    }
    Ok(StreamRow {
        kind,
        scheme,
        key,
        first_ms,
        last_ms,
        count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MemBackend;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn info() -> StreamInfo {
        StreamInfo {
            scheme: "otel".into(),
            key: "spans".into(),
            kind: "span".into(),
        }
    }

    fn ev(ts: u64, body: &[u8]) -> Event {
        Event::new(ts, 0, body.to_vec()).unwrap()
    }

    fn tag_first(e: &Event) -> Option<[u8; 16]> {
        let mut t = [0u8; 16];
        t[0] = *e.body.first()?;
        Some(t)
    }

    /// The event substrate is a set: batching, order and repeats change no read and no digest,
    /// and a tag finds every hour its events are in.
    #[test]
    fn events_are_a_set_found_by_tag_and_evicted_by_the_hour() {
        let (s, i) = (uid(1), uid(2));
        let all: Vec<Event> = (0..10)
            .map(|n| ev(n * 1_000_000, &[b'a' + (n % 2) as u8, n as u8]))
            .collect();
        let mut one = Substrate::new(MemBackend::default());
        let r = one.add_events(&s, &i, &info(), &all, &tag_first).unwrap();
        assert_eq!((r.added, r.already), (10, 0));
        let mut two = Substrate::new(MemBackend::default());
        let mut rev = all.clone();
        rev.reverse();
        two.add_events(&s, &i, &info(), &rev[..4], &tag_first)
            .unwrap();
        let r = two.add_events(&s, &i, &info(), &rev, &tag_first).unwrap();
        assert_eq!((r.added, r.already), (6, 4));
        let a = one.events(&s, &i, 0, u64::MAX).unwrap();
        assert_eq!(a, two.events(&s, &i, 0, u64::MAX).unwrap());
        assert_eq!(a.events.len(), 10);
        assert_eq!(one.stream(&s).unwrap().unwrap().count, 10);

        let mut t = [0u8; 16];
        t[0] = b'a';
        let hours: Vec<u64> = one.tagged(&t).unwrap().iter().map(|h| h.2).collect();
        assert_eq!(
            hours.len(),
            3,
            "events at 0, 2, 4, 6, 8 M ms: hours 0, 1, 2: {hours:?}"
        );

        // Evict everything before 2 h: hours 0 and 1 go, the stream row follows.
        let r = one.evict(2 * HOUR_MS, 99).unwrap();
        assert_eq!((r.event_hours, r.events, r.streams), (2, 8, 1));
        assert_eq!(one.events(&s, &i, 0, u64::MAX).unwrap().events.len(), 2);
        let c = one.events_present(&s, &i, 0, 3 * HOUR_MS).unwrap();
        assert_eq!((c.present.len(), c.evicted.len()), (1, 2));
        assert_eq!(one.tagged(&t).unwrap().len(), 3, "a tag outlives its hour");
        // Adding them back clears the eviction records.
        one.add_events(&s, &i, &info(), &all, &tag_first).unwrap();
        assert!(one.event_evictions().unwrap().is_empty());
        assert_eq!(one.events(&s, &i, 0, u64::MAX).unwrap(), a);
    }
}
