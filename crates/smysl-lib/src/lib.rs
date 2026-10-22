//! smysl library import and export (RFC SMYSL-2.7).
//!
//! Export cuts a selection of a store into files by a layout, one canonical file per layout
//! key, each holding a generated view and everything the selection needs: one inclusion rule
//! per record type (§3.6), closed to a fixpoint. The contract is design rule 8. The
//! record-set digest of what an import retains must equal the digest of what was exported
//! (P1). An import must export again (P2), and a re-export must be byte-identical (P3).
//!
//! TX-P11 step 2 built the exporter over a store the caller holds ([`export_store`]), with
//! the `single` and `by-view` layouts and `full` closure. Step 3 adds `ref` closure, the
//! collection's index ([`ExportIndex`], `export-index.cbor`), and [`verify()`]: each file
//! against its index, then P1 and P3. Step 4 adds import ([`import()`], [`accept()`],
//! [`discard()`], [`status()`]) over the library seam ([`LibraryWrite`]): each record of a file
//! accepted, staged, waiting or quarantined, with one manifest per file and the import tables
//! in their CBOR fallback.
//!
//! Pure. No clock, no environment, no I/O: the same store and options give the same bytes,
//! whatever order the store's log holds its records in (rule D).

#![forbid(unsafe_code)]

pub mod access;
pub mod export;
pub mod import;
pub mod index;
pub mod scope;
pub mod tables;
pub mod verify;

pub use access::{LibraryRead, LibraryWrite, NoText, TextSource};
pub use export::{
    export_library, export_store, slug, snapshot, Closure, Collection, ExportOptions, ExportedFile,
    Format, Layout, LibError, Selection, UnknownRecords, With,
};
pub use import::{
    accept, discard, import, original_path, retained, status, FileOutcome, ImportOptions,
    ImportReport, Input,
};
pub use index::{ExportIndex, IndexError, IndexFile, Role, INDEX_FILE};
pub use tables::{Batch, State};
pub use verify::{
    p2, verify, verify_collection, verify_files, write_collection, FileCheck, Property,
    VerifyReport,
};

/// The tool id an export records itself under.
pub const TOOL: &str = concat!("smysl-lib/", env!("CARGO_PKG_VERSION"));
