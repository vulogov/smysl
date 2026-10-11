//! Versification schemes: a named map between two numberings (TX-P4 step 4).
//!
//! # The problem this is for
//!
//! Psalm 14 in a Hebrew-numbered Bible is Psalm 13 in a Septuagint-numbered one, and the shift
//! runs from Psalm 10 to Psalm 147. Two manifests can hold the *same* text, cut into parts with
//! the same policy, and address it with locators that never agree. A reader cannot fix this —
//! the numbering is not in the file, and two Bibles with identical markup can be numbered
//! differently — so it is a fact about the expression, declared by the manifest (key 14,
//! `versification`) and resolved by a map between the two declarations.
//!
//! # Why the maps live in the library and not in the crate
//!
//! RFC SMYSL-2.4 §4.1 reserved `data/versification/` in this crate for shipped maps, "licence
//! per OQ-4", and OQ-4 is open: a complete KJV ↔ Synodal map is a dataset with a provenance and
//! a licence, and this repository has neither. But that is not the only reason, and the other
//! reason is the stronger one: the `align` table is **derived**, and §3.8's gate is that a
//! rebuild from the library alone reproduces it. A map passed on a command line would make the
//! table depend on an argument nobody recorded — the next rebuild would produce a different
//! table, or none, and the gate would be measuring a coincidence.
//!
//! So a map is installed into the library it describes, at `versification/<name>.tsv`, and the
//! rebuild reads what is there. `text align --scheme FILE` installs and then aligns. A shipped
//! map, when OQ-4 is settled, is a file copied into the same place by the same code.
//!
//! # The format
//!
//! ```text
//! smysl/versification/1
//! from    kjv
//! to      synodal
//! Ps.14   Ps.13
//! Ps.51.1 Ps.50.2
//! ```
//!
//! Tab-separated, the first line a format marker, then `from` and `to` naming the two schemes as
//! a manifest's key 14 names them, then the rows. A row maps one locator to one locator in the
//! canonical OSIS-shaped vocabulary §3.3 settled on.
//!
//! **A chapter row carries its verses.** `Ps.14 → Ps.13` maps `Ps.14.3` to `Ps.13.3`, because a
//! whole-chapter offset is what most of a versification difference *is*, and a map that had to
//! state every verse of every psalm would be thousands of rows to say one thing. An exact row
//! always wins, so a scheme whose verse numbering also moves — the Hebrew titles are verse 1 in
//! the Septuagint numbering, which shifts every later verse of that psalm — states those verses
//! and keeps the carry for the rest.
//!
//! # What is deliberately not here
//!
//! No inference, no fuzzy matching, no "nearest verse". A locator the scheme does not cover is
//! **not aligned and counted** (`SMY-W451`): an alignment is the thing a later engine grounds
//! `x.text/same-as` on (TX-P7's `anchored`), and a guessed alignment would put a claim about one
//! verse onto another.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The directory a library keeps its maps in, relative to the root.
pub const DIR: &str = "versification";

/// The format marker a map's first line carries.
pub const MARKER: &str = "smysl/versification/1";

/// The extension an installed map has.
pub const EXT: &str = "tsv";

/// A map between two numberings, in one direction.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Scheme {
    /// The file's stem, which is what `text align` reports and what the `align` table records.
    pub name: String,
    /// The `versification` value of the manifests this maps *from*.
    pub from: String,
    /// The `versification` value of the manifests this maps *to*.
    pub to: String,
    rows: BTreeMap<String, String>,
}

/// Why a map could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemeError {
    /// The first line is not [`MARKER`].
    NotAMap,
    /// `from` or `to` is missing, or names the same scheme twice.
    Endpoints,
    /// A row that is not two tab-separated locators, with the line number.
    Row(usize),
    /// A locator mapped twice, with the line number of the second.
    Duplicate(usize),
}

impl core::fmt::Display for SchemeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            SchemeError::NotAMap => write!(f, "not a versification map: line 1 must be `{MARKER}`"),
            SchemeError::Endpoints => f.write_str(
                "a map needs `from` and `to` lines naming two different versification schemes",
            ),
            SchemeError::Row(n) => write!(f, "line {n} is not two tab-separated locators"),
            SchemeError::Duplicate(n) => write!(f, "line {n} maps a locator the map already maps"),
        }
    }
}

impl std::error::Error for SchemeError {}

impl Scheme {
    /// Parse a map. `name` is the file's stem.
    ///
    /// Strict about every line, because a map is read on every rebuild and a row nobody noticed
    /// was malformed is an alignment nobody noticed was missing. A duplicate is refused rather
    /// than last-wins for the reason the index's tables are sets: two rows for one locator is a
    /// disagreement, and picking one of them hides it.
    pub fn parse(name: &str, text: &str) -> Result<Scheme, SchemeError> {
        let mut lines = text.lines().enumerate();
        match lines.next() {
            Some((_, first)) if first.trim() == MARKER => {}
            _ => return Err(SchemeError::NotAMap),
        }
        let mut from = None;
        let mut to = None;
        let mut rows: BTreeMap<String, String> = BTreeMap::new();
        for (i, line) in lines {
            let n = i + 1;
            let line = line.trim_end();
            if line.trim().is_empty() || line.trim_start().starts_with('#') {
                continue;
            }
            let Some((a, b)) = line.split_once('\t') else {
                return Err(SchemeError::Row(n));
            };
            let (a, b) = (a.trim(), b.trim());
            if a.is_empty() || b.is_empty() {
                return Err(SchemeError::Row(n));
            }
            match a {
                "from" => from = Some(b.to_string()),
                "to" => to = Some(b.to_string()),
                _ => {
                    if rows.insert(a.to_string(), b.to_string()).is_some() {
                        return Err(SchemeError::Duplicate(n));
                    }
                }
            }
        }
        let (Some(from), Some(to)) = (from, to) else {
            return Err(SchemeError::Endpoints);
        };
        if from == to {
            return Err(SchemeError::Endpoints);
        }
        Ok(Scheme {
            name: name.to_string(),
            from,
            to,
            rows,
        })
    }

    /// How many rows the map states.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The rows, in locator order.
    pub fn rows(&self) -> impl Iterator<Item = (&str, &str)> {
        self.rows.iter().map(|(a, b)| (a.as_str(), b.as_str()))
    }

    /// Whether this map goes from `a`'s numbering to `b`'s.
    pub fn connects(&self, a: &str, b: &str) -> bool {
        self.from == a && self.to == b
    }

    /// The locator `locator` becomes, or `None` when the map does not cover it.
    ///
    /// An exact row first, then the chapter carry: `Ps.14.3` with only `Ps.14 → Ps.13` stated
    /// becomes `Ps.13.3`. The carry is one level deep by construction — it drops the last
    /// dot-separated component, looks that up, and puts the component back — so a book-level row
    /// does not silently renumber a whole Bible's verses through two levels of guessing.
    pub fn map(&self, locator: &str) -> Option<String> {
        if let Some(exact) = self.rows.get(locator) {
            return Some(exact.clone());
        }
        let (head, last) = locator.rsplit_once('.')?;
        let mapped = self.rows.get(head)?;
        Some(format!("{mapped}.{last}"))
    }

    /// The reverse map, for aligning the other way.
    ///
    /// A map is written in one direction and alignment is asked in both. The reverse is built
    /// rather than read from a second file so that the two cannot disagree — and it refuses when
    /// the forward map is not injective, because a reverse that silently dropped one of two rows
    /// pointing at the same locator would answer a question with the wrong verse.
    pub fn reversed(&self) -> Option<Scheme> {
        let mut rows: BTreeMap<String, String> = BTreeMap::new();
        for (a, b) in &self.rows {
            if rows.insert(b.clone(), a.clone()).is_some() {
                return None;
            }
        }
        Some(Scheme {
            name: self.name.clone(),
            from: self.to.clone(),
            to: self.from.clone(),
            rows,
        })
    }
}

/// Where a library keeps its maps.
pub fn dir(root: &Path) -> PathBuf {
    root.join(DIR)
}

/// Where one map lives.
pub fn path(root: &Path, name: &str) -> PathBuf {
    dir(root).join(format!("{name}.{EXT}"))
}

/// Every map installed in a library, by name.
///
/// A file that is not a map, or is a malformed one, is **skipped** rather than fatal: this is
/// read on every open that rebuilds the `align` table, and a derived table may not stop a
/// library opening — the same rule the sidecar is under. `text align` is where a malformed map
/// is reported, because that is where somebody is holding one.
pub fn installed(root: &Path) -> BTreeMap<String, Scheme> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir(root)) else {
        return out;
    };
    // Collected and sorted: `read_dir` is in whatever order the filesystem gives, and a table
    // built from these has to be the same table on every machine.
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == EXT))
        .collect();
    paths.sort();
    for path in paths {
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(scheme) = Scheme::parse(name, &text) {
            out.insert(scheme.name.clone(), scheme);
        }
    }
    out
}

/// Install a map into a library, replacing one of the same name.
///
/// Parsed before it is written, so a library never holds a file that `installed` will skip. The
/// name comes from the caller rather than from the file's own `from`/`to`, because one pair of
/// numberings can have more than one map between them — a conservative one and a complete one —
/// and which is in use is a thing to be able to say.
pub fn install(root: &Path, name: &str, text: &str) -> Result<Scheme, InstallError> {
    let scheme = Scheme::parse(name, text).map_err(InstallError::Scheme)?;
    std::fs::create_dir_all(dir(root)).map_err(InstallError::Io)?;
    std::fs::write(path(root, name), text).map_err(InstallError::Io)?;
    Ok(scheme)
}

/// Why a map could not be installed.
#[derive(Debug)]
#[non_exhaustive]
pub enum InstallError {
    Scheme(SchemeError),
    Io(std::io::Error),
}

impl core::fmt::Display for InstallError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            InstallError::Scheme(e) => write!(f, "{e}"),
            InstallError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for InstallError {}
