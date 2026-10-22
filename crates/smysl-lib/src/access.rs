//! The seam to the library (SMYSL-2.7 §4.2): what import reads and writes, and nothing else.
//!
//! §4.2 sketches a wider pair, with shards, an object store and the substrate index behind
//! them. A library has one log until SMYSL-2.8 shards it (`Library::shards` is `["catalog"]`),
//! and the tables have their CBOR fallback ([`crate::tables`]). So the seam is the log, the
//! object store's two kinds of text, a directory for the tables and batches, and an append.
//! The traits grow as those steps land, which a trait can and a concrete type in every
//! signature cannot.
//!
//! The caller holds the library's locks, as for `Library::add`: `lib import` takes the same
//! locks as `text add`, for as long, and a method that took its own would either deadlock with
//! its caller or run unlocked.
//!
//! **Text goes through `append` like everything else** (TX-P11 step 6). A log refuses records
//! 15 and 18 (`SMY-E452`, OQ-39), so the library's `append` puts them in its object store and
//! the rest in its log. Import, staging, `accept` and promotion then need no case for text:
//! a staged batch holds its parts as it holds its units, and accepting it puts each where it
//! lives.

use crate::export::LibError;
use smysl_core::types::{PartResolver, Resolved};
use smysl_core::{PartReading, PartText, Rdid, Record, Tid};
use smysl_graph::Store;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// A part resolver `check` can hold: what `CheckOptions::parts` takes.
pub type Parts = Arc<dyn PartResolver + Send + Sync + core::panic::RefUnwindSafe>;

/// Where an export finds the text a manifest names: records 15 and 18, by tid and rdid.
pub trait TextSource {
    fn part(&self, tid: &Tid) -> Option<PartText>;
    fn reading(&self, rdid: &Rdid) -> Option<PartReading>;
}

/// No text at all: a plain store exports none.
pub struct NoText;

impl TextSource for NoText {
    fn part(&self, _: &Tid) -> Option<PartText> {
        None
    }

    fn reading(&self, _: &Rdid) -> Option<PartReading> {
        None
    }
}

/// The text a store holds among its records, which an in-memory store can and a log cannot:
/// what `verify` re-exports from (P3), since the collection it imported is such a store.
pub struct StoreText {
    parts: BTreeMap<Tid, PartText>,
    readings: BTreeMap<Rdid, PartReading>,
}

impl StoreText {
    pub fn new(store: &Store) -> StoreText {
        let mut parts = BTreeMap::new();
        let mut readings = BTreeMap::new();
        for r in store.iter() {
            match r {
                Record::PartText(p) => {
                    parts.insert(p.tid, p.clone());
                }
                Record::PartReading(x) => {
                    readings.insert(x.rdid(), x.clone());
                }
                _ => {}
            }
        }
        StoreText { parts, readings }
    }
}

impl TextSource for StoreText {
    fn part(&self, tid: &Tid) -> Option<PartText> {
        self.parts.get(tid).cloned()
    }

    fn reading(&self, rdid: &Rdid) -> Option<PartReading> {
        self.readings.get(rdid).cloned()
    }
}

/// What import and export read of a library.
pub trait LibraryRead: TextSource {
    /// The library's records, as a store.
    fn snapshot(&self) -> &Store;
    /// The library's root: `lib/` under it holds the tables, staged, waiting and quarantined
    /// records (§4.1).
    fn root(&self) -> &Path;
    /// Whether the library holds this record: in its log, or, for text, in its object store.
    fn holds(&self, r: &Record) -> bool {
        self.snapshot().contains_record(&Store::record_hash(r))
    }
    /// The library's parts, for the span checks import runs (`SMY-E404`, §3.2 step 4). `None`
    /// for a library with no object store, whose spans are then not re-verified.
    fn parts(&self) -> Option<Parts> {
        None
    }
}

/// The parts an import's check resolves against: those arriving with it, then the library's.
///
/// Only parts that hash to their tid arrive here. One that does not is quarantined on its own
/// (`SMY-E446`), and offered to the check it would be reported against the whole store.
#[derive(Debug)]
pub(crate) struct ImportParts {
    pub library: Option<Parts>,
    pub incoming: BTreeMap<Tid, PartText>,
}

impl PartResolver for ImportParts {
    fn part(&self, tid: &Tid) -> Resolved {
        match self.incoming.get(tid) {
            Some(p) => Resolved::Part(p.clone()),
            None => self
                .library
                .as_ref()
                .map_or(Resolved::Absent, |l| l.part(tid)),
        }
    }

    fn locator(
        &self,
        mid: &smysl_core::Mid,
        locator: &str,
    ) -> Option<(Tid, core::ops::Range<u64>)> {
        self.library.as_ref()?.locator(mid, locator)
    }
}

/// What import writes.
pub trait LibraryWrite: LibraryRead {
    /// Append records, all or none: text to the object store, the rest to the log.
    fn append(&mut self, records: &[Record]) -> Result<(), LibError>;
}

impl TextSource for smysl_text::Library {
    fn part(&self, tid: &Tid) -> Option<PartText> {
        self.objects().get_part(tid).ok()
    }

    fn reading(&self, rdid: &Rdid) -> Option<PartReading> {
        self.objects().get_reading(rdid).ok()
    }
}

impl LibraryRead for smysl_text::Library {
    fn snapshot(&self) -> &Store {
        self.store()
    }

    fn root(&self) -> &Path {
        smysl_text::Library::root(self)
    }

    fn holds(&self, r: &Record) -> bool {
        match r {
            Record::PartText(p) => self.objects().has_part(&p.tid),
            Record::PartReading(x) => self.objects().has_reading(&x.rdid()),
            _ => self.store().contains_record(&Store::record_hash(r)),
        }
    }

    fn parts(&self) -> Option<Parts> {
        Some(Arc::new(self.objects().clone()))
    }
}

impl LibraryWrite for smysl_text::Library {
    /// Text first, then the log. A part is verified against its tid on the way in
    /// (`SMY-E446`), and an object store is content-addressed, so a part written before a log
    /// append that fails is a part nothing names yet, which the next import of it finds held.
    fn append(&mut self, records: &[Record]) -> Result<(), LibError> {
        let refuse = |e: smysl_core::error::LibError| LibError::Library(e.to_string());
        let mut log = Vec::new();
        for r in records {
            match r {
                Record::PartText(p) => {
                    self.objects().put_part(p).map_err(refuse)?;
                }
                Record::PartReading(x) => {
                    self.objects().put_reading(x).map_err(refuse)?;
                }
                _ => log.push(r.clone()),
            }
        }
        if log.is_empty() {
            return Ok(());
        }
        self.append_records(&log).map_err(refuse)
    }
}
