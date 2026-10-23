//! Export: a selection of a store, cut by a layout into canonical files (SMYSL-2.7 §3.6–§3.8).
//!
//! [`export_store`] takes a store the caller holds and the units to export, partitions those
//! units by the layout key, closes each part to a fixpoint, selects records by one rule per
//! type, and writes each file in one canonical order. The bytes are a
//! function of the store's records and the options: no clock, no log order, no walk order.
//!
//! [`export_library`] does the same over a library: its text, from the object store beside
//! the log (`--with-text`), its staged and quarantined batches when asked, and its `imported`
//! table for `--with-provenance`. [`snapshot`] is the store it exports from.

mod format;
mod layout;
mod select;
pub mod target_revision;
mod time;
mod write;

pub use layout::slug;
pub(crate) use select::is_checkpoint;
pub use target_revision::TargetRevision;

use crate::access::{LibraryRead, NoText, TextSource};
use crate::index::{ExportIndex, IndexError, IndexFile, Role, COUNTED};
use select::{Picked, Source};
use smysl_core::{
    canonical_uid, Carry, Code, Diagnostic, Mid, Record, RelKind, Report, Tid, Uid, ViewId,
};
use smysl_graph::{record_set_digest, EdgeKind, EdgeSet, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// How the selection is cut into files (§3.4).
///
/// `single` and `by-view` in TX-P11 step 2, `by-expression` and `by-collection` in step 6,
/// `by-time` in step 7, `by-query` in step 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Layout {
    /// One file, key `all`.
    #[default]
    Single,
    /// One file per corpus view, keyed by its id with `/` as `-`; units in no view go to
    /// `unkeyed`. A unit in several views goes to the least view id.
    ByView,
    /// One file per expression: the alias of the manifest a unit's source names. Units that
    /// name none go to `unkeyed`.
    ByExpression,
    /// One file per collection: that alias's namespace, up to its first `/`.
    ByCollection,
    /// One file per window of effective time (rule E, `said`), `ExportOptions::window` wide
    /// and aligned to the Unix epoch. Undated units go to `unkeyed`.
    ByTime,
    /// One file per value of a `group by` field: the keys [`crate::scope::group`] puts on the
    /// selection.
    ByQuery,
}

impl Layout {
    /// The layout's name, as an index records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Layout::Single => "single",
            Layout::ByView => "by-view",
            Layout::ByExpression => "by-expression",
            Layout::ByCollection => "by-collection",
            Layout::ByTime => "by-time",
            Layout::ByQuery => "by-query",
        }
    }

    /// The layout an index names.
    pub fn parse(s: &str) -> Option<Layout> {
        match s {
            "single" => Some(Layout::Single),
            "by-view" => Some(Layout::ByView),
            "by-expression" => Some(Layout::ByExpression),
            "by-collection" => Some(Layout::ByCollection),
            "by-time" => Some(Layout::ByTime),
            "by-query" => Some(Layout::ByQuery),
            _ => None,
        }
    }
}

/// How a collection's keyed files are written (§3.8).
///
/// Every format selects and closes the same records; `surface` and `pack` are made from each
/// file's canonical CBOR form, so the formats cut the same files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Format {
    /// Canonical CBOR, which P1 and P3 are stated for.
    #[default]
    Cbor,
    /// The surface text, `<slug>.smy`. Records with no surface form are counted (`SMY-W428`),
    /// and a file that does not read back as the rest is `SMY-E424`, with the index marked
    /// incomplete.
    Surface,
    /// A budgeted pack of each file, `<slug>.pack.cbor`: lossy, and outside P1 and P3.
    Pack { budget: u64 },
}

impl Format {
    /// The format's name, as an index records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Format::Cbor => "cbor",
            Format::Surface => "surface",
            Format::Pack { .. } => "pack",
        }
    }
}

/// A file's records, read as its name says it is written: surface for `.smy`, CBOR otherwise.
pub(crate) fn decode_file(path: &str, bytes: &[u8]) -> Result<Vec<Record>, String> {
    if path.ends_with(".smy") {
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        smysl_core::surface::parse_surface(text)
            .map(|o| o.records)
            .map_err(|e| e.to_string())
    } else {
        smysl_core::from_cbor_seq(bytes)
            .map(|(rs, _)| rs)
            .map_err(|e| e.to_string())
    }
}

/// What a file holds of what its selection needs (§3.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Closure {
    /// Every file holds everything selected for its key, so each file is closed on its own.
    #[default]
    Full,
    /// Every record is written once, by the file that owns it. A file lists in the index the
    /// files it takes the rest from, and a collection is imported whole.
    Ref,
}

impl Closure {
    /// The mode's name, as an index records it.
    pub const fn as_str(self) -> &'static str {
        match self {
            Closure::Full => "full",
            Closure::Ref => "ref",
        }
    }

    /// The mode an index names.
    pub fn parse(s: &str) -> Option<Closure> {
        match s {
            "full" => Some(Closure::Full),
            "ref" => Some(Closure::Ref),
            _ => None,
        }
    }
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
    /// The text of the parts the selected units are drawn from, where the manifest's carry
    /// and licence allow it and the part is not redacted (rows 15 and 18).
    pub text: bool,
    /// The import manifests a library's `imported` table lists the selected records under
    /// (row 14). Outside P3: a re-export names the importer's manifests, not these.
    pub provenance: bool,
    /// A library's staged batches, in the snapshot (§3.6).
    pub staged: bool,
    /// A library's quarantined records, in the snapshot (§3.6).
    pub quarantine: bool,
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

    pub fn text(mut self, on: bool) -> With {
        self.text = on;
        self
    }

    pub fn provenance(mut self, on: bool) -> With {
        self.provenance = on;
        self
    }

    pub fn staged(mut self, on: bool) -> With {
        self.staged = on;
        self
    }

    pub fn quarantine(mut self, on: bool) -> With {
        self.quarantine = on;
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
    /// The window of `by-time`, in milliseconds; a day when unset.
    pub window: Option<u64>,
    /// `--as-of`: export only what was known before the end of this EDTF value (§3.10).
    pub as_of: Option<String>,
    /// How the keyed files are written (§3.8).
    pub format: Format,
    /// `--target-revision ID@N` (§3.11): the selection rewritten as revision `N` writes it.
    pub target_revision: Option<TargetRevision>,
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

    pub fn closure(mut self, closure: Closure) -> ExportOptions {
        self.closure = closure;
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

    pub fn window(mut self, ms: u64) -> ExportOptions {
        self.window = Some(ms);
        self
    }

    pub fn as_of(mut self, edtf: impl Into<String>) -> ExportOptions {
        self.as_of = Some(edtf.into());
        self
    }

    pub fn format(mut self, format: Format) -> ExportOptions {
        self.format = format;
        self
    }

    pub fn target_revision(mut self, t: TargetRevision) -> ExportOptions {
        self.target_revision = Some(t);
        self
    }
}

/// `by-time`'s window when none is given: a day, the window a chat is most often read by.
pub const DEFAULT_WINDOW_MS: u64 = 86_400_000;

/// The exclusive upper bound of an `--as-of` value, in Unix milliseconds: the end of the
/// interval its EDTF admits, so `2024-01` keeps all of January.
fn as_of_bound(edtf: &str) -> Result<i64, LibError> {
    let v = smysl_text::time::edtf::parse(edtf)
        .map_err(|e| LibError::AsOf(format!("`{edtf}` is not EDTF: {e}")))?;
    match smysl_text::time::edtf::to_interval(&v).hi {
        smysl_text::time::Bound::At(at) => Ok(at.0),
        smysl_text::time::Bound::Open => Err(LibError::AsOf(format!(
            "`{edtf}` has no upper bound, so it excludes nothing"
        ))),
    }
}

/// The units an export selects (`Sel`, §3.6), before the layout partitions them.
///
/// What an `sq` scope returns ([`crate::scope::query`]), or the units a caller names, or all of
/// them. It carries the scope's text for the index, and, for `by-query`, the key of each unit.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Selection {
    units: BTreeSet<Uid>,
    scope: Option<String>,
    group: Option<(String, BTreeMap<Uid, String>)>,
}

impl Selection {
    /// Every unit the store holds.
    pub fn all(store: &Store) -> Selection {
        Selection {
            units: store.units().map(|(u, _)| *u).collect(),
            ..Selection::default()
        }
    }

    /// These units.
    pub fn of(units: impl IntoIterator<Item = Uid>) -> Selection {
        Selection {
            units: units.into_iter().collect(),
            ..Selection::default()
        }
    }

    /// The same units, recorded as answering this scope (index key 2).
    pub fn scoped(mut self, text: impl Into<String>) -> Selection {
        self.scope = Some(text.into());
        self
    }

    /// The same units, keyed by the value of a `group by` field, for `by-query`.
    pub(crate) fn grouped(mut self, field: &str, keys: BTreeMap<Uid, String>) -> Selection {
        self.group = Some((field.to_string(), keys));
        self
    }

    /// The scope's text, when the units answer one.
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
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
    /// Under `ref`, the files this one takes records from, by path, sorted. It is in the
    /// index, never in the view (draft 3 §16.7). Empty under `full`.
    pub requires: Vec<String>,
}

/// What an export wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Collection {
    /// The files, sorted by path.
    pub files: Vec<ExportedFile>,
    /// What the collection says about itself, written last as `export-index.cbor`.
    pub index: ExportIndex,
    /// What the export counted: `SMY-W425`, `W490`, `W495`, `W497`.
    pub report: Report,
}

impl Collection {
    /// The record-set digest of everything written, every file's records together: `rsd(X)`,
    /// what P1 compares an import's retained records against.
    pub fn rsd(&self) -> [u8; 32] {
        self.index.rsd
    }

    /// The duplication factor's two terms (§3.7): the records selected summed over files, and
    /// the records selected counted once. Generated views are in neither. The same under
    /// `full` and `ref`, since it measures what the files need, not what they hold.
    pub fn duplication(&self) -> (u64, u64) {
        self.index.duplication
    }

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
            .flat_map(|f| decode_file(&f.path, &f.bytes).expect("an exported file decodes"))
            .collect()
    }
}

/// Why an export was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LibError {
    /// The selection names a unit the store does not hold.
    NotInStore(Uid),
    /// An index that does not decode.
    Index(IndexError),
    /// A file could not be read or written. The path, and what the system said.
    Io(String, String),
    /// The library refused a write, and why.
    Library(String),
    /// An import name that is not a slug (`[a-z0-9_-]`, starting with a letter, at most 48).
    BadName(String),
    /// Another import already has this name.
    NameTaken(String),
    /// No import has this name.
    NoImport(String),
    /// The import is not staged, so there is nothing to accept or discard.
    NotStaged(String),
    /// `accept --fail-on-contention` found this many new contentions, and merged nothing.
    Contentions(usize),
    /// An `--as-of` value that is not EDTF, or names no upper bound.
    AsOf(String),
    /// A scope that does not parse, plan or run, or answers no units.
    Query(String),
    /// A file's selection does not pack in the budget given.
    Pack(String),
    /// `--target-revision` refused (§3.11): the code says why — `SMY-E422` a backward path
    /// with no inverse, `SMY-E470` no path or no such revision, `SMY-E471` an ill-formed step,
    /// `SMY-E492` a cycle among the units to rewrite, `SMY-W429` a forked schema.
    TargetRevision(Code, String),
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
            LibError::Index(e) => write!(f, "{e}"),
            LibError::Io(path, e) => write!(f, "{path}: {e}"),
            LibError::Library(e) => write!(f, "the library refused the write: {e}"),
            LibError::BadName(n) => write!(f, "`{n}` is not an import name: lowercase letters, digits, `-` and `_`, starting with a letter"),
            LibError::NameTaken(n) => write!(f, "an import named `{n}` already exists"),
            LibError::NoImport(n) => write!(f, "no import is named `{n}`"),
            LibError::NotStaged(n) => write!(f, "import `{n}` is not staged"),
            LibError::Contentions(k) => write!(f, "{k} new contention(s); nothing was merged"),
            LibError::AsOf(e) => write!(f, "--as-of {e}"),
            LibError::Query(e) => write!(f, "--scope: {e}"),
            LibError::Pack(e) => write!(f, "--format pack: {e}"),
            LibError::TargetRevision(c, e) => write!(f, "--target-revision: {c}: {e}"),
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
    export_with(store, &NoText, None, sel, opts)
}

/// The store a library exports from: its log, with its staged batches under `--with-staged`
/// and its quarantined records under `--with-quarantine` (§3.6's snapshot). Without them,
/// staged and quarantined records are never in it.
pub fn snapshot(lib: &impl LibraryRead, with: &With) -> Result<Store, LibError> {
    if !with.staged && !with.quarantine {
        return Ok(lib.snapshot().clone());
    }
    let mut records: Vec<Record> = lib.snapshot().iter().cloned().collect();
    let lib_dir = lib.root().join("lib");
    let mut dirs = Vec::new();
    if with.staged {
        dirs.push(lib_dir.join("staged"));
    }
    if with.quarantine {
        dirs.push(lib_dir.join("quarantine"));
    }
    for d in dirs {
        for p in cbor_files(&d)? {
            let bytes = std::fs::read(&p)
                .map_err(|e| LibError::Io(p.display().to_string(), e.to_string()))?;
            let (rs, _) = smysl_core::from_cbor_seq(&bytes)
                .map_err(|e| LibError::Io(p.display().to_string(), e.to_string()))?;
            records.extend(rs);
        }
    }
    Ok(Store::from_records(records))
}

/// Every `.cbor` file under `dir`, one level of subdirectories deep, in path order.
fn cbor_files(dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>, LibError> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(out);
    };
    for e in entries.filter_map(|e| e.ok()) {
        let p = e.path();
        if p.is_dir() {
            out.extend(cbor_files(&p)?);
        } else if p.extension().is_some_and(|x| x == "cbor") {
            out.push(p);
        }
    }
    out.sort();
    Ok(out)
}

/// Export `sel` from a library: `snapshot` is [`snapshot`]'s, and the library supplies the
/// text the manifests name and the `imported` table `--with-provenance` reads.
pub fn export_library(
    lib: &impl LibraryRead,
    snapshot: &Store,
    sel: &Selection,
    opts: &ExportOptions,
) -> Result<Collection, LibError> {
    let tables;
    let provenance = if opts.with.provenance {
        tables = crate::tables::Tables::load(lib.root())?;
        Some(
            tables
                .imported
                .iter()
                .map(|(h, set)| {
                    let mids: BTreeSet<Mid> = set
                        .iter()
                        .filter(|(_, st)| *st != crate::tables::State::Discarded)
                        .map(|(m, _)| *m)
                        .collect();
                    (*h, mids)
                })
                .collect::<BTreeMap<[u8; 32], BTreeSet<Mid>>>(),
        )
    } else {
        None
    };
    export_with(snapshot, lib, provenance.as_ref(), sel, opts)
}

/// The text of every part a unit of `store` is drawn from that a manifest may carry, from
/// `text`: what rows 15 and 18 choose among. Only parts some unit references are read, so a
/// library's whole object store is not, and the rows decide which of these a file takes.
fn text_records(store: &Store, text: &dyn TextSource) -> Vec<Record> {
    let referenced: BTreeSet<Tid> = store
        .units()
        .filter_map(|(_, u)| u.core.source.as_ref())
        .filter_map(|s| Store::part_of_reference(&s.reference))
        .collect();
    let mut out = Vec::new();
    for (_, m) in store.manifests() {
        if m.carry != Carry::Text || smysl_text::carry_allowed(&m.licence, m.carry).is_err() {
            continue;
        }
        for e in m
            .parts
            .iter()
            .filter(|e| referenced.contains(&e.tid) && !store.is_redacted(&e.tid))
        {
            out.extend(text.part(&e.tid).map(Record::PartText));
            out.extend(text.reading(&e.rdid).map(Record::PartReading));
        }
    }
    out
}

/// [`export_store`] with a text source and, for a library, its provenance.
pub(crate) fn export_with(
    store: &Store,
    text: &dyn TextSource,
    provenance: Option<&BTreeMap<[u8; 32], BTreeSet<Mid>>>,
    sel: &Selection,
    opts: &ExportOptions,
) -> Result<Collection, LibError> {
    if let Some(u) = sel.units.iter().find(|u| !store.contains_uid(u)) {
        return Err(LibError::NotInStore(*u));
    }
    // `--as-of` (§3.10): the known-time subset, built before anything is selected, and the
    // selection cut to the units it keeps. What it left out is counted, not refused.
    let subset;
    let (store, sel, known_counts) = match &opts.as_of {
        Some(edtf) => {
            subset = time::known(store, as_of_bound(edtf)?);
            let units: BTreeSet<Uid> = sel
                .units
                .iter()
                .filter(|u| subset.store.contains_uid(u))
                .copied()
                .collect();
            (
                &subset.store,
                Selection {
                    units,
                    ..sel.clone()
                },
                (subset.unknown, subset.unclosed),
            )
        }
        None => (store, sel.clone(), (0, 0)),
    };
    // `--target-revision` (§3.11): the store rewritten at revision N, and the selection carried
    // to what its units became. Like `--as-of`, the rest of the pipeline is not told.
    let rewritten = match &opts.target_revision {
        Some(t) => Some(target_revision::rewrite(store, &sel.units, t)?),
        None => None,
    };
    let (store, sel) = match &rewritten {
        Some(rw) => {
            let map = rw.map();
            let to = |u: &Uid| *map.get(u).unwrap_or(u);
            let group = sel.group.as_ref().map(|(f, keys)| {
                let keys = keys.iter().map(|(u, k)| (to(u), k.clone())).collect();
                (f.clone(), keys)
            });
            let sel = Selection {
                units: sel.units.iter().map(to).collect(),
                scope: sel.scope.clone(),
                group,
            };
            (&rw.store, sel)
        }
        None => (store, sel),
    };
    let sel = &sel;
    let extra = if opts.with.text {
        text_records(store, text)
    } else {
        Vec::new()
    };
    let src = Source::new(store, &extra, provenance);

    let (keyed, views): (BTreeMap<Uid, String>, BTreeMap<String, ViewId>) = match opts.layout {
        Layout::Single => (
            sel.units.iter().map(|u| (*u, "all".to_string())).collect(),
            BTreeMap::new(),
        ),
        Layout::ByView => layout::by_view(store, &sel.units),
        Layout::ByExpression => (
            layout::by_manifest(store, &sel.units, false),
            BTreeMap::new(),
        ),
        Layout::ByCollection => (
            layout::by_manifest(store, &sel.units, true),
            BTreeMap::new(),
        ),
        Layout::ByTime => (
            time::by_time(store, &sel.units, opts.window.unwrap_or(DEFAULT_WINDOW_MS)),
            BTreeMap::new(),
        ),
        Layout::ByQuery => {
            let Some((_, keys)) = &sel.group else {
                return Err(LibError::Query(
                    "by-query files units by a `group by` field, and the selection has none"
                        .to_string(),
                ));
            };
            (
                sel.units
                    .iter()
                    .map(|u| (*u, keys.get(u).cloned().unwrap_or_else(|| "unkeyed".into())))
                    .collect(),
                BTreeMap::new(),
            )
        }
    };
    let mut parts: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for (u, k) in keyed {
        parts.entry(k).or_default().insert(u);
    }
    // `single` is one file whatever it selects. A store of records that name no unit, F12's
    // three pack infos, has nothing for the other layouts to key, and its records belong in
    // every file they are vacuously closed in; with no file at all they had nowhere to go, and
    // P2 lost them (found by the cross-implementation harness, TX-P11 step 11).
    if opts.layout == Layout::Single {
        parts.entry("all".to_string()).or_default();
    }
    let keys: BTreeSet<String> = parts.keys().cloned().collect();
    let (slugs, suffixed) = layout::slugs(&keys);

    // What each key selects, before anything is written: `ref` needs every file's selection
    // to decide which file owns a record.
    let mut cut: Vec<(String, &BTreeSet<Uid>, Picked<'_>)> = Vec::new();
    let mut left_out: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut dangling = BTreeSet::new();
    let mut summed = 0;
    let mut once: BTreeSet<[u8; 32]> = BTreeSet::new();
    for (key, roots) in &parts {
        let view = views.get(key);
        let units = select::close(&src, roots, opts, view);
        let picked: Picked<'_> = select::pick(&src, &units, &opts.with, view);
        for (t, n) in &picked.left_out {
            *left_out.entry(t).or_default() += n;
        }
        // Per record, not per file: a record in two files is one record naming a missing uid.
        dangling.extend(picked.dangling.iter().copied());
        summed += picked.records.len();
        once.extend(picked.records.keys().map(|(_, h)| *h));
        cut.push((format!("{}.cbor", slugs[key]), roots, picked));
    }
    let owners = match opts.closure {
        Closure::Full => BTreeMap::new(),
        Closure::Ref => owners(&cut),
    };

    let mut files = Vec::new();
    for (path, roots, picked) in &cut {
        let slug = &path[..path.len() - ".cbor".len()];
        let (written, requires): (Vec<&Record>, Vec<String>) = match opts.closure {
            Closure::Full => (picked.records.values().copied().collect(), Vec::new()),
            Closure::Ref => {
                let mut written = Vec::new();
                let mut requires = BTreeSet::new();
                for ((_, h), r) in &picked.records {
                    match &owners[h] {
                        o if o == path => written.push(*r),
                        o => {
                            requires.insert(o.clone());
                        }
                    }
                }
                (written, requires.into_iter().collect())
            }
        };
        files.push(write::file(store, slug, roots, picked, &written, requires));
    }

    let mut report = Report::new();
    if let Some(rw) = &rewritten {
        report.extend(rw.report.iter().cloned());
    }

    // The derived forms (§3.8), made from each keyed file's CBOR one.
    let mut w428 = 0usize;
    let mut complete = true;
    match opts.format {
        Format::Cbor => {}
        Format::Surface => {
            let mut omitted: BTreeMap<&'static str, usize> = BTreeMap::new();
            for f in &mut files {
                let s = format::surface(f, store);
                for (t, n) in s.omitted {
                    *omitted.entry(t).or_default() += n;
                }
                if let Some(why) = s.mismatch {
                    complete = false;
                    report.push(Diagnostic::new(Code::E424).with_message(why));
                }
                *f = s.file;
            }
            for (t, n) in omitted {
                w428 += n;
                report.push(Diagnostic::new(Code::W428).with_message(format!(
                    "{n} {t} record(s) have no surface form and were left out of the surface files"
                )));
            }
        }
        Format::Pack { budget } => {
            for f in &mut files {
                *f = format::pack(f, budget).map_err(LibError::Pack)?;
            }
        }
    }

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
    let w490: usize = left_out.values().sum();
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
    let (w415, w498) = known_counts;
    if w415 > 0 {
        report.push(Diagnostic::new(Code::W415).with_message(format!(
            "{w415} unit(s) left out of an --as-of export: no attestation says when they were known"
        )));
    }
    if w498 > 0 {
        report.push(Diagnostic::new(Code::W498).with_message(format!(
            "{w498} unit(s) left out of an --as-of export: their deps or grounds were not yet known"
        )));
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let written: Vec<Record> = files
        .iter()
        .flat_map(|f| decode_file(&f.path, &f.bytes).expect("encoded here"))
        .collect();
    let schemas: BTreeSet<(String, u32)> = cut
        .iter()
        .flat_map(|(_, _, p)| p.records.values())
        .filter_map(|r| match r {
            Record::SchemaDecl(d) => Some((d.id.as_str().to_string(), d.version)),
            _ => None,
        })
        .collect();
    let mut counts: BTreeMap<String, u64> =
        COUNTED.iter().map(|c: &&str| (c.to_string(), 0)).collect();
    for (code, n) in [
        ("SMY-W425", unknown.len()),
        ("SMY-W490", w490),
        ("SMY-W495", dangling.len()),
        ("SMY-W497", suffixed),
        ("SMY-W415", w415),
        ("SMY-W498", w498),
        ("SMY-W428", w428),
        ("SMY-W491", rewritten.as_ref().map_or(0, |rw| rw.dropped)),
    ] {
        counts.insert(code.to_string(), n as u64);
    }
    let index = ExportIndex {
        files: files
            .iter()
            .map(|f| IndexFile {
                path: f.path.clone(),
                blake3: *blake3::hash(&f.bytes).as_bytes(),
                records: f.records as u64,
                rsd: f.rsd,
                view: f.view.as_ref().map(|v| v.as_str().to_string()),
                requires: f.requires.clone(),
                role: if f.view.is_some() {
                    Role::Data
                } else {
                    Role::Unattributed
                },
            })
            .collect(),
        scope: sel.scope.clone().unwrap_or_default(),
        sq_version: sel
            .scope
            .as_ref()
            .map(|_| crate::scope::SQ_VERSION.to_string()),
        schemas: schemas.into_iter().collect(),
        lens_chain: rewritten.as_ref().map(|rw| rw.chain),
        layout: opts.layout.as_str().to_string(),
        closure: opts.closure.as_str().to_string(),
        format: opts.format.as_str().to_string(),
        as_of: opts.as_of.clone(),
        tool: crate::TOOL.to_string(),
        rsd: record_set_digest(&written),
        flags: {
            let mut f = flags(store, opts);
            if let (Layout::ByQuery, Some((field, _))) = (opts.layout, &sel.group) {
                f.push(format!("group-by={field}"));
                f.sort();
            }
            f
        },
        derived: {
            let mut d: Vec<String> = Vec::new();
            d.extend(opts.target_revision.as_ref().map(TargetRevision::derived));
            if matches!(opts.format, Format::Pack { .. }) {
                d.push("lossy".to_string());
            }
            (!d.is_empty()).then(|| d.join(", "))
        },
        complete,
        counts,
        duplication: (summed as u64, once.len() as u64),
    };
    Ok(Collection {
        files,
        index,
        report,
    })
}

/// Under `ref`, the file that writes each record (§3.7): for a unit, the file that selects it
/// as a root, and the layouts give every selected unit one; for anything else, the least path
/// among the files that select it.
fn owners(cut: &[(String, &BTreeSet<Uid>, Picked<'_>)]) -> BTreeMap<[u8; 32], String> {
    let mut owner: BTreeMap<[u8; 32], String> = BTreeMap::new();
    for (path, roots, picked) in cut {
        for ((_, h), r) in &picked.records {
            if matches!(r, Record::Unit(c) if roots.contains(&canonical_uid(c))) {
                owner.insert(*h, path.clone());
            }
        }
    }
    let mut paths: Vec<&(String, &BTreeSet<Uid>, Picked<'_>)> = cut.iter().collect();
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, _, picked) in paths {
        for (_, h) in picked.records.keys() {
            owner.entry(*h).or_insert_with(|| path.clone());
        }
    }
    owner
}

/// The options that change what is written, as the index records them (key 12), sorted.
fn flags(store: &Store, opts: &ExportOptions) -> Vec<String> {
    let mut out = Vec::new();
    if opts.with.views {
        out.push("with-views".to_string());
    }
    if opts.with.packs {
        out.push("with-packs".to_string());
    }
    if opts.with.text {
        out.push("with-text".to_string());
    }
    if opts.with.provenance {
        out.push("with-provenance".to_string());
    }
    if opts.with.staged {
        out.push("with-staged".to_string());
    }
    if opts.with.quarantine {
        out.push("with-quarantine".to_string());
    }
    if opts.unknown == UnknownRecords::Drop {
        out.push("unknown-drop".to_string());
    }
    if opts.layout == Layout::ByTime {
        out.push(format!(
            "window={}",
            opts.window.unwrap_or(DEFAULT_WINDOW_MS)
        ));
    }
    if let Some(e) = &opts.edges {
        let kinds = EdgeSet::support().union(e);
        if kinds != EdgeSet::all() {
            out.push(format!("edges={}", edge_names(store, &kinds).join(",")));
        }
    }
    out.sort();
    out
}

/// The kinds of `kinds` this store has edges of, by name: `deps`, `grounds`, then relation
/// kinds by code, then extensions in their intern order. A kind the store holds no edge of
/// changes nothing an export follows, so it is not named.
fn edge_names(store: &Store, kinds: &EdgeSet) -> Vec<String> {
    let g = store.adjacency();
    let mut out = Vec::new();
    for (k, name) in [(EdgeKind::Deps, "deps"), (EdgeKind::Grounds, "grounds")] {
        if kinds.contains(k) {
            out.push(name.to_string());
        }
    }
    for c in 0..u8::MAX {
        let Some(rk) = RelKind::from_code(c) else {
            break;
        };
        if kinds.contains(EdgeKind::Kernel(c)) {
            out.push(rk.as_str().to_string());
        }
    }
    let mut i = 0;
    while let Some(name) = g.extension_name(i) {
        if kinds.contains(EdgeKind::Extension(i)) {
            out.push(name.to_string());
        }
        i += 1;
    }
    out
}

impl ExportOptions {
    /// The options an index records, over the store a collection was imported into: what P3
    /// re-exports with (§3.9). `None` for a layout, closure or flag this build does not know.
    /// A `by-query` index's field is its `group-by=` flag, which the selection carries rather
    /// than the options ([`crate::scope::group`]).
    pub fn from_index(index: &ExportIndex, store: &Store) -> Option<ExportOptions> {
        options_of(index, store)
    }
}

/// [`ExportOptions::from_index`].
pub(crate) fn options_of(index: &ExportIndex, store: &Store) -> Option<ExportOptions> {
    let mut o = ExportOptions::new()
        .layout(Layout::parse(&index.layout)?)
        .closure(Closure::parse(&index.closure)?);
    o.as_of = index.as_of.clone();
    for flag in &index.flags {
        match flag.as_str() {
            "with-views" => o.with.views = true,
            "with-packs" => o.with.packs = true,
            "with-text" => o.with.text = true,
            "with-provenance" => o.with.provenance = true,
            "with-staged" => o.with.staged = true,
            "with-quarantine" => o.with.quarantine = true,
            f if f.starts_with("window=") => o.window = Some(f["window=".len()..].parse().ok()?),
            // Read by `verify`'s P3 from the index, which re-groups the re-export's selection.
            f if f.starts_with("group-by=") => {}
            "unknown-drop" => o.unknown = UnknownRecords::Drop,
            f => {
                let list = f.strip_prefix("edges=")?;
                let g = store.adjacency();
                let mut e = EdgeSet::of([]);
                for name in list.split(',').filter(|n| !n.is_empty()) {
                    let kind = match name {
                        "deps" => Some(EdgeKind::Deps),
                        "grounds" => Some(EdgeKind::Grounds),
                        n => RelKind::parse(n).ok().and_then(|k| g.edge_kind(&k)),
                    };
                    if let Some(k) = kind {
                        e = e.with(k);
                    }
                }
                o.edges = Some(e);
            }
        }
    }
    Some(o)
}
