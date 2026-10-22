//! Verify a collection against its index: the file checks, P1 and P3 (SMYSL-2.7 §3.9, §4.2).
//!
//! The file checks come first. Each file the index names must exist, have the BLAKE3, the
//! record count and the record-set digest the index gives (`SMY-E494`). Only a collection
//! whose files are all its index's is then read as a whole:
//!
//! - **P1**: what an empty store retains of every file together has the record-set digest the
//!   index gives, `rsd(X)`. Until import arrives (TX-P11 step 4), `Store::from_records` of the
//!   files stands in for `lib import --accept --no-join` into an empty library.
//! - **P3**: a re-export of that store, with the index's layout, closure and flags and the
//!   union of the roots of its generated views as the selection, writes every file byte for
//!   byte, and an index with the same `files` and `rsd`. The `scope` text differs by
//!   construction and is not compared.
//!
//! P2, an arbitrary file imported and exported again, needs import and `from import:` scopes,
//! and arrives with them.

use crate::export::{export_store, options_of, Collection, LibError, Selection};
use crate::index::{ExportIndex, INDEX_FILE};
use smysl_core::{from_cbor_seq, Code, Diagnostic, ExitCode, Report};
use smysl_graph::{record_set_digest, Store};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One file's check against its index.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileCheck {
    pub path: String,
    /// What did not match, or `None` when the file is the index's.
    pub problem: Option<String>,
}

/// Whether a round-trip property holds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Property {
    Holds,
    /// It does not. For P1, the record hashes the files hold and the import did not retain,
    /// and the ones it retained that no file holds. For P3, the first file whose bytes differ
    /// and the offset of the first byte that does.
    Fails {
        missing: Vec<[u8; 32]>,
        extra: Vec<[u8; 32]>,
        first_diff: Option<(String, u64)>,
    },
    /// It was not asked, and why.
    NotApplicable(&'static str),
}

impl Property {
    pub fn holds(&self) -> bool {
        matches!(self, Property::Holds)
    }

    fn fails(&self) -> bool {
        matches!(self, Property::Fails { .. })
    }
}

/// What `verify` found.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct VerifyReport {
    /// Every file the index names, in its order.
    pub files: Vec<FileCheck>,
    pub p1: Property,
    pub p3: Property,
    /// `SMY-E494`, once per file that does not match its index.
    pub report: Report,
}

impl VerifyReport {
    /// Every file matches its index, and neither property fails.
    pub fn holds(&self) -> bool {
        self.files.iter().all(|f| f.problem.is_none()) && !self.p1.fails() && !self.p3.fails()
    }

    /// `lib verify`'s exit (§4.4): 0 when everything holds; 9, hash verification, when a file
    /// or a digest does not match.
    pub fn exit_code(&self) -> ExitCode {
        if self.holds() {
            ExitCode::Success
        } else {
            ExitCode::HashVerification
        }
    }
}

/// Verify a collection in memory, as `lib export` does before it calls its index complete.
pub fn verify_collection(c: &Collection) -> VerifyReport {
    let files: BTreeMap<String, Vec<u8>> = c
        .files
        .iter()
        .map(|f| (f.path.clone(), f.bytes.clone()))
        .collect();
    verify_files(&c.index, &files)
}

/// Verify the collection whose index is at `index`, reading the files it names from the same
/// directory. A file that is absent is a failed check, not an error. An index that does not
/// decode is an error.
pub fn verify(index: &Path) -> Result<VerifyReport, LibError> {
    let io = |p: &Path, e: std::io::Error| LibError::Io(p.display().to_string(), e.to_string());
    let bytes = std::fs::read(index).map_err(|e| io(index, e))?;
    let idx = ExportIndex::from_cbor(&bytes).map_err(LibError::Index)?;
    let dir = index.parent().unwrap_or(Path::new("."));
    let mut files = BTreeMap::new();
    for f in &idx.files {
        if let Ok(b) = std::fs::read(dir.join(&f.path)) {
            files.insert(f.path.clone(), b);
        }
    }
    Ok(verify_files(&idx, &files))
}

/// Write a collection's files into `dir`, creating it, and its index last: a directory with an
/// index in it is a directory whose files were all written.
pub fn write_collection(c: &Collection, dir: &Path) -> Result<(), LibError> {
    let io = |p: &Path, e: std::io::Error| LibError::Io(p.display().to_string(), e.to_string());
    std::fs::create_dir_all(dir).map_err(|e| io(dir, e))?;
    for f in &c.files {
        let p = dir.join(&f.path);
        std::fs::write(&p, &f.bytes).map_err(|e| io(&p, e))?;
    }
    let p = dir.join(INDEX_FILE);
    std::fs::write(&p, c.index.to_cbor()).map_err(|e| io(&p, e))
}

/// The checks, over an index and the bytes of the files found for it.
pub fn verify_files(index: &ExportIndex, files: &BTreeMap<String, Vec<u8>>) -> VerifyReport {
    let mut report = Report::new();
    let mut checks = Vec::new();
    let mut records = Vec::new();
    for f in &index.files {
        let problem = match files.get(&f.path) {
            None => Some("absent".to_string()),
            Some(b) if *blake3::hash(b).as_bytes() != f.blake3 => {
                Some("its BLAKE3 is not the index's".to_string())
            }
            Some(b) => match from_cbor_seq(b) {
                Err(e) => Some(format!("it does not decode: {e}")),
                Ok((rs, _)) if rs.len() as u64 != f.records => Some(format!(
                    "it holds {} records, and the index says {}",
                    rs.len(),
                    f.records
                )),
                Ok((rs, _)) if record_set_digest(&rs) != f.rsd => {
                    Some("its record-set digest is not the index's".to_string())
                }
                Ok((rs, _)) => {
                    records.extend(rs);
                    None
                }
            },
        };
        if let Some(p) = &problem {
            report.push(Diagnostic::new(Code::E494).with_message(format!("{}: {p}", f.path)));
        }
        checks.push(FileCheck {
            path: f.path.clone(),
            problem,
        });
    }
    if checks.iter().any(|c| c.problem.is_some()) {
        let why = "a file does not match its index";
        return VerifyReport {
            files: checks,
            p1: Property::NotApplicable(why),
            p3: Property::NotApplicable(why),
            report,
        };
    }

    let written: BTreeSet<[u8; 32]> = records.iter().map(Store::record_hash).collect();
    let imported = Store::from_records(records);
    let p1 = if imported.record_set_digest() == index.rsd {
        Property::Holds
    } else {
        let retained: BTreeSet<[u8; 32]> = imported.iter().map(Store::record_hash).collect();
        Property::Fails {
            missing: written.difference(&retained).copied().collect(),
            extra: retained.difference(&written).copied().collect(),
            first_diff: None,
        }
    };
    VerifyReport {
        files: checks,
        p1,
        p3: p3(index, files, &imported),
        report,
    }
}

/// P3: the collection exported again from what an import of it holds.
fn p3(index: &ExportIndex, files: &BTreeMap<String, Vec<u8>>, imported: &Store) -> Property {
    if index.format != "cbor" || index.derived.is_some() {
        return Property::NotApplicable("only a canonical cbor export is re-exported");
    }
    let Some(opts) = options_of(index, imported) else {
        return Property::NotApplicable("a layout or closure this build does not know");
    };
    let roots = imported
        .views()
        .filter(|v| v.id.as_str().starts_with("export/"))
        .flat_map(|v| v.roots.iter().copied());
    let fails = |first_diff| Property::Fails {
        missing: Vec::new(),
        extra: Vec::new(),
        first_diff,
    };
    let again = match export_store(imported, &Selection::of(roots), &opts) {
        Ok(c) => c,
        Err(_) => return fails(None),
    };
    for f in &index.files {
        let ours = files.get(&f.path).map(Vec::as_slice).unwrap_or_default();
        let theirs = again.file(&f.path).map(|f| f.bytes.as_slice());
        match theirs {
            None => return fails(Some((f.path.clone(), 0))),
            Some(t) if t != ours => {
                let at = ours.iter().zip(t).take_while(|(a, b)| a == b).count();
                return fails(Some((f.path.clone(), at as u64)));
            }
            Some(_) => {}
        }
    }
    if let Some(f) = again.files.iter().find(|f| index.file(&f.path).is_none()) {
        return fails(Some((f.path.clone(), 0)));
    }
    if again.index.files != index.files || again.index.rsd != index.rsd {
        return fails(None);
    }
    Property::Holds
}
