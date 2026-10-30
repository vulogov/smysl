//! The export index, `export-index.cbor` (SMYSL-2.7 §4.2).
//!
//! What a collection says about itself: each file's path, BLAKE3, record count and record-set
//! digest, the options it was cut with, and the digest of everything written, which is P1's
//! `rsd(X)`. Deterministic CBOR with sorted integer keys, shortest form. It is not a smysl
//! record and has no type envelope, so no store will ever hold one.
//!
//! Decoding is strict: every key in ascending order, no key it does not know, every digest 32
//! bytes. So `to_cbor(from_cbor(b)) == b` for every `b` that decodes, and a tampered index is
//! refused rather than read differently.

use smysl_core::cbor::{major, Dec, Enc};
use smysl_core::CodecError;
use std::collections::BTreeMap;
use std::fmt;

/// The file name of a collection's index.
pub const INDEX_FILE: &str = "export-index.cbor";

/// The index format, key 0.
pub const INDEX_VERSION: &str = "smysl/export-index/1";

/// The codes an index counts, key 15, every one of them in every index.
pub const COUNTED: [&str; 8] = [
    "SMY-W415", "SMY-W425", "SMY-W428", "SMY-W490", "SMY-W491", "SMY-W495", "SMY-W497", "SMY-W498",
];

/// What a file of a collection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// A keyed file: a generated view and what it selects.
    Data,
    /// `unattributed.cbor`: the records this build cannot interpret.
    Unattributed,
}

/// One file as the index records it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct IndexFile {
    pub path: String,
    /// BLAKE3 of the file's bytes.
    pub blake3: [u8; 32],
    pub records: u64,
    /// The record-set digest of what the file holds.
    pub rsd: [u8; 32],
    /// The generated view's id; `None` for `unattributed.cbor`.
    pub view: Option<String>,
    /// Under `ref`, the files this one names records from, by path. Empty under `full`.
    pub requires: Vec<String>,
    pub role: Role,
}

/// `export-index.cbor`, field by field (§4.2's table).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ExportIndex {
    /// The files, sorted by path.
    pub files: Vec<IndexFile>,
    /// The scope as given. Empty until `sq` scopes arrive (TX-P11 step 8).
    pub scope: String,
    pub sq_version: Option<String>,
    /// The schema declarations included, as `(schema id, version)`, sorted.
    pub schemas: Vec<(String, u32)>,
    pub lens_chain: Option<[u8; 32]>,
    pub layout: String,
    pub closure: String,
    pub format: String,
    pub as_of: Option<String>,
    pub tool: String,
    /// The record-set digest of everything written: `rsd(X)`.
    pub rsd: [u8; 32],
    /// The options that change what is written, sorted: `with-views`, `with-packs`,
    /// `unknown-drop`, `edges=…`.
    pub flags: Vec<String>,
    pub derived: Option<String>,
    /// False when the export could not write everything it selected (`SMY-E424`).
    pub complete: bool,
    /// Per code of [`COUNTED`], how many.
    pub counts: BTreeMap<String, u64>,
    /// The duplication factor's two terms: records selected summed over files, and counted
    /// once (§3.7).
    pub duplication: (u64, u64),
}

/// Why bytes are not an export index.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexError {
    /// Not deterministic CBOR, or cut short.
    Codec(CodecError),
    /// CBOR, but not this shape. The message names what was wrong.
    Shape(&'static str),
}

impl fmt::Display for IndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            IndexError::Codec(e) => write!(f, "not an export index: {e}"),
            IndexError::Shape(what) => write!(f, "not an export index: {what}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl From<CodecError> for IndexError {
    fn from(e: CodecError) -> IndexError {
        IndexError::Codec(e)
    }
}

fn text(s: &str) -> Vec<u8> {
    let mut e = Enc::new();
    e.text(s);
    e.into_bytes()
}

fn bytes(b: &[u8]) -> Vec<u8> {
    let mut e = Enc::new();
    e.bytes(b);
    e.into_bytes()
}

fn uint(n: u64) -> Vec<u8> {
    let mut e = Enc::new();
    e.uint(n);
    e.into_bytes()
}

fn texts(items: &[String]) -> Vec<u8> {
    let mut e = Enc::new();
    e.array(items.iter().map(|s| text(s)).collect());
    e.into_bytes()
}

const TRUE: u8 = 0xF5;
const FALSE: u8 = 0xF4;

impl IndexFile {
    fn to_cbor(&self) -> Vec<u8> {
        let mut m = vec![
            (0, text(&self.path)),
            (1, bytes(&self.blake3)),
            (2, uint(self.records)),
            (3, bytes(&self.rsd)),
        ];
        if let Some(v) = &self.view {
            m.push((4, text(v)));
        }
        if !self.requires.is_empty() {
            m.push((5, texts(&self.requires)));
        }
        let role = match self.role {
            Role::Data => 0,
            Role::Unattributed => 1,
        };
        m.push((6, uint(role)));
        let mut e = Enc::new();
        e.map(m);
        e.into_bytes()
    }

    fn read(d: &mut Dec<'_>) -> Result<IndexFile, IndexError> {
        let n = d.map_head()?;
        let mut f = IndexFile {
            path: String::new(),
            blake3: [0; 32],
            records: 0,
            rsd: [0; 32],
            view: None,
            requires: Vec::new(),
            role: Role::Data,
        };
        let mut seen = [false; 7];
        let mut prev = None;
        for _ in 0..n {
            let k = d.map_key(prev)?;
            prev = Some(k);
            match k {
                0 => f.path = d.text()?.to_string(),
                1 => f.blake3 = digest(d)?,
                2 => f.records = d.uint()?,
                3 => f.rsd = digest(d)?,
                4 => f.view = Some(d.text()?.to_string()),
                5 => {
                    f.requires = d.array(|d| d.text().map(str::to_string))?;
                    if f.requires.is_empty() {
                        return Err(IndexError::Shape("an empty requires is written absent"));
                    }
                }
                6 => {
                    f.role = match d.uint()? {
                        0 => Role::Data,
                        1 => Role::Unattributed,
                        _ => return Err(IndexError::Shape("a file role is 0 or 1")),
                    }
                }
                _ => return Err(IndexError::Shape("a file entry has an unknown key")),
            }
            seen[k as usize] = true;
        }
        if !(seen[0] && seen[1] && seen[2] && seen[3] && seen[6]) {
            return Err(IndexError::Shape("a file entry lacks a required key"));
        }
        Ok(f)
    }
}

fn digest(d: &mut Dec<'_>) -> Result<[u8; 32], IndexError> {
    d.bytes()?
        .try_into()
        .map_err(|_| IndexError::Shape("a digest is 32 bytes"))
}

impl ExportIndex {
    /// The index's canonical bytes.
    pub fn to_cbor(&self) -> Vec<u8> {
        let mut m = vec![
            (0, text(INDEX_VERSION)),
            (1, {
                let mut e = Enc::new();
                e.array(self.files.iter().map(IndexFile::to_cbor).collect());
                e.into_bytes()
            }),
            (2, text(&self.scope)),
        ];
        if let Some(v) = &self.sq_version {
            m.push((3, text(v)));
        }
        m.push((4, {
            let mut e = Enc::new();
            e.array(
                self.schemas
                    .iter()
                    .map(|(id, v)| {
                        let mut e = Enc::new();
                        e.array(vec![text(id), uint(u64::from(*v))]);
                        e.into_bytes()
                    })
                    .collect(),
            );
            e.into_bytes()
        }));
        if let Some(l) = &self.lens_chain {
            m.push((5, bytes(l)));
        }
        m.push((6, text(&self.layout)));
        m.push((7, text(&self.closure)));
        m.push((8, text(&self.format)));
        if let Some(a) = &self.as_of {
            m.push((9, text(a)));
        }
        m.push((10, text(&self.tool)));
        m.push((11, bytes(&self.rsd)));
        m.push((12, texts(&self.flags)));
        if let Some(d) = &self.derived {
            m.push((13, text(d)));
        }
        m.push((14, vec![if self.complete { TRUE } else { FALSE }]));
        m.push((15, {
            let mut e = Enc::new();
            e.head(major::MAP, self.counts.len() as u64);
            for (code, n) in &self.counts {
                e.text(code);
                e.uint(*n);
            }
            e.into_bytes()
        }));
        m.push((16, {
            let mut e = Enc::new();
            e.array(vec![uint(self.duplication.0), uint(self.duplication.1)]);
            e.into_bytes()
        }));
        let mut e = Enc::new();
        e.map(m);
        e.into_bytes()
    }

    /// Read an index, strictly: what decodes encodes back to the same bytes.
    pub fn from_cbor(b: &[u8]) -> Result<ExportIndex, IndexError> {
        let mut d = Dec::new(b);
        let n = d.map_head()?;
        let mut x = ExportIndex {
            files: Vec::new(),
            scope: String::new(),
            sq_version: None,
            schemas: Vec::new(),
            lens_chain: None,
            layout: String::new(),
            closure: String::new(),
            format: String::new(),
            as_of: None,
            tool: String::new(),
            rsd: [0; 32],
            flags: Vec::new(),
            derived: None,
            complete: false,
            counts: BTreeMap::new(),
            duplication: (0, 0),
        };
        let mut seen = [false; 17];
        let mut prev = None;
        for _ in 0..n {
            let k = d.map_key(prev)?;
            prev = Some(k);
            match k {
                0 => {
                    if d.text()? != INDEX_VERSION {
                        return Err(IndexError::Shape("the version is not smysl/export-index/1"));
                    }
                }
                1 => {
                    x.files = d
                        .array(|d| Ok(IndexFile::read(d)))?
                        .into_iter()
                        .collect::<Result<_, _>>()?
                }
                2 => x.scope = d.text()?.to_string(),
                3 => x.sq_version = Some(d.text()?.to_string()),
                4 => {
                    x.schemas = d
                        .array(|d| {
                            if d.array_head()? != 2 {
                                return Ok(Err(IndexError::Shape("a schema is [id, version]")));
                            }
                            let id = d.text()?.to_string();
                            let v = d.uint()?;
                            Ok(u32::try_from(v)
                                .map(|v| (id, v))
                                .map_err(|_| IndexError::Shape("a schema version is a u32")))
                        })?
                        .into_iter()
                        .collect::<Result<_, _>>()?
                }
                5 => x.lens_chain = Some(digest(&mut d)?),
                6 => x.layout = d.text()?.to_string(),
                7 => x.closure = d.text()?.to_string(),
                8 => x.format = d.text()?.to_string(),
                9 => x.as_of = Some(d.text()?.to_string()),
                10 => x.tool = d.text()?.to_string(),
                11 => x.rsd = digest(&mut d)?,
                12 => x.flags = d.array(|d| d.text().map(str::to_string))?,
                13 => x.derived = Some(d.text()?.to_string()),
                14 => {
                    x.complete = match d.peek_byte()? {
                        TRUE => true,
                        FALSE => false,
                        _ => return Err(IndexError::Shape("complete is a bool")),
                    };
                    d.advance(1);
                }
                15 => {
                    let n = d.map_head()?;
                    let mut last: Option<String> = None;
                    for _ in 0..n {
                        let code = d.text()?.to_string();
                        if last.as_ref().is_some_and(|l| *l >= code) {
                            return Err(IndexError::Shape("counts are sorted, once each"));
                        }
                        let v = d.uint()?;
                        last = Some(code.clone());
                        x.counts.insert(code, v);
                    }
                }
                16 => {
                    if d.array_head()? != 2 {
                        return Err(IndexError::Shape("duplication is two uints"));
                    }
                    x.duplication = (d.uint()?, d.uint()?);
                }
                _ => return Err(IndexError::Shape("an unknown key")),
            }
            seen[k as usize] = true;
        }
        if !d.is_done() {
            return Err(IndexError::Shape("bytes after the index"));
        }
        for k in [0, 1, 2, 4, 6, 7, 8, 10, 11, 12, 14, 15, 16] {
            if !seen[k] {
                return Err(IndexError::Shape("a required key is absent"));
            }
        }
        if x.to_cbor() != b {
            return Err(IndexError::Shape("not in canonical form"));
        }
        Ok(x)
    }

    /// The file at this path.
    pub fn file(&self, path: &str) -> Option<&IndexFile> {
        self.files.iter().find(|f| f.path == path)
    }
}
