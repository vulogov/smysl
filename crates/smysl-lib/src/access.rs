//! The seam to the library (SMYSL-2.7 §4.2): what import reads and writes, and nothing else.
//!
//! §4.2 sketches a wider pair, with shards, an object store and the substrate index behind
//! them. A library has one log until SMYSL-2.8 shards it (`Library::shards` is `["catalog"]`),
//! text objects arrive with step 6, and the tables have their CBOR fallback ([`crate::tables`]).
//! So the seam is the log, a directory for the tables and batches, and an append. The traits
//! grow as those steps land, which a trait can and a concrete type in every signature cannot.
//!
//! The caller holds the library's locks, as for `Library::add`: `lib import` takes the same
//! locks as `text add`, for as long, and a method that took its own would either deadlock with
//! its caller or run unlocked.

use crate::export::LibError;
use smysl_core::Record;
use smysl_graph::Store;
use std::path::Path;

/// What import and export read of a library.
pub trait LibraryRead {
    /// The library's records, as a store.
    fn snapshot(&self) -> &Store;
    /// The library's root: `lib/` under it holds the tables, staged, waiting and quarantined
    /// records (§4.1).
    fn root(&self) -> &Path;
}

/// What import writes.
pub trait LibraryWrite: LibraryRead {
    /// Append records to the log, all or none.
    fn append(&mut self, records: &[Record]) -> Result<(), LibError>;
}

impl LibraryRead for smysl_text::Library {
    fn snapshot(&self) -> &Store {
        self.store()
    }

    fn root(&self) -> &Path {
        smysl_text::Library::root(self)
    }
}

impl LibraryWrite for smysl_text::Library {
    fn append(&mut self, records: &[Record]) -> Result<(), LibError> {
        self.append_records(records)
            .map_err(|e| LibError::Library(e.to_string()))
    }
}
