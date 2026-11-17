//! The series store (RFC SMYSL-3 §19.3): chunks, the series table, and the reads over them.
//!
//! [`Substrate`] is the logic, once; a [`Backend`] holds its four tables. [`MemBackend`] keeps
//! them in `BTreeMap`s, for tests and for a run that keeps nothing; `crate::redb` (feature
//! `store-redb`) keeps them in `<library>/index/series.redb`.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::Enc;
use smysl_core::Uid;

use crate::chunk::{self, Span};
use crate::sample::{self, Sample};
use crate::Error;

/// One row of the `series` table: series uid → this.
///
/// §19.3 names (span, first ms, last ms, sample count, class). The row also keeps the series'
/// `scheme` and `key`, so that the substrate can answer "which series" on its own ([`Substrate::
/// series`]) and a `tel at` can name a series by key without the unit store. The table is
/// local, as chunks are, so the extra columns change no identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesRow {
    pub span: Span,
    pub first_ms: u64,
    pub last_ms: u64,
    pub count: u64,
    pub class: String,
    pub scheme: String,
    pub key: String,
}

/// What [`Substrate::add`] needs to know of a series it has not seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesInfo {
    pub scheme: String,
    pub key: String,
    pub class: String,
}

/// A chunk's key: (series uid, instrument uid, span start ms), as 72 bytes that sort in that
/// order.
pub type ChunkKey = [u8; 72];

pub fn chunk_key(series: &Uid, inst: &Uid, start: u64) -> ChunkKey {
    let mut k = [0u8; 72];
    k[..32].copy_from_slice(series.as_bytes());
    k[32..64].copy_from_slice(inst.as_bytes());
    k[64..].copy_from_slice(&start.to_be_bytes());
    k
}

/// The instrument of a chunk key.
pub fn key_inst(k: &ChunkKey) -> Uid {
    Uid::from_bytes(k[32..64].try_into().expect("thirty-two bytes"))
}

fn key_start(k: &ChunkKey) -> u64 {
    u64::from_be_bytes(k[64..].try_into().expect("eight bytes"))
}

/// The tables a substrate lives in.
pub trait Backend {
    /// The chunk at `key`.
    fn chunk(&self, key: &ChunkKey) -> Result<Option<Vec<u8>>, Error>;
    /// The chunks of `(series, inst)` whose start is in `[lo, hi]`, in start order.
    fn chunks(
        &self,
        series: &Uid,
        inst: &Uid,
        lo: u64,
        hi: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error>;
    /// The last chunk of `(series, inst)` whose start is below `before`.
    fn chunk_before(
        &self,
        series: &Uid,
        inst: &Uid,
        before: u64,
    ) -> Result<Option<(u64, Vec<u8>)>, Error>;
    /// The instruments the substrate holds chunks of `series` from, in uid order.
    fn instruments(&self, series: &Uid) -> Result<Vec<Uid>, Error>;
    /// The span starts of `(series, inst)` in `[lo, hi]` recorded as evicted (§13; written from
    /// TL-P8).
    fn evicted(&self, series: &Uid, inst: &Uid, lo: u64, hi: u64) -> Result<Vec<u64>, Error>;
    /// The series row.
    fn row(&self, series: &Uid) -> Result<Option<Vec<u8>>, Error>;
    /// Every series row, in uid order.
    fn rows(&self) -> Result<Vec<(Uid, Vec<u8>)>, Error>;
    /// Write chunks and rows together: all of them or none.
    fn write(
        &mut self,
        chunks: Vec<(ChunkKey, Vec<u8>)>,
        rows: Vec<(Uid, Vec<u8>)>,
    ) -> Result<(), Error>;
}

/// The tables in memory.
#[derive(Debug, Default, Clone)]
pub struct MemBackend {
    chunks: BTreeMap<ChunkKey, Vec<u8>>,
    rows: BTreeMap<Uid, Vec<u8>>,
    evicted: BTreeMap<ChunkKey, u64>,
}

impl Backend for MemBackend {
    fn chunk(&self, key: &ChunkKey) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.chunks.get(key).cloned())
    }

    fn chunks(
        &self,
        series: &Uid,
        inst: &Uid,
        lo: u64,
        hi: u64,
    ) -> Result<Vec<(u64, Vec<u8>)>, Error> {
        Ok(self
            .chunks
            .range(chunk_key(series, inst, lo)..=chunk_key(series, inst, hi))
            .map(|(k, v)| (key_start(k), v.clone()))
            .collect())
    }

    fn chunk_before(
        &self,
        series: &Uid,
        inst: &Uid,
        before: u64,
    ) -> Result<Option<(u64, Vec<u8>)>, Error> {
        Ok(self
            .chunks
            .range(chunk_key(series, inst, 0)..chunk_key(series, inst, before))
            .next_back()
            .map(|(k, v)| (key_start(k), v.clone())))
    }

    fn instruments(&self, series: &Uid) -> Result<Vec<Uid>, Error> {
        use core::ops::Bound::{Excluded, Included};
        let end = chunk_key(series, &Uid::from_bytes([0xff; 32]), u64::MAX);
        let mut out = Vec::new();
        let mut from = Included(chunk_key(series, &Uid::from_bytes([0; 32]), 0));
        // One seek per instrument, not one step per chunk.
        while let Some((k, _)) = self.chunks.range((from, Included(end))).next() {
            let inst = key_inst(k);
            out.push(inst);
            from = Excluded(chunk_key(series, &inst, u64::MAX));
        }
        Ok(out)
    }

    fn evicted(&self, series: &Uid, inst: &Uid, lo: u64, hi: u64) -> Result<Vec<u64>, Error> {
        Ok(self
            .evicted
            .range(chunk_key(series, inst, lo)..=chunk_key(series, inst, hi))
            .map(|(k, _)| key_start(k))
            .collect())
    }

    fn row(&self, series: &Uid) -> Result<Option<Vec<u8>>, Error> {
        Ok(self.rows.get(series).cloned())
    }

    fn rows(&self) -> Result<Vec<(Uid, Vec<u8>)>, Error> {
        Ok(self.rows.iter().map(|(u, r)| (*u, r.clone())).collect())
    }

    fn write(
        &mut self,
        chunks: Vec<(ChunkKey, Vec<u8>)>,
        rows: Vec<(Uid, Vec<u8>)>,
    ) -> Result<(), Error> {
        self.chunks.extend(chunks);
        self.rows.extend(rows);
        Ok(())
    }
}

/// What one [`Substrate::add`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddReport {
    /// Samples the substrate did not already hold.
    pub added: u64,
    /// Samples it already held: the add is a set union, so these change nothing.
    pub already: u64,
    /// Instants this add touched that now hold more than one sample (`SMY-W615`).
    pub conflicts: Vec<(u64, u32)>,
}

/// The samples of a window and their digest (§9.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRead {
    /// In sample order.
    pub samples: Vec<Sample>,
    pub digest: [u8; 32],
    /// Instants holding more than one sample (`SMY-W615`).
    pub conflicts: Vec<(u64, u32)>,
}

/// Which spans of a window the substrate holds, for `SMY-W605`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Span starts with a chunk.
    pub present: Vec<u64>,
    /// Span starts recorded as evicted.
    pub evicted: Vec<u64>,
}

impl Coverage {
    /// Nothing of the window is here: evidence over it is not re-derivable.
    pub fn is_absent(&self) -> bool {
        self.present.is_empty()
    }
}

/// The series substrate over one backend.
#[derive(Debug, Default, Clone)]
pub struct Substrate<B> {
    backend: B,
}

impl<B: Backend> Substrate<B> {
    pub fn new(backend: B) -> Substrate<B> {
        Substrate { backend }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Add `samples` of `(series, inst)` as a set (§19.3): each target chunk is read, merged
    /// and rewritten. A series not seen before takes `info` and a span chosen from these
    /// samples' rate; one already seen keeps its own.
    pub fn add(
        &mut self,
        series: &Uid,
        inst: &Uid,
        info: &SeriesInfo,
        samples: &[Sample],
    ) -> Result<AddReport, Error> {
        let mut report = AddReport::default();
        if samples.is_empty() {
            return Ok(report);
        }
        let incoming: BTreeSet<Sample> = samples.iter().copied().collect();
        report.already = (samples.len() - incoming.len()) as u64;
        let mut row = match self.row(series)? {
            Some(row) => row,
            None => {
                let sorted: Vec<Sample> = incoming.iter().copied().collect();
                SeriesRow {
                    span: Span::choose(&sorted),
                    first_ms: u64::MAX,
                    last_ms: 0,
                    count: 0,
                    class: info.class.clone(),
                    scheme: info.scheme.clone(),
                    key: info.key.clone(),
                }
            }
        };

        let mut by_chunk: BTreeMap<u64, Vec<Sample>> = BTreeMap::new();
        for s in incoming {
            by_chunk
                .entry(row.span.start(s.ts_ms()))
                .or_default()
                .push(s);
        }
        let mut writes = Vec::new();
        for (start, new) in by_chunk {
            let key = chunk_key(series, inst, start);
            let mut set: BTreeSet<Sample> = match self.backend.chunk(&key)? {
                Some(bytes) => chunk::decode(&bytes)?.into_iter().collect(),
                None => BTreeSet::new(),
            };
            let before = set.len();
            let touched: BTreeSet<(u64, u32)> = new.iter().map(Sample::instant).collect();
            set.extend(new.iter().copied());
            let added = (set.len() - before) as u64;
            report.added += added;
            report.already += new.len() as u64 - added;
            if added == 0 {
                continue;
            }
            let merged: Vec<Sample> = set.into_iter().collect();
            report.conflicts.extend(
                sample::conflicts(&merged)
                    .into_iter()
                    .filter(|i| touched.contains(i)),
            );
            row.first_ms = row.first_ms.min(merged[0].ts_ms());
            row.last_ms = row.last_ms.max(merged[merged.len() - 1].ts_ms());
            writes.push((key, chunk::encode(&merged)));
        }
        if writes.is_empty() {
            return Ok(report);
        }
        row.count += report.added;
        self.backend
            .write(writes, vec![(*series, encode_row(&row))])?;
        Ok(report)
    }

    /// The series row, if the substrate holds any sample of the series.
    pub fn row(&self, series: &Uid) -> Result<Option<SeriesRow>, Error> {
        self.backend
            .row(series)?
            .map(|b| decode_row(&b))
            .transpose()
    }

    /// The series the substrate holds, with their rows, in uid order; those of `scheme` only
    /// when it is given.
    pub fn series(&self, scheme: Option<&str>) -> Result<Vec<(Uid, SeriesRow)>, Error> {
        let mut out = Vec::new();
        for (uid, bytes) in self.backend.rows()? {
            let row = decode_row(&bytes)?;
            if scheme.is_none_or(|s| s == row.scheme) {
                out.push((uid, row));
            }
        }
        Ok(out)
    }

    /// The samples of `(series, inst)` with `ts_ms` in `[from, to)`, in sample order, and their
    /// digest (§19.3).
    pub fn window(
        &self,
        series: &Uid,
        inst: &Uid,
        from: u64,
        to: u64,
    ) -> Result<WindowRead, Error> {
        let mut samples = Vec::new();
        if let (Some(row), true) = (self.row(series)?, from < to) {
            let (lo, hi) = (row.span.start(from), row.span.start(to - 1));
            for (_, bytes) in self.backend.chunks(series, inst, lo, hi)? {
                samples.extend(
                    chunk::decode(&bytes)?
                        .into_iter()
                        .filter(|s| (from..to).contains(&s.ts_ms())),
                );
            }
        }
        Ok(WindowRead {
            digest: sample::digest(&samples),
            conflicts: sample::conflicts(&samples),
            samples,
        })
    }

    /// The value in force at `t_ms`: the last sample at or before it, in sample order (§19.3).
    ///
    /// One key lookup and one chunk decode, and a second of each when `t_ms` falls before the
    /// first sample of its span. What "in force" means past a stale marker is the caller's: the
    /// marker is returned as the sample it is.
    pub fn at(&self, series: &Uid, inst: &Uid, t_ms: u64) -> Result<Option<Sample>, Error> {
        let Some(row) = self.row(series)? else {
            return Ok(None);
        };
        let start = row.span.start(t_ms);
        if let Some(bytes) = self.backend.chunk(&chunk_key(series, inst, start))? {
            let found = chunk::decode(&bytes)?
                .into_iter()
                .take_while(|s| s.ts_ms() <= t_ms)
                .last();
            if found.is_some() {
                return Ok(found);
            }
        }
        match self.backend.chunk_before(series, inst, start)? {
            Some((_, bytes)) => Ok(chunk::decode(&bytes)?.pop()),
            None => Ok(None),
        }
    }

    /// The instruments `series` has samples from, in uid order.
    pub fn instruments(&self, series: &Uid) -> Result<Vec<Uid>, Error> {
        self.backend.instruments(series)
    }

    /// Which spans of `[from, to)` the substrate holds for `(series, inst)`.
    pub fn present(&self, series: &Uid, inst: &Uid, from: u64, to: u64) -> Result<Coverage, Error> {
        let Some(row) = self.row(series)? else {
            return Ok(Coverage::default());
        };
        if from >= to {
            return Ok(Coverage::default());
        }
        let (lo, hi) = (row.span.start(from), row.span.start(to - 1));
        Ok(Coverage {
            present: self
                .backend
                .chunks(series, inst, lo, hi)?
                .into_iter()
                .map(|(s, _)| s)
                .collect(),
            evicted: self.backend.evicted(series, inst, lo, hi)?,
        })
    }
}

/// `[span ms, first, last, count, class, scheme, key]`.
fn encode_row(r: &SeriesRow) -> Vec<u8> {
    let mut e = Enc::new();
    e.array_head(7);
    e.uint(r.span.ms());
    e.uint(r.first_ms);
    e.uint(r.last_ms);
    e.uint(r.count);
    e.text(&r.class);
    e.text(&r.scheme);
    e.text(&r.key);
    e.into_bytes()
}

fn decode_row(b: &[u8]) -> Result<SeriesRow, Error> {
    let bad = |what: &str| Error::Corrupt(format!("a series row: {what}"));
    let mut d = Dec::new(b);
    if d.head().map_err(|_| bad("not CBOR"))? != (smysl_core::cbor::major::ARRAY, 7) {
        return Err(bad("not an array of seven"));
    }
    let uint = |d: &mut Dec<'_>| d.uint().map_err(|_| bad("not an integer"));
    let span = Span::from_ms(uint(&mut d)?).ok_or_else(|| bad("an unknown span"))?;
    let (first_ms, last_ms, count) = (uint(&mut d)?, uint(&mut d)?, uint(&mut d)?);
    let text = |d: &mut Dec<'_>| d.text().map(str::to_owned).map_err(|_| bad("not text"));
    let (class, scheme, key) = (text(&mut d)?, text(&mut d)?, text(&mut d)?);
    if !d.is_done() {
        return Err(bad("trailing bytes"));
    }
    Ok(SeriesRow {
        span,
        first_ms,
        last_ms,
        count,
        class,
        scheme,
        key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::Decimal;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn info() -> SeriesInfo {
        SeriesInfo {
            scheme: "prom".into(),
            key: "up".into(),
            class: "gauge".into(),
        }
    }

    /// §19.5's claim at the store: the substrate is a set, so how samples arrive — split into
    /// any batches, in any order, repeated — changes no window and no digest. The span a series
    /// takes depends on its first batch, and that is the point: chunks differ, digests do not.
    #[test]
    fn arrival_order_and_batching_change_no_digest() {
        let (s, i) = (uid(1), uid(2));
        // A fixed linear congruential sequence: nothing on this path may draw entropy.
        let mut x: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move || {
            x = x.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            x >> 33
        };
        let all: Vec<Sample> = (0..300)
            .map(|_| {
                let ts = next() % 20_000_000;
                match next() % 7 {
                    0 => Sample::stale(ts),
                    1 => Sample::value(
                        ts,
                        Decimal::parse_token(&format!("{}.{}", next() % 100, next() % 1000))
                            .unwrap(),
                    ),
                    _ => Sample::value(ts, Decimal::from_i64(next() as i64 - 1_000_000)),
                }
            })
            .collect();
        let mut reference: Option<WindowRead> = None;
        for round in 0..5 {
            let mut order = all.clone();
            for k in (1..order.len()).rev() {
                order.swap(k, (next() % (k as u64 + 1)) as usize);
            }
            let mut sub = Substrate::new(MemBackend::default());
            let mut rest = order.as_slice();
            while !rest.is_empty() {
                let n = (1 + next() % 40) as usize;
                let (batch, tail) = rest.split_at(n.min(rest.len()));
                sub.add(&s, &i, &info(), batch).unwrap();
                if round % 2 == 0 {
                    sub.add(&s, &i, &info(), batch).unwrap();
                }
                rest = tail;
            }
            let w = sub.window(&s, &i, 0, 20_000_000).unwrap();
            match &reference {
                None => reference = Some(w),
                Some(r) => assert_eq!(&w, r, "round {round}"),
            }
        }
        let mut set = all;
        set.sort();
        set.dedup();
        assert_eq!(reference.unwrap().digest, sample::digest(&set));
    }

    #[test]
    fn a_row_round_trips() {
        let r = SeriesRow {
            span: Span::H2,
            first_ms: 1,
            last_ms: 2,
            count: 3,
            class: "counter".into(),
            scheme: "otel".into(),
            key: "{}".into(),
        };
        assert_eq!(decode_row(&encode_row(&r)).unwrap(), r);
        assert!(decode_row(&[0x80]).is_err());
    }
}
