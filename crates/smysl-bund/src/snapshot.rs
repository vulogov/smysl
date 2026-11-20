//! The snapshot: a fresh library in a private directory, which the process sees instead of
//! the caller's (RFC SMYSL-4 §4).
//!
//! It is written, not copied. A copy would carry what `bund2`'s smysl 1.15 cannot read: a
//! primary shard on redb is refused with `SM02`. It would also carry lock files and index
//! sidecars, which writing leaves out by construction. And inside the copy, 1.15's open may
//! unlink the objects of an interrupted redaction (`enforce_z`) without harm.

use std::path::{Path, PathBuf};

use smysl_graph::Store;
use smysl_text::Library;

use crate::error::BundError;

/// A snapshot and the directory that holds it. Dropping it removes both.
pub struct Snapshot {
    dir: tempfile::TempDir,
    root: PathBuf,
}

impl Snapshot {
    /// The private directory: the process's working directory, `HOME` and `TMPDIR`.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The snapshot library's root, as the preamble names it.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// From a store's records, in store order (§4.2).
    pub fn of_store(store: &Store) -> Result<Snapshot, BundError> {
        let snap = Snapshot::empty()?;
        let mut lib = Library::create(&snap.root).map_err(lib_err)?;
        let records: Vec<_> = store.iter().cloned().collect();
        if !records.is_empty() {
            lib.append_records(&records).map_err(lib_err)?;
        }
        Ok(snap)
    }

    /// From a library on disk (§4.3). The live library is opened to read, never locked, and
    /// its directory is not handed to the process.
    pub fn of_library(live: &Path) -> Result<(Snapshot, Store), BundError> {
        let source = Library::open(live).map_err(lib_err)?;
        let snap = Snapshot::of_store(source.store())?;
        // Part texts and readings. `Library::open` completed any interrupted redaction before
        // returning, so what is under `objects/` is what the catalog says is there.
        for kind in ["t3", "r3"] {
            let from = live.join("objects").join(kind);
            if from.is_dir() {
                copy_tree(&from, &snap.root.join("objects").join(kind))?;
            }
        }
        Ok((snap, source.store().clone()))
    }

    fn empty() -> Result<Snapshot, BundError> {
        let dir = tempfile::Builder::new()
            .prefix("smysl-bund-")
            .tempdir()
            .map_err(|e| BundError::IoError {
                message: format!("the snapshot directory: {e}"),
            })?;
        // The canonical path: on macOS the temporary directory is reached through a symlink,
        // and the preamble names the root as `bund2` will see it.
        let base = dir.path().canonicalize().map_err(|e| BundError::IoError {
            message: format!("the snapshot directory: {e}"),
        })?;
        let root = base.join("lib");
        Ok(Snapshot { dir, root })
    }
}

fn lib_err(e: smysl_core::error::LibError) -> BundError {
    BundError::IoError {
        message: e.to_string(),
    }
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), BundError> {
    let io = |what: &Path, e: std::io::Error| BundError::IoError {
        message: format!("copying {}: {e}", what.display()),
    };
    std::fs::create_dir_all(to).map_err(|e| io(to, e))?;
    for entry in std::fs::read_dir(from).map_err(|e| io(from, e))? {
        let entry = entry.map_err(|e| io(from, e))?;
        let kind = entry.file_type().map_err(|e| io(&entry.path(), e))?;
        let target = to.join(entry.file_name());
        if kind.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target).map_err(|e| io(&entry.path(), e))?;
        }
        // A symlink in an object store is not one smysl wrote, and is not followed.
    }
    Ok(())
}
