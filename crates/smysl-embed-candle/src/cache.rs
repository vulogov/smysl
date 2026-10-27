//! The vector cache: `(model hash, uid) → vector`, in a sidecar beside a store.
//!
//! Until SMYSL-2.4's `vectors` table exists, a store's vectors live in `<store>.vec-<hash12>`,
//! a CBOR sequence: a header `["smysl-vec/1", model hash, dim]`, then one `[uid, vector]` a
//! unit, in uid order, the vector a byte string of `dim` little-endian `f32`s. Bytes rather
//! than CBOR floats because the format's floats are quantised to 1/1024 (constraint 4), and
//! a cached vector has to be the one the encoder returned, bit for bit, or the cache is not
//! standing in for the encoder.
//!
//! A cache is derived and never identity: deleting it loses nothing but the time to embed
//! again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::cbor::{Dec, Enc};
use smysl_core::diag::{Code, Diagnostic};
use smysl_core::Uid;

use crate::encoder::Encoder;
use crate::Error;

const MAGIC: &str = "smysl-vec/1";

/// Vectors one encoder computed, by uid.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorCache {
    hash: [u8; 32],
    dim: usize,
    vectors: BTreeMap<Uid, Vec<f32>>,
}

impl VectorCache {
    /// An empty cache for `encoder`.
    pub fn new(encoder: &Encoder) -> VectorCache {
        VectorCache {
            hash: encoder.model_hash(),
            dim: encoder.dim(),
            vectors: BTreeMap::new(),
        }
    }

    /// The sidecar for a store at `store` and a model hash: `<store>.vec-<first 12 hex>`.
    /// A model of its own has a file of its own, so changing models leaves the other's
    /// vectors where they were rather than overwriting them.
    pub fn sidecar(store: impl AsRef<Path>, model_hash: &[u8; 32]) -> PathBuf {
        let mut name = store.as_ref().as_os_str().to_owned();
        name.push(".vec-");
        name.push(hex12(model_hash));
        PathBuf::from(name)
    }

    /// Read the cache at `path` for `encoder`.
    ///
    /// A file that is not there is an empty cache and no diagnostic: nothing has been
    /// embedded yet. A file written under another model hash — a model replaced in its
    /// directory, its prefixes changed, a sidecar copied by hand — is **ignored**, every
    /// vector in it to be embedded again, and said so with `SMY-W476` and a count. A file
    /// that does not decode is an error: it is not this crate's to delete.
    pub fn open(
        path: impl AsRef<Path>,
        encoder: &Encoder,
    ) -> Result<(VectorCache, Vec<Diagnostic>), Error> {
        let path = path.as_ref();
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((VectorCache::new(encoder), Vec::new()))
            }
            Err(e) => return Err(Error::Cache(format!("{}: {e}", path.display()))),
        };
        let bad = |e: String| Error::Cache(format!("{}: {e}", path.display()));
        let (hash, dim, vectors) = decode(&bytes).map_err(bad)?;
        let mut diags = Vec::new();
        if hash != encoder.model_hash() || dim != encoder.dim() {
            diags.push(Diagnostic::new(Code::W476).with_message(format!(
                "{} cached vector(s) in {} are model {}'s and this encoder is {}; ignored, to \
                 be embedded again",
                vectors.len(),
                path.display(),
                hex12(&hash),
                hex12(&encoder.model_hash()),
            )));
            return Ok((VectorCache::new(encoder), diags));
        }
        Ok((VectorCache { hash, dim, vectors }, diags))
    }

    /// Write the cache to `path`, through a temporary file renamed into place, so a reader
    /// never sees half of one.
    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        let path = path.as_ref();
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        let io = |e: std::io::Error| Error::Cache(format!("{}: {e}", path.display()));
        std::fs::write(&tmp, self.encode()).map_err(io)?;
        std::fs::rename(&tmp, path).map_err(io)
    }

    pub fn model_hash(&self) -> [u8; 32] {
        self.hash
    }

    pub fn get(&self, uid: &Uid) -> Option<&[f32]> {
        self.vectors.get(uid).map(Vec::as_slice)
    }

    pub fn len(&self) -> usize {
        self.vectors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vectors.is_empty()
    }

    pub(crate) fn insert(&mut self, uid: Uid, v: Vec<f32>) {
        self.vectors.insert(uid, v);
    }

    /// Keep only the uids `keep` admits — a store's units, so a unit no longer there does
    /// not stay in its sidecar for ever.
    pub(crate) fn retain(&mut self, keep: impl Fn(&Uid) -> bool) {
        self.vectors.retain(|u, _| keep(u));
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = Enc::with_capacity(48 + self.vectors.len() * (40 + 4 * self.dim));
        e.array_head(3);
        e.text(MAGIC);
        e.bytes(&self.hash);
        e.uint(self.dim as u64);
        for (uid, v) in &self.vectors {
            e.array_head(2);
            e.uid(uid);
            let raw: Vec<u8> = v.iter().flat_map(|x| x.to_le_bytes()).collect();
            e.bytes(&raw);
        }
        e.into_bytes()
    }
}

type Decoded = ([u8; 32], usize, BTreeMap<Uid, Vec<f32>>);

fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    let mut d = Dec::new(bytes);
    let s = |e: smysl_core::error::CodecError| e.to_string();
    if d.array_head().map_err(s)? != 3 || d.text().map_err(s)? != MAGIC {
        return Err(format!("not a {MAGIC} sidecar"));
    }
    let hash: [u8; 32] = d
        .bytes()
        .map_err(s)?
        .try_into()
        .map_err(|_| "the model hash is not 32 bytes".to_string())?;
    let dim = d.uint().map_err(s)? as usize;
    let mut vectors = BTreeMap::new();
    while !d.is_done() {
        if d.array_head().map_err(s)? != 2 {
            return Err("an entry is not [uid, vector]".into());
        }
        let uid = d.uid().map_err(s)?;
        let raw = d.bytes().map_err(s)?;
        if raw.len() != 4 * dim {
            return Err(format!(
                "a vector of {} bytes where {dim} f32s were declared",
                raw.len()
            ));
        }
        let v = raw
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c))
            .collect();
        vectors.insert(uid, v);
    }
    Ok((hash, dim, vectors))
}

fn hex12(hash: &[u8; 32]) -> String {
    hash[..6].iter().map(|b| format!("{b:02x}")).collect()
}
