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

use crate::index::{ExportIndex, IndexError, IndexFile, Role, COUNTED};
use select::{Picked, Source};
use smysl_core::{canonical_uid, Code, Diagnostic, Record, RelKind, Report, Uid, ViewId};
use smysl_graph::{record_set_digest, EdgeKind, EdgeSet, Store};
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

    /// The layout an index names.
    pub fn parse(s: &str) -> Option<Layout> {
        match s {
            "single" => Some(Layout::Single),
            "by-view" => Some(Layout::ByView),
            _ => None,
        }
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

    let (keyed, views): (BTreeMap<Uid, String>, BTreeMap<String, ViewId>) = match opts.layout {
        Layout::Single => (
            sel.units.iter().map(|u| (*u, "all".to_string())).collect(),
            BTreeMap::new(),
        ),
        Layout::ByView => layout::by_view(store, &sel.units),
    };
    let mut parts: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for (u, k) in keyed {
        parts.entry(k).or_default().insert(u);
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

    files.sort_by(|a, b| a.path.cmp(&b.path));
    let written: Vec<Record> = files
        .iter()
        .flat_map(|f| smysl_core::from_cbor_seq(&f.bytes).expect("encoded here").0)
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
        scope: String::new(),
        sq_version: None,
        schemas: schemas.into_iter().collect(),
        lens_chain: None,
        layout: opts.layout.as_str().to_string(),
        closure: opts.closure.as_str().to_string(),
        format: "cbor".to_string(),
        as_of: None,
        tool: crate::TOOL.to_string(),
        rsd: record_set_digest(&written),
        flags: flags(store, opts),
        derived: None,
        complete: true,
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
    if opts.unknown == UnknownRecords::Drop {
        out.push("unknown-drop".to_string());
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

/// The options an index records, over the store a collection was imported into: what P3
/// re-exports with. `None` for a layout or closure this build does not know.
pub(crate) fn options_of(index: &ExportIndex, store: &Store) -> Option<ExportOptions> {
    let mut o = ExportOptions::new()
        .layout(Layout::parse(&index.layout)?)
        .closure(Closure::parse(&index.closure)?);
    for flag in &index.flags {
        match flag.as_str() {
            "with-views" => o.with.views = true,
            "with-packs" => o.with.packs = true,
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
