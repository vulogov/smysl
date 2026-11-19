//! Substrate exchange (RFC SMYSL-3 §19.5): samples, not chunks, under an export's `substrate/`.
//!
//! - **Day files.** Each file holds the samples of one (series, instrument, UTC day) as a CBOR
//!   sequence of §9.1 sample arrays `[ts_ms, ns_or_0, value, flags]`, in sample order. It is
//!   named by the BLAKE3 of its bytes, `<hex>.cbor`. Chunk spans divide a day, so a day is a
//!   whole number of chunks; but the file's bytes do not depend on span, encoder version or
//!   anything else local (§19.2), so two peers holding the same samples write the same file.
//! - **The listing**, `listing.cbor`, records the series (scheme, key, class), the
//!   (series, instrument, day) triples held with each file's name and sample count, and the
//!   triples the exporter knows are evicted (rule TL, §13). A day partly evicted is in both.
//! - **Import adds the samples as a set** ([`Substrate::add`]), so exchange is commutative,
//!   associative and idempotent. A sample written where a span was evicted clears its eviction
//!   record. The listing's evicted triples are information only: they evict nothing here.
//!
//! **Events** (§19.6, TL-P9) travel the same way: one file per (stream, instrument, UTC day),
//! the day's event arrays `[ts_ms, ns, body]` in event order, named by its BLAKE3, and listed
//! beside the sample days. Import adds them as a set, indexing each span under its trace with
//! the caller's tagger ([`import_tagged`]).
//!
//! The listing is
//!
//! ```text
//! [ "smysl/substrate/1",
//!   [ [series, scheme, key, class], … ],              series uid order
//!   [ [series, instrument, day ms, name, count], … ], key order
//!   [ [series, instrument, day ms], … ],              key order
//!   [ [stream, scheme, key, kind], … ],               stream uid order (TL-P9)
//!   [ [stream, instrument, day ms, name, count], … ], key order
//!   [ [stream, instrument, day ms], … ] ]             key order
//! ```
//!
//! with uids and names as 32-byte strings and `day ms` the UTC midnight that starts the day.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::Enc;
use smysl_core::{Decimal, Uid};

use crate::chunk;
use crate::event::{self, Event, HOUR_MS};
use crate::event_store::{StreamInfo, Tagger};
use crate::sample::{flags, Sample};
use crate::store::{key_inst, key_start, Backend, SeriesInfo, Substrate};
use crate::Error;

/// The directory of an export that holds the substrate.
pub const DIR: &str = "substrate";
/// The listing, inside [`DIR`].
pub const LISTING: &str = "listing.cbor";
/// The listing's tag, its first element.
pub const TAG: &str = "smysl/substrate/1";
/// A UTC day, in ms.
pub const DAY_MS: u64 = 86_400_000;

/// One held (series, instrument, day).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    pub series: Uid,
    pub inst: Uid,
    pub day_ms: u64,
    /// BLAKE3 of the day file's bytes, which is its name.
    pub name: [u8; 32],
    pub count: u64,
}

/// What an export holds and knows was evicted (§19.5).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub series: Vec<(Uid, SeriesInfo)>,
    pub held: Vec<Held>,
    pub evicted: Vec<(Uid, Uid, u64)>,
    /// Event streams (TL-P9); a [`Held`]'s `series` is the stream.
    pub streams: Vec<(Uid, StreamInfo)>,
    pub events_held: Vec<Held>,
    pub events_evicted: Vec<(Uid, Uid, u64)>,
}

/// What [`import`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Day files read.
    pub files: u64,
    /// Samples the substrate did not already hold.
    pub added: u64,
    /// Samples it held already.
    pub already: u64,
    /// Instants that now hold more than one sample (`SMY-W615`).
    pub conflicts: u64,
    /// Triples the exporter listed as evicted.
    pub evicted: u64,
    /// Event day files read (TL-P9).
    pub event_files: u64,
    /// Events the substrate did not already hold.
    pub events_added: u64,
    /// Events it held already.
    pub events_already: u64,
    /// Event days the exporter listed as evicted.
    pub events_evicted: u64,
}

/// The file name of a day file: `<hex of name>.cbor`.
pub fn file_name(name: &[u8; 32]) -> String {
    let mut s = String::with_capacity(69);
    for b in name {
        s.push_str(&format!("{b:02x}"));
    }
    s.push_str(".cbor");
    s
}

/// The day file of `samples`, which are in sample order: their §9.1 arrays, one after another.
pub fn encode_day(samples: &[Sample]) -> Vec<u8> {
    let mut e = Enc::with_capacity(samples.len() * 16);
    for s in samples {
        s.encode(&mut e);
    }
    e.into_bytes()
}

/// The samples of a day file of `day_ms`. Anything but what [`encode_day`] writes for samples
/// of that day is refused: out of order, repeated, outside the day, or not canonical.
pub fn decode_day(bytes: &[u8], day_ms: u64) -> Result<Vec<Sample>, Error> {
    let bad = |what: String| Error::Corrupt(format!("a substrate day file: {what}"));
    let mut d = Dec::new(bytes);
    let mut out: Vec<Sample> = Vec::new();
    while !d.is_done() {
        let n = d.array_head().map_err(|e| bad(e.to_string()))?;
        if n != 4 {
            return Err(bad(format!("a sample of {n} elements, not 4")));
        }
        let ts = d.uint().map_err(|e| bad(e.to_string()))?;
        let ns = d.uint().map_err(|e| bad(e.to_string()))?;
        let value = Decimal::decode(&mut d).map_err(|e| bad(e.to_string()))?;
        let fl = d.uint().map_err(|e| bad(e.to_string()))?;
        let fl = u8::try_from(fl).map_err(|_| bad(format!("flags {fl}")))?;
        let ns = u32::try_from(ns).map_err(|_| bad(format!("a remainder of {ns} ns")))?;
        let ns = if fl & flags::NS != 0 { Some(ns) } else { None };
        let s = Sample::new(ts, ns, value, fl).map_err(|e| bad(e.to_string()))?;
        if !(day_ms..day_ms.saturating_add(DAY_MS)).contains(&ts) {
            return Err(bad(format!(
                "a sample at {ts}, outside the day at {day_ms}"
            )));
        }
        if out.last().is_some_and(|p| *p >= s) {
            return Err(bad("samples not in sample order, or repeated".to_string()));
        }
        out.push(s);
    }
    if encode_day(&out) != bytes {
        return Err(bad("not in canonical form".to_string()));
    }
    Ok(out)
}

impl Listing {
    pub fn encode(&self) -> Vec<u8> {
        let mut e = Enc::new();
        e.array_head(7);
        e.text(TAG);
        e.array_head(self.series.len());
        for (uid, info) in &self.series {
            e.array_head(4);
            e.uid(uid);
            e.text(&info.scheme);
            e.text(&info.key);
            e.text(&info.class);
        }
        e.array_head(self.held.len());
        for h in &self.held {
            e.array_head(5);
            e.uid(&h.series);
            e.uid(&h.inst);
            e.uint(h.day_ms);
            e.bytes(&h.name);
            e.uint(h.count);
        }
        let triples = |e: &mut Enc, ts: &[(Uid, Uid, u64)]| {
            e.array_head(ts.len());
            for (s, i, day) in ts {
                e.array_head(3);
                e.uid(s);
                e.uid(i);
                e.uint(*day);
            }
        };
        let held = |e: &mut Enc, hs: &[Held]| {
            e.array_head(hs.len());
            for h in hs {
                e.array_head(5);
                e.uid(&h.series);
                e.uid(&h.inst);
                e.uint(h.day_ms);
                e.bytes(&h.name);
                e.uint(h.count);
            }
        };
        triples(&mut e, &self.evicted);
        e.array_head(self.streams.len());
        for (uid, info) in &self.streams {
            e.array_head(4);
            e.uid(uid);
            e.text(&info.scheme);
            e.text(&info.key);
            e.text(&info.kind);
        }
        held(&mut e, &self.events_held);
        triples(&mut e, &self.events_evicted);
        e.into_bytes()
    }

    pub fn decode(bytes: &[u8]) -> Result<Listing, Error> {
        let bad = |what: String| Error::Corrupt(format!("a substrate listing: {what}"));
        let c = |e: smysl_core::CodecError| bad(e.to_string());
        let mut d = Dec::new(bytes);
        let head = |d: &mut Dec<'_>, n: usize| -> Result<(), Error> {
            match d.array_head() {
                Ok(m) if m == n => Ok(()),
                Ok(m) => Err(bad(format!("an array of {m}, not {n}"))),
                Err(e) => Err(bad(e.to_string())),
            }
        };
        head(&mut d, 7)?;
        let tag = d.text().map_err(c)?;
        if tag != TAG {
            return Err(bad(format!("tagged `{tag}`, not `{TAG}`")));
        }
        let mut l = Listing::default();
        for _ in 0..d.array_head().map_err(c)? {
            head(&mut d, 4)?;
            let uid = d.uid().map_err(c)?;
            let (scheme, key, class) = (
                d.text().map_err(c)?.to_string(),
                d.text().map_err(c)?.to_string(),
                d.text().map_err(c)?.to_string(),
            );
            l.series.push((uid, SeriesInfo { scheme, key, class }));
        }
        let held = |d: &mut Dec<'_>| -> Result<Vec<Held>, Error> {
            let mut out = Vec::new();
            for _ in 0..d.array_head().map_err(c)? {
                head(d, 5)?;
                let (series, inst) = (d.uid().map_err(c)?, d.uid().map_err(c)?);
                let day_ms = d.uint().map_err(c)?;
                let name = <[u8; 32]>::try_from(d.bytes().map_err(c)?)
                    .map_err(|_| bad("a file name that is not 32 bytes".to_string()))?;
                let count = d.uint().map_err(c)?;
                if day_ms % DAY_MS != 0 {
                    return Err(bad(format!("a day at {day_ms}, not a UTC midnight")));
                }
                out.push(Held {
                    series,
                    inst,
                    day_ms,
                    name,
                    count,
                });
            }
            Ok(out)
        };
        let triples = |d: &mut Dec<'_>| -> Result<Vec<(Uid, Uid, u64)>, Error> {
            let mut out = Vec::new();
            for _ in 0..d.array_head().map_err(c)? {
                head(d, 3)?;
                out.push((
                    d.uid().map_err(c)?,
                    d.uid().map_err(c)?,
                    d.uint().map_err(c)?,
                ));
            }
            Ok(out)
        };
        l.held = held(&mut d)?;
        l.evicted = triples(&mut d)?;
        for _ in 0..d.array_head().map_err(c)? {
            head(&mut d, 4)?;
            let uid = d.uid().map_err(c)?;
            let (scheme, key, kind) = (
                d.text().map_err(c)?.to_string(),
                d.text().map_err(c)?.to_string(),
                d.text().map_err(c)?.to_string(),
            );
            l.streams.push((uid, StreamInfo { scheme, key, kind }));
        }
        l.events_held = held(&mut d)?;
        l.events_evicted = triples(&mut d)?;
        if !d.is_done() {
            return Err(bad("bytes after the listing".to_string()));
        }
        Ok(l)
    }
}

/// Write the substrate into `dir` (an export's [`DIR`]): one day file per held
/// (series, instrument, day), then the listing. A day file already there under its name is
/// the same bytes, and is left.
pub fn export<B: Backend>(sub: &Substrate<B>, dir: &Path) -> Result<Listing, Error> {
    let io = |p: &Path, e: std::io::Error| Error::Backend(format!("{}: {e}", p.display()));
    fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    let mut listing = Listing {
        series: sub
            .series(None)?
            .into_iter()
            .map(|(u, r)| {
                (
                    u,
                    SeriesInfo {
                        scheme: r.scheme,
                        key: r.key,
                        class: r.class,
                    },
                )
            })
            .collect(),
        ..Listing::default()
    };
    // Chunk keys sort by (series, instrument, start), and a day is a run of them.
    let mut day: Option<(Uid, Uid, u64)> = None;
    let mut samples: Vec<Sample> = Vec::new();
    let mut flush = |day: (Uid, Uid, u64), samples: &mut Vec<Sample>| -> Result<(), Error> {
        if samples.is_empty() {
            return Ok(());
        }
        let bytes = encode_day(samples);
        let name = smysl_core::hash_bytes(&bytes);
        let path = dir.join(file_name(&name));
        if !path.is_file() {
            fs::write(&path, &bytes).map_err(|e| io(&path, e))?;
        }
        listing.held.push(Held {
            series: day.0,
            inst: day.1,
            day_ms: day.2,
            name,
            count: samples.len() as u64,
        });
        samples.clear();
        Ok(())
    };
    for key in sub.backend().chunk_keys()? {
        let series = Uid::from_bytes(key[..32].try_into().expect("thirty-two bytes"));
        let this = (series, key_inst(&key), key_start(&key) / DAY_MS * DAY_MS);
        if day.is_some_and(|d| d != this) {
            flush(day.expect("some"), &mut samples)?;
        }
        day = Some(this);
        let bytes = sub
            .backend()
            .chunk(&key)?
            .ok_or_else(|| Error::Backend("a chunk listed and not there".to_string()))?;
        samples.extend(chunk::decode(&bytes)?);
    }
    if let Some(d) = day {
        flush(d, &mut samples)?;
    }
    // Events, the same way: hour keys sort by (stream, instrument, start), and a day is a run.
    listing.streams = sub
        .streams()?
        .into_iter()
        .map(|(u, r)| {
            (
                u,
                StreamInfo {
                    scheme: r.scheme,
                    key: r.key,
                    kind: r.kind,
                },
            )
        })
        .collect();
    let mut eday: Option<(Uid, Uid, u64)> = None;
    let mut events: Vec<Event> = Vec::new();
    let mut flush_events = |day: (Uid, Uid, u64), events: &mut Vec<Event>| -> Result<(), Error> {
        if events.is_empty() {
            return Ok(());
        }
        let bytes = event::encode_run(events);
        let name = smysl_core::hash_bytes(&bytes);
        let path = dir.join(file_name(&name));
        if !path.is_file() {
            fs::write(&path, &bytes).map_err(|e| io(&path, e))?;
        }
        listing.events_held.push(Held {
            series: day.0,
            inst: day.1,
            day_ms: day.2,
            name,
            count: events.len() as u64,
        });
        events.clear();
        Ok(())
    };
    for key in sub.backend().event_keys()? {
        let stream = Uid::from_bytes(key[..32].try_into().expect("thirty-two bytes"));
        let start = key_start(&key);
        let this = (stream, key_inst(&key), start / DAY_MS * DAY_MS);
        if eday.is_some_and(|d| d != this) {
            flush_events(eday.expect("some"), &mut events)?;
        }
        eday = Some(this);
        let bytes = sub
            .backend()
            .event_hour(&key)?
            .ok_or_else(|| Error::Backend("an event hour listed and not there".to_string()))?;
        events.extend(event::decode_run(&bytes, start, start + HOUR_MS)?);
    }
    if let Some(d) = eday {
        flush_events(d, &mut events)?;
    }
    let mut ev_evicted: Vec<(Uid, Uid, u64)> = sub
        .event_evictions()?
        .into_iter()
        .map(|(s, i, start, _)| (s, i, start / DAY_MS * DAY_MS))
        .collect();
    ev_evicted.dedup();
    listing.events_evicted = ev_evicted;
    let mut evicted: Vec<(Uid, Uid, u64)> = sub
        .evictions()?
        .into_iter()
        .map(|(s, i, start, _)| (s, i, start / DAY_MS * DAY_MS))
        .collect();
    evicted.dedup();
    listing.evicted = evicted;
    let path = dir.join(LISTING);
    fs::write(&path, listing.encode()).map_err(|e| io(&path, e))?;
    Ok(listing)
}

/// The samples of the day file `h` names under `dir`, checked against its name, its day and its
/// count.
fn day(dir: &Path, h: &Held) -> Result<Vec<Sample>, Error> {
    let path = dir.join(file_name(&h.name));
    let bytes = fs::read(&path).map_err(|e| Error::Backend(format!("{}: {e}", path.display())))?;
    if smysl_core::hash_bytes(&bytes) != h.name {
        return Err(Error::Corrupt(format!(
            "{}: its bytes are not the file its name says",
            path.display()
        )));
    }
    let samples = decode_day(&bytes, h.day_ms)?;
    if samples.len() as u64 != h.count {
        return Err(Error::Corrupt(format!(
            "{}: {} sample(s), and the listing says {}",
            path.display(),
            samples.len(),
            h.count
        )));
    }
    Ok(samples)
}

/// The events of the event day file `h` names under `dir`, checked as [`day`] checks samples.
fn event_day(dir: &Path, h: &Held) -> Result<Vec<Event>, Error> {
    let path = dir.join(file_name(&h.name));
    let bytes = fs::read(&path).map_err(|e| Error::Backend(format!("{}: {e}", path.display())))?;
    if smysl_core::hash_bytes(&bytes) != h.name {
        return Err(Error::Corrupt(format!(
            "{}: its bytes are not the file its name says",
            path.display()
        )));
    }
    let events = event::decode_run(&bytes, h.day_ms, h.day_ms.saturating_add(DAY_MS))?;
    if events.len() as u64 != h.count {
        return Err(Error::Corrupt(format!(
            "{}: {} event(s), and the listing says {}",
            path.display(),
            events.len(),
            h.count
        )));
    }
    Ok(events)
}

/// Check the substrate under `dir`: its listing, and every day file it names against its name,
/// its day and its count, and that every series it holds samples of is described. Holds no
/// samples, so it can run before anything is written.
pub fn verify(dir: &Path) -> Result<Listing, Error> {
    let path = dir.join(LISTING);
    let bytes = fs::read(&path).map_err(|e| Error::Backend(format!("{}: {e}", path.display())))?;
    let listing = Listing::decode(&bytes)?;
    let described: std::collections::BTreeSet<Uid> =
        listing.series.iter().map(|(u, _)| *u).collect();
    for h in &listing.held {
        if !described.contains(&h.series) {
            return Err(Error::Corrupt(format!(
                "a substrate listing: samples of series {} it does not describe",
                h.series.canonical()
            )));
        }
        day(dir, h)?;
    }
    let streams: std::collections::BTreeSet<Uid> =
        listing.streams.iter().map(|(u, _)| *u).collect();
    for h in &listing.events_held {
        if !streams.contains(&h.series) {
            return Err(Error::Corrupt(format!(
                "a substrate listing: events of stream {} it does not describe",
                h.series.canonical()
            )));
        }
        event_day(dir, h)?;
    }
    Ok(listing)
}

/// Add the substrate under `dir` to `sub`, as a set (§19.5). Nothing is added unless the whole
/// substrate verifies ([`verify`]); then one day at a time. Events are indexed under no tag:
/// [`import_tagged`] names the tagger.
pub fn import<B: Backend>(sub: &mut Substrate<B>, dir: &Path) -> Result<ImportReport, Error> {
    import_tagged(sub, dir, &|_| None)
}

/// [`import`], each event indexed under `tag`'s tag, as reading it would have.
pub fn import_tagged<B: Backend>(
    sub: &mut Substrate<B>,
    dir: &Path,
    tag: Tagger<'_>,
) -> Result<ImportReport, Error> {
    let listing = verify(dir)?;
    let info: BTreeMap<Uid, SeriesInfo> = listing.series.iter().cloned().collect();
    let mut report = ImportReport {
        evicted: listing.evicted.len() as u64,
        ..ImportReport::default()
    };
    for h in &listing.held {
        let samples = day(dir, h)?;
        let r = sub.add(&h.series, &h.inst, &info[&h.series], &samples)?;
        report.files += 1;
        report.added += r.added;
        report.already += r.already;
        report.conflicts += r.conflicts.len() as u64;
    }
    let streams: BTreeMap<Uid, StreamInfo> = listing.streams.iter().cloned().collect();
    report.events_evicted = listing.events_evicted.len() as u64;
    for h in &listing.events_held {
        let events = event_day(dir, h)?;
        let r = sub.add_events(&h.series, &h.inst, &streams[&h.series], &events, tag)?;
        report.event_files += 1;
        report.events_added += r.added;
        report.events_already += r.already;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(ts: u64, x: i64) -> Sample {
        Sample::value(ts, Decimal::from_i64(x))
    }

    #[test]
    fn day_files_are_canonical() {
        let day = 3 * DAY_MS;
        let s = [v(day, 1), v(day + 5, 2), Sample::stale(day + 9)];
        let b = encode_day(&s);
        assert_eq!(decode_day(&b, day).unwrap(), s);
        assert!(decode_day(&b, 0).is_err(), "outside the day");
        let swapped = [encode_day(&s[1..2]), encode_day(&s[0..1])].concat();
        assert!(decode_day(&swapped, day).is_err(), "out of order");
        let twice = [encode_day(&s[0..1]), encode_day(&s[0..1])].concat();
        assert!(decode_day(&twice, day).is_err(), "repeated");
        assert_eq!(decode_day(&[], day).unwrap(), vec![]);
        let l = Listing::default();
        assert_eq!(Listing::decode(&l.encode()).unwrap(), l);
    }
}
