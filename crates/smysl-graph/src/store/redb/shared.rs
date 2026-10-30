//! One open database per store file, per process (SMYSL-2.8 §3.8).
//!
//! redb 2.6 locks a file with `flock` on its own descriptor, and a second open of the same file
//! in the same process conflicts with the first exactly as another process's would. So readers
//! and the writer in one process could not both hold a store, although §3.8 says a writer in
//! the same process does not block a reader — and on redb's side it does not: read
//! transactions are snapshots, and a commit leaves them where they were. What was in the way
//! was the second open.
//!
//! Hence one [`Shared`] per file, handed to every open in the process while any is alive: a
//! [`super::RedbStore`] holds it for its snapshot, a [`super::RedbShard`] for its writes, and
//! each takes its own transactions from the one database.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use ::redb::{Builder, Database, DatabaseError};
use smysl_core::diag::{Code, Diagnostic};
use smysl_core::Error;

use super::{backend, held, OpenReport, RedbOptions};

/// A database and the lock its catch-ups take.
pub(crate) struct Shared {
    pub(super) db: Database,
    /// Held from reading what the store describes to committing what it now describes, so two
    /// opens in one process cannot fold the same tail from the same place at once.
    fold: Mutex<()>,
}

impl Shared {
    pub(crate) fn new(db: Database) -> Shared {
        Shared {
            db,
            fold: Mutex::new(()),
        }
    }

    pub(super) fn folding(&self) -> MutexGuard<'_, ()> {
        self.fold.lock().unwrap_or_else(|e| e.into_inner())
    }
}

static OPEN: Mutex<BTreeMap<PathBuf, Weak<Shared>>> = Mutex::new(BTreeMap::new());

/// The open database for the store file at `path`, opening it if nothing in this process has.
///
/// Fails at once with [`held`]'s `WouldBlock` when another process has the file. A file that
/// will not open as a store is replaced (`SMY-W501`): it is derived, so it is rebuilt rather
/// than repaired.
pub(super) fn open(
    path: &Path,
    opts: &RedbOptions,
    report: &mut OpenReport,
) -> Result<Arc<Shared>, Error> {
    let key = key(path)?;
    let mut open = OPEN.lock().unwrap_or_else(|e| e.into_inner());
    let was_open = match open.get(&key) {
        Some(w) => match w.upgrade() {
            Some(s) => return Ok(s),
            None => true,
        },
        None => false,
    };
    // A store whose last holder in this process is being dropped can still hold its file for a
    // moment after the registry stops seeing it; that moment is not another process.
    let patience = if was_open {
        Duration::from_millis(200)
    } else {
        Duration::ZERO
    };
    let started = Instant::now();
    let mut builder = Builder::new();
    builder.set_cache_size(opts.page_cache_bytes);
    let db = loop {
        match builder.create(path) {
            Ok(db) => break db,
            Err(DatabaseError::DatabaseAlreadyOpen) if started.elapsed() < patience => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(DatabaseError::DatabaseAlreadyOpen) => return Err(held(path)),
            Err(e) => {
                report
                    .report
                    .push(Diagnostic::new(Code::W501).with_message(format!(
                        "the unit store would not open ({e}); rebuilding it from the log"
                    )));
                std::fs::remove_file(path).map_err(Error::Io)?;
                break builder.create(path).map_err(|e| match e {
                    DatabaseError::DatabaseAlreadyOpen => held(path),
                    other => backend(other),
                })?;
            }
        }
    };
    let shared = Arc::new(Shared::new(db));
    open.insert(key, Arc::downgrade(&shared));
    // Entries whose stores have all closed, so the map is as large as what is open.
    open.retain(|_, w| w.strong_count() > 0);
    Ok(shared)
}

/// One name for a file however it was reached: its directory resolved, its own name as given.
fn key(path: &Path) -> Result<PathBuf, Error> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let dir = std::fs::canonicalize(dir).map_err(Error::Io)?;
    Ok(match path.file_name() {
        Some(name) => dir.join(name),
        None => dir,
    })
}
