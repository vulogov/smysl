//! Inputs and decoding (SMYSL-2.7 §3.1, §3.2 step 1).
//!
//! An input is a file, a directory of files (not recursive, in byte order of name), an export
//! index (the collection whole, its files verified against it first), or bytes a caller holds.
//! Each file is one import unit. A file is surface when it reads as surface, as `smysl check`
//! decides; otherwise it is a CBOR sequence that must decode to its last byte. Anything else
//! is `SMY-E493`, and nothing of that file is used.

use super::Input;
use crate::export::{slug, LibError};
use crate::index::ExportIndex;
use crate::verify::verify_files;
use smysl_core::surface::parse_surface;
use smysl_core::{from_cbor_seq, Code, Diagnostic, Record, Report};
use std::collections::BTreeMap;
use std::path::Path;

/// One file, read.
pub(crate) struct Piece {
    /// The name its manifest records: the basename, or the path an index gives. Never a
    /// directory, so one file imported on two machines has one mid (§3.1).
    pub file: String,
    /// The file stem slugged: the alias suffix when an import holds several files.
    pub stem: String,
    pub bytes: Vec<u8>,
}

fn io(p: &Path, e: std::io::Error) -> LibError {
    LibError::Io(p.display().to_string(), e.to_string())
}

fn base(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A file name's stem, slugged as a layout key is (§3.4), without the hash suffix the
/// alias does not need: `F6-quarantine.smy` is `f6-quarantine`.
pub(crate) fn stem(file: &str) -> String {
    let s = file.split('/').next_back().unwrap_or(file);
    let s = s.strip_suffix(".cbor").unwrap_or(s);
    let s = match s.rsplit_once('.') {
        Some((a, _)) if !a.is_empty() => a,
        _ => s,
    };
    slug(s).0
}

/// What the inputs hold, file by file, and the name the import takes when none is given.
pub(crate) fn read(
    inputs: &[Input],
    report: &mut Report,
) -> Result<(Vec<Piece>, String), LibError> {
    let mut pieces = Vec::new();
    let mut name = None;
    for input in inputs {
        match input {
            Input::File(p) => {
                let file = base(p);
                name.get_or_insert_with(|| stem(&file));
                let bytes = std::fs::read(p).map_err(|e| io(p, e))?;
                pieces.push(Piece {
                    stem: stem(&file),
                    file,
                    bytes,
                });
            }
            Input::Bytes { name: n, bytes } => {
                name.get_or_insert_with(|| stem(n));
                pieces.push(Piece {
                    file: n.clone(),
                    stem: stem(n),
                    bytes: bytes.clone(),
                });
            }
            Input::Dir(d) => {
                let collection = slug(&base(d)).0;
                name.get_or_insert_with(|| collection.clone());
                let mut files: Vec<_> = std::fs::read_dir(d)
                    .map_err(|e| io(d, e))?
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.path())
                    .collect();
                files.sort_by(|a, b| base(a).as_bytes().cmp(base(b).as_bytes()));
                for p in files {
                    let file = base(&p);
                    let bytes = std::fs::read(&p).map_err(|e| io(&p, e))?;
                    pieces.push(Piece {
                        stem: stem(&file),
                        file,
                        bytes,
                    });
                }
            }
            Input::Index(p) => {
                let dir = p.parent().unwrap_or(Path::new("."));
                let collection = slug(&base(dir)).0;
                name.get_or_insert_with(|| collection.clone());
                let bytes = std::fs::read(p).map_err(|e| io(p, e))?;
                let index = ExportIndex::from_cbor(&bytes).map_err(LibError::Index)?;
                let mut files = BTreeMap::new();
                for f in &index.files {
                    if let Ok(b) = std::fs::read(dir.join(&f.path)) {
                        files.insert(f.path.clone(), b);
                    }
                }
                // The files against the index only: P1 and P3 are `verify`'s, and an import
                // refuses what does not match rather than what does not round-trip.
                let checked = verify_files(&index, &files);
                if checked.files.iter().any(|c| c.problem.is_some()) {
                    report.extend(checked.report.iter().cloned());
                    continue;
                }
                for f in &index.files {
                    pieces.push(Piece {
                        file: f.path.clone(),
                        stem: stem(&f.path),
                        bytes: files.remove(&f.path).expect("checked above"),
                    });
                }
            }
        }
    }
    Ok((pieces, name.unwrap_or_else(|| "import".to_string())))
}

fn looks_like_surface(bytes: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(bytes) else {
        return false;
    };
    match s
        .lines()
        .find(|l| !(l.trim().is_empty() || l.starts_with('#') || l.starts_with("//")))
    {
        None => true,
        Some(l) => l.starts_with('@'),
    }
}

/// A file's records, or `SMY-E493` saying why there are none.
pub(crate) fn decode(file: &str, bytes: &[u8]) -> Result<Vec<Record>, Diagnostic> {
    let refuse = |why: String| Diagnostic::new(Code::E493).with_message(format!("{file}: {why}"));
    if looks_like_surface(bytes) {
        let src = std::str::from_utf8(bytes).expect("checked by looks_like_surface");
        return parse_surface(src)
            .map(|o| o.records)
            .map_err(|e| refuse(e.to_string()));
    }
    match from_cbor_seq(bytes) {
        Ok((records, used)) if used == bytes.len() => Ok(records),
        Ok((_, used)) => Err(refuse(format!(
            "cut short: {used} of {} bytes decode",
            bytes.len()
        ))),
        Err(e) => Err(refuse(e.to_string())),
    }
}
