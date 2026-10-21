//! Export: a selection of a store, cut by a layout into canonical files (SMYSL-2.7 §3.6–§3.8).
//!
//! [`export_store`] takes a store the caller holds and the units to export, partitions those
//! units by the layout key, closes each part to a fixpoint, selects records by one rule per
//! type, and writes each file in one canonical order. The bytes are a
//! function of the store's records and the options: no clock, no log order, no walk order.

mod layout;
mod select;
mod write;

pub use layout::slug;

use select::{Picked, Source};
use smysl_core::{Code, Diagnostic, Record, Report, Uid, ViewId};
use smysl_graph::{record_set_digest, EdgeSet, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// How the selection is cut into files (§3.4).
///
/// `single` and `by-view` in TX-P11 step 2. `by-expression`, `by-collection`, `by-time` and
/// `by-query` need manifests, datings and `sq`, and arrive with the steps that bring those.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Layout {
    /// One file, key `all`.
    #[default]
    Single,
    /// One file per corpus view, keyed by its id with `/` as `-`; units in no view go to
    /// `unkeyed`. A unit in several views goes to the least view id.
    ByView,
}

impl Layout {
    /// The layout's name, as an index records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Layout::Single => "single",
            Layout::ByView => "by-view",
        }
    }
}

/// What a file holds of what its selection needs (§3.7).
///
/// `full` in step 2. `ref`, which writes each record once and lists the files a file
/// requires, arrives with the index in step 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Closure {
    /// Every file holds everything selected for its key.
    #[default]
    Full,
}

/// What happens to records this build cannot interpret (row "other", `SMY-W425`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum UnknownRecords {
    /// Written to `unattributed.cbor`, verbatim.
    #[default]
    Include,
    /// Left out, and still counted.
    Drop,
}

/// The optional record classes (`--with-*`).
///
/// Corpus views and pack infos in step 2. Text, provenance, staged and quarantined records
/// arrive with the steps that hold them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct With {
    /// Corpus views whose roots are all in a file, other than an earlier export's (row 5).
    pub views: bool,
    /// Pack infos whose named units are all in a file (row 7).
    pub packs: bool,
}

impl With {
    /// None of the optional classes.
    pub fn new() -> With {
        With::default()
    }

    pub fn views(mut self, on: bool) -> With {
        self.views = on;
        self
    }

    pub fn packs(mut self, on: bool) -> With {
        self.packs = on;
        self
    }
}

/// What to export, and how.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ExportOptions {
    pub layout: Layout,
    pub closure: Closure,
    pub with: With,
    /// Edges the unit closure follows beside `deps` and `grounds`, which it always follows
    /// (rule L). `None` follows every edge.
    pub edges: Option<EdgeSet>,
    pub unknown: UnknownRecords,
}

impl ExportOptions {
    /// The defaults: `single`, `full`, no optional classes, every edge, unknown records kept.
    pub fn new() -> ExportOptions {
        ExportOptions::default()
    }

    pub fn layout(mut self, layout: Layout) -> ExportOptions {
        self.layout = layout;
        self
    }

    pub fn with(mut self, with: With) -> ExportOptions {
        self.with = with;
        self
    }

    pub fn edges(mut self, edges: EdgeSet) -> ExportOptions {
        self.edges = Some(edges);
        self
    }

    pub fn unknown(mut self, unknown: UnknownRecords) -> ExportOptions {
        self.unknown = unknown;
        self
    }
}

/// The units an export selects (`Sel`, §3.6), before the layout partitions them.
///
/// What an `sq` scope returns, once TX-P11 step 8 connects one. Until then a caller names the
/// units, or takes them all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    units: BTreeSet<Uid>,
}

impl Selection {
    /// Every unit the store holds.
    pub fn all(store: &Store) -> Selection {
        Selection {
            units: store.units().map(|(u, _)| *u).collect(),
        }
    }

    /// These units.
    pub fn of(units: impl IntoIterator<Item = Uid>) -> Selection {
        Selection {
            units: units.into_iter().collect(),
        }
    }

    pub fn units(&self) -> &BTreeSet<Uid> {
        &self.units
    }

    pub fn len(&self) -> usize {
        self.units.len()
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// One file of a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportedFile {
    /// The file name: `<slug>.cbor`, or `unattributed.cbor`.
    pub path: String,
    /// The concatenated canonical CBOR of its records, view first.
    pub bytes: Vec<u8>,
    /// How many records it holds, its view included.
    pub records: usize,
    /// The record-set digest of what it holds.
    pub rsd: [u8; 32],
    /// The generated view's id; `None` for `unattributed.cbor`.
    pub view: Option<ViewId>,
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Collection {
    /// The files, sorted by path.
    pub files: Vec<ExportedFile>,
    /// The record-set digest of everything written, every file's records together: `rsd(X)`,
    /// what P1 compares an import's retained records against.
    pub rsd: [u8; 32],
    /// The duplication factor's two terms (§3.7): the records selected summed over files, and
    /// the records selected counted once. Generated views are in neither.
    pub duplication: (usize, usize),
    /// What the export counted: `SMY-W425`, `W490`, `W495`, `W497`.
    pub report: Report,
}

impl Collection {
    /// The file at this path.
    pub fn file(&self, path: &str) -> Option<&ExportedFile> {
        self.files.iter().find(|f| f.path == path)
    }

    /// Every record written, decoded back, in file order.
    ///
    /// For a caller that wants to compare what an export holds with something else; the
    /// files decode because they were encoded here.
    pub fn records(&self) -> Vec<Record> {
        self.files
            .iter()
            .flat_map(|f| {
                smysl_core::from_cbor_seq(&f.bytes)
                    .expect("an exported file decodes")
                    .0
            })
            .collect()
    }
}

/// Why an export was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LibError {
    /// The selection names a unit the store does not hold.
    NotInStore(Uid),
}

impl fmt::Display for LibError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibError::NotInStore(u) => {
                write!(
                    f,
                    "the selection names {}, which the store does not hold",
                    u.short()
                )
            }
        }
    }
}

impl std::error::Error for LibError {}

/// Export `sel` from `store`: one canonical file per layout key, closed under §3.6's rules.
///
/// Pure: the same records and options give the same bytes, whatever order the store's log
/// holds them in (rule D).
pub fn export_store(
    store: &Store,
    sel: &Selection,
    opts: &ExportOptions,
) -> Result<Collection, LibError> {
    if let Some(u) = sel.units.iter().find(|u| !store.contains_uid(u)) {
        return Err(LibError::NotInStore(*u));
    }
    let src = Source::new(store);

    let keyed: BTreeMap<Uid, String> = match opts.layout {
        Layout::Single => sel.units.iter().map(|u| (*u, "all".to_string())).collect(),
        Layout::ByView => layout::by_view(store, &sel.units),
    };
    let mut parts: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for (u, k) in keyed {
        parts.entry(k).or_default().insert(u);
    }
    let keys: BTreeSet<String> = parts.keys().cloned().collect();
    let (slugs, suffixed) = layout::slugs(&keys);

    let mut files = Vec::new();
    let mut left_out: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut dangling = BTreeSet::new();
    let mut summed = 0;
    let mut once: BTreeSet<[u8; 32]> = BTreeSet::new();
    for (key, roots) in &parts {
        let units = select::close(&src, roots, opts);
        let picked: Picked<'_> = select::pick(&src, &units, &opts.with);
        for (t, n) in &picked.left_out {
            *left_out.entry(t).or_default() += n;
        }
        // Per record, not per file: a record in two files is one record naming a missing uid.
        dangling.extend(picked.dangling.iter().copied());
        summed += picked.records.len();
        once.extend(picked.records.keys().map(|(_, h)| *h));
        files.push(write::file(store, &slugs[key], roots, &picked));
    }

    let mut report = Report::new();
    let unknown: Vec<&Record> = src
        .entries
        .iter()
        .map(|e| e.record)
        .filter(|r| r.is_unknown() && !select::is_checkpoint(r))
        .collect();
    if !unknown.is_empty() {
        let verb = match opts.unknown {
            UnknownRecords::Include => "exported to unattributed.cbor",
            UnknownRecords::Drop => "dropped",
        };
        report.push(
            Diagnostic::new(Code::W425)
                .with_message(format!("{} unknown record(s) {verb}", unknown.len())),
        );
        if opts.unknown == UnknownRecords::Include {
            files.push(write::unattributed(&unknown));
        }
    }
    for (t, n) in left_out {
        report.push(Diagnostic::new(Code::W490).with_message(format!(
            "{n} {t} record(s) left out of a file that does not hold everything they name"
        )));
    }
    if !dangling.is_empty() {
        report.push(Diagnostic::new(Code::W495).with_message(format!(
            "{} exported record(s) name uids the source store does not hold",
            dangling.len()
        )));
    }
    if suffixed > 0 {
        report.push(Diagnostic::new(Code::W497).with_message(format!(
            "{suffixed} layout key(s) given a hash suffix in their view id"
        )));
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let written: Vec<Record> = files
        .iter()
        .flat_map(|f| smysl_core::from_cbor_seq(&f.bytes).expect("encoded here").0)
        .collect();
    Ok(Collection {
        rsd: record_set_digest(&written),
        files,
        duplication: (summed, once.len()),
        report,
    })
}
