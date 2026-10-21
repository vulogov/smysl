//! smysl library import and export (RFC SMYSL-2.7).
//!
//! Export cuts a selection of a store into files by a layout, one canonical file per layout
//! key, each holding a generated view and everything the selection needs: one inclusion rule
//! per record type (§3.6), closed to a fixpoint. The contract is design rule 8. The
//! record-set digest of what an import retains must equal the digest of what was exported
//! (P1). An import must export again (P2), and a re-export must be byte-identical (P3).
//!
//! TX-P11 step 2 builds the exporter over a store the caller holds ([`export_store`]), with
//! the `single` and `by-view` layouts and `full` closure. The index file, `verify`, `ref`
//! mode, import and the library seam arrive in later steps.
//!
//! Pure. No clock, no environment, no I/O: the same store and options give the same bytes,
//! whatever order the store's log holds its records in (rule D).

#![forbid(unsafe_code)]

pub mod export;

pub use export::{
    export_store, slug, Closure, Collection, ExportOptions, ExportedFile, Layout, LibError,
    Selection, UnknownRecords, With,
};

/// The tool id an export records itself under.
pub const TOOL: &str = concat!("smysl-lib/", env!("CARGO_PKG_VERSION"));
