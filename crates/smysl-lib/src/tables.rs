//! The import tables (SMYSL-2.7 §4.1), in their CBOR-file fallback: `lib/tables/*.cbor`.
//!
//! - `imported`: per record hash, every `(mid, state)` it was imported under. Operational
//!   state, not derived (draft 3 §16.3): what a library retained of each file, which is P1's
//!   import side.
//! - `import_batches`: per import name, its manifests, its state and its counts. `lib status`.
//! - `waiting`: per record hash, the import it belongs to and the ids it waits for.
//!
//! §4.1 keys `waiting` by missing id and keeps `waiting_by_record` beside it. The fallback
//! stores the second, which holds everything, and builds the first in memory when promotion
//! asks. The substrate index (SMYSL-2.4 §3.8) is where these move, keyed as §4.1 gives them.
//!
//! Each file is deterministic CBOR, entries sorted by key, so the same imports in any order
//! write the same bytes.

use crate::export::LibError;
use crate::import::classify::Id;
use smysl_core::cbor::{Dec, Enc};
use smysl_core::{ContentionId, Mid, Tid, Uid};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What became of a record under one manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum State {
    Accepted,
    Staged,
    Waiting,
    Quarantined,
    Discarded,
}

impl State {
    const fn code(self) -> u64 {
        match self {
            State::Accepted => 0,
            State::Staged => 1,
            State::Waiting => 2,
            State::Quarantined => 3,
            State::Discarded => 4,
        }
    }

    fn from_code(c: u64) -> Option<State> {
        Some(match c {
            0 => State::Accepted,
            1 => State::Staged,
            2 => State::Waiting,
            3 => State::Quarantined,
            4 => State::Discarded,
            _ => return None,
        })
    }

    /// The state's name, as `lib status` prints it.
    pub const fn as_str(self) -> &'static str {
        match self {
            State::Accepted => "accepted",
            State::Staged => "staged",
            State::Waiting => "waiting",
            State::Quarantined => "quarantined",
            State::Discarded => "discarded",
        }
    }
}

/// One import: what `lib status` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Batch {
    pub name: String,
    /// The manifests it wrote, one per file, sorted.
    pub mids: Vec<Mid>,
    /// `Accepted`, `Staged` or `Discarded`: what happened to the records that were neither
    /// waiting nor quarantined.
    pub state: State,
    pub accepted: u64,
    pub staged: u64,
    pub waiting: u64,
    pub quarantined: u64,
    /// The tool that imported it.
    pub tool: String,
}

/// A record held waiting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waiting {
    /// The import it arrived with.
    pub name: String,
    /// What it waits for.
    pub missing: BTreeSet<Id>,
}

/// The three tables, loaded whole.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tables {
    pub imported: BTreeMap<[u8; 32], BTreeSet<(Mid, State)>>,
    pub batches: BTreeMap<String, Batch>,
    pub waiting: BTreeMap<[u8; 32], Waiting>,
}

fn dir(root: &Path) -> PathBuf {
    root.join("lib").join("tables")
}

fn io(p: &Path, e: std::io::Error) -> LibError {
    LibError::Io(p.display().to_string(), e.to_string())
}

fn corrupt(p: &Path, e: impl std::fmt::Display) -> LibError {
    LibError::Io(p.display().to_string(), format!("not an import table: {e}"))
}

fn read(p: &Path) -> Result<Option<Vec<u8>>, LibError> {
    match std::fs::read(p) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(io(p, e)),
    }
}

/// Write by rename, so a table is never half-written.
pub(crate) fn write_atomic(p: &Path, bytes: &[u8]) -> Result<(), LibError> {
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io(parent, e))?;
    }
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| io(&tmp, e))?;
    std::fs::rename(&tmp, p).map_err(|e| io(p, e))
}

fn enc(f: impl FnOnce(&mut Enc)) -> Vec<u8> {
    let mut e = Enc::new();
    f(&mut e);
    e.into_bytes()
}

fn id_bytes(i: &Id) -> Vec<u8> {
    enc(|e| {
        e.array_head(2);
        e.uint(u64::from(i.tag()));
        match i {
            Id::Uid(u) => e.bytes(u.as_bytes()),
            Id::Tid(t) => e.bytes(t.as_bytes()),
            Id::Mid(m) => e.bytes(m.as_bytes()),
            Id::Contention(k) => e.text(k.as_str()),
        }
    })
}

type Res<T> = Result<T, String>;

fn digest(d: &mut Dec<'_>) -> Res<[u8; 32]> {
    d.bytes()
        .map_err(|e| e.to_string())?
        .try_into()
        .map_err(|_| "a digest is 32 bytes".to_string())
}

fn read_id(d: &mut Dec<'_>) -> Res<Id> {
    let s = |e: smysl_core::CodecError| e.to_string();
    if d.array_head().map_err(s)? != 2 {
        return Err("an id is [tag, value]".into());
    }
    Ok(match d.uint().map_err(s)? {
        0x01 => Id::Uid(Uid::from_bytes(digest(d)?)),
        0x0F => Id::Tid(Tid::from_bytes(digest(d)?)),
        0x0E => Id::Mid(Mid::from_bytes(digest(d)?)),
        0x03 => Id::Contention(ContentionId::new(d.text().map_err(s)?).map_err(|e| e.to_string())?),
        t => return Err(format!("an id tag {t} this build does not know")),
    })
}

impl Tables {
    /// The tables of the library at `root`; empty when it has none yet.
    pub fn load(root: &Path) -> Result<Tables, LibError> {
        let d = dir(root);
        let mut t = Tables::default();
        let s = |e: smysl_core::CodecError| e.to_string();

        let p = d.join("imported.cbor");
        if let Some(b) = read(&p)? {
            let mut dec = Dec::new(&b);
            let rows = (|| -> Res<_> {
                let n = dec.array_head().map_err(s)?;
                let mut out = BTreeMap::new();
                for _ in 0..n {
                    if dec.array_head().map_err(s)? != 2 {
                        return Err("a row is [hash, states]".into());
                    }
                    let h = digest(&mut dec)?;
                    let m = dec.array_head().map_err(s)?;
                    let mut set = BTreeSet::new();
                    for _ in 0..m {
                        if dec.array_head().map_err(s)? != 2 {
                            return Err("a state is [mid, code]".into());
                        }
                        let mid = Mid::from_bytes(digest(&mut dec)?);
                        let st = State::from_code(dec.uint().map_err(s)?)
                            .ok_or("a state code this build does not know")?;
                        set.insert((mid, st));
                    }
                    out.insert(h, set);
                }
                Ok(out)
            })()
            .map_err(|e| corrupt(&p, e))?;
            t.imported = rows;
        }

        let p = d.join("batches.cbor");
        if let Some(b) = read(&p)? {
            let mut dec = Dec::new(&b);
            t.batches = (|| -> Res<_> {
                let n = dec.array_head().map_err(s)?;
                let mut out = BTreeMap::new();
                for _ in 0..n {
                    if dec.array_head().map_err(s)? != 8 {
                        return Err("a batch has eight fields".into());
                    }
                    let name = dec.text().map_err(s)?.to_string();
                    let m = dec.array_head().map_err(s)?;
                    let mut mids = Vec::new();
                    for _ in 0..m {
                        mids.push(Mid::from_bytes(digest(&mut dec)?));
                    }
                    let state = State::from_code(dec.uint().map_err(s)?)
                        .ok_or("a state code this build does not know")?;
                    let accepted = dec.uint().map_err(s)?;
                    let staged = dec.uint().map_err(s)?;
                    let waiting = dec.uint().map_err(s)?;
                    let quarantined = dec.uint().map_err(s)?;
                    let tool = dec.text().map_err(s)?.to_string();
                    out.insert(
                        name.clone(),
                        Batch {
                            name,
                            mids,
                            state,
                            accepted,
                            staged,
                            waiting,
                            quarantined,
                            tool,
                        },
                    );
                }
                Ok(out)
            })()
            .map_err(|e| corrupt(&p, e))?;
        }

        let p = d.join("waiting.cbor");
        if let Some(b) = read(&p)? {
            let mut dec = Dec::new(&b);
            t.waiting = (|| -> Res<_> {
                let n = dec.array_head().map_err(s)?;
                let mut out = BTreeMap::new();
                for _ in 0..n {
                    if dec.array_head().map_err(s)? != 3 {
                        return Err("a waiting row is [hash, name, missing]".into());
                    }
                    let h = digest(&mut dec)?;
                    let name = dec.text().map_err(s)?.to_string();
                    let m = dec.array_head().map_err(s)?;
                    let mut missing = BTreeSet::new();
                    for _ in 0..m {
                        missing.insert(read_id(&mut dec)?);
                    }
                    out.insert(h, Waiting { name, missing });
                }
                Ok(out)
            })()
            .map_err(|e| corrupt(&p, e))?;
        }
        Ok(t)
    }

    /// Write the three tables.
    pub fn save(&self, root: &Path) -> Result<(), LibError> {
        let d = dir(root);
        let imported = enc(|e| {
            e.array_head(self.imported.len());
            for (h, set) in &self.imported {
                e.array_head(2);
                e.bytes(h);
                e.array_head(set.len());
                for (mid, st) in set {
                    e.array_head(2);
                    e.bytes(mid.as_bytes());
                    e.uint(st.code());
                }
            }
        });
        write_atomic(&d.join("imported.cbor"), &imported)?;
        let batches = enc(|e| {
            e.array_head(self.batches.len());
            for b in self.batches.values() {
                e.array_head(8);
                e.text(&b.name);
                e.array_head(b.mids.len());
                for m in &b.mids {
                    e.bytes(m.as_bytes());
                }
                e.uint(b.state.code());
                for n in [b.accepted, b.staged, b.waiting, b.quarantined] {
                    e.uint(n);
                }
                e.text(&b.tool);
            }
        });
        write_atomic(&d.join("batches.cbor"), &batches)?;
        let waiting = enc(|e| {
            e.array_head(self.waiting.len());
            for (h, w) in &self.waiting {
                e.array_head(3);
                e.bytes(h);
                e.text(&w.name);
                e.array(w.missing.iter().map(id_bytes).collect());
            }
        });
        write_atomic(&d.join("waiting.cbor"), &waiting)
    }

    /// Set the state of `h` under every mid of `mids` it was imported under.
    pub(crate) fn set_state(&mut self, h: &[u8; 32], mids: &BTreeSet<Mid>, to: State) {
        if let Some(set) = self.imported.get_mut(h) {
            let moved: Vec<Mid> = set
                .iter()
                .filter(|(m, _)| mids.contains(m))
                .map(|(m, _)| *m)
                .collect();
            set.retain(|(m, _)| !mids.contains(m));
            set.extend(moved.into_iter().map(|m| (m, to)));
        }
    }
}
