//! The derived index sidecar (§7.3).
//!
//! **Never authoritative.** The log is the truth; this is a cache of offsets and
//! adjacency so that opening a store does not mean re-deriving everything. On open an
//! implementation compares `log_len` and `log_hash` against the log and rebuilds on
//! mismatch (`SMY-W110`) rather than failing.
//!
//! The serialised form is deterministic - big-endian, every collection sorted - because
//! the SM-P3 gate is that an index rebuilt from the log alone is **byte-identical** to
//! the one maintained incrementally. Anything else would mean the two paths disagree
//! about the graph, which is the sort of divergence that only shows up under load.

use std::collections::BTreeMap;

use smysl_core::{AgentId, Label, Status, ThreadId, Uid};

use crate::adjacency::EdgeKind;

/// File magic. Eight bytes so the header is aligned and greppable.
pub const MAGIC: &[u8; 8] = b"SMYSLIDX";
/// Sidecar format version. Independent of the wire format: the index is derived, so
/// bumping this costs a rebuild and nothing else.
pub const VERSION: u16 = 1;

/// Where a record lives in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Entry {
    pub offset: u64,
    pub len: u32,
    /// Envelope type code (Appendix B).
    pub type_code: u8,
}

/// The cached per-unit facts that traversals ask for constantly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Cached {
    pub status: Status,
    /// Salience quantised to 1/1024, so the cache is exact rather than approximate.
    pub salience_q: u16,
}

/// The derived index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Index {
    /// Bytes of the log this index describes.
    pub log_len: u64,
    /// BLAKE3 of those bytes.
    pub log_hash: [u8; 32],
    pub entries: BTreeMap<Uid, Entry>,
    pub fwd_adj: BTreeMap<Uid, Vec<(EdgeKind, Uid)>>,
    pub rev_adj: BTreeMap<Uid, Vec<(EdgeKind, Uid)>>,
    pub labels: BTreeMap<Label, Uid>,
    pub threads: BTreeMap<(ThreadId, AgentId), u64>,
    pub contentions: Vec<u64>,
    pub cache: BTreeMap<Uid, Cached>,
}

/// Why a sidecar could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IndexError {
    BadMagic,
    UnsupportedVersion(u16),
    Truncated,
    Malformed(&'static str),
}

impl core::fmt::Display for IndexError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            IndexError::BadMagic => f.write_str("not a smysl index"),
            IndexError::UnsupportedVersion(v) => write!(f, "index version {v} is not supported"),
            IndexError::Truncated => f.write_str("index ends mid-record"),
            IndexError::Malformed(what) => write!(f, "malformed index: {what}"),
        }
    }
}

impl std::error::Error for IndexError {}

impl Index {
    /// Whether this sidecar describes the log it was opened beside.
    ///
    /// A stale index is a rebuild, never a failure: the log is authoritative and the
    /// index can always be reconstructed from it.
    pub fn matches(&self, log_len: u64, log_hash: &[u8; 32]) -> bool {
        self.log_len == log_len && self.log_hash == *log_hash
    }

    pub fn unit_count(&self) -> usize {
        self.cache.len()
    }

    // -- encoding ----------------------------------------------------------

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.raw(MAGIC);
        w.u16(VERSION);
        w.u64(self.log_len);
        w.raw(&self.log_hash);

        w.u32(self.entries.len() as u32);
        for (uid, e) in &self.entries {
            w.raw(uid.as_bytes());
            w.u64(e.offset);
            w.u32(e.len);
            w.u8(e.type_code);
        }

        for adj in [&self.fwd_adj, &self.rev_adj] {
            w.u32(adj.len() as u32);
            for (uid, edges) in adj {
                w.raw(uid.as_bytes());
                w.u32(edges.len() as u32);
                for (kind, target) in edges {
                    w.u16(kind.code());
                    w.raw(target.as_bytes());
                }
            }
        }

        w.u32(self.labels.len() as u32);
        for (l, uid) in &self.labels {
            w.text(l.as_str());
            w.raw(uid.as_bytes());
        }

        w.u32(self.threads.len() as u32);
        for ((id, owner), offset) in &self.threads {
            w.text(id.as_str());
            w.text(owner.as_str());
            w.u64(*offset);
        }

        w.u32(self.contentions.len() as u32);
        for o in &self.contentions {
            w.u64(*o);
        }

        w.u32(self.cache.len() as u32);
        for (uid, c) in &self.cache {
            w.raw(uid.as_bytes());
            w.u8(c.status.as_u8());
            w.u16(c.salience_q);
        }

        w.into_bytes()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Index, IndexError> {
        let mut r = Reader::new(b);
        if r.take(8)? != MAGIC {
            return Err(IndexError::BadMagic);
        }
        let version = r.u16()?;
        if version != VERSION {
            return Err(IndexError::UnsupportedVersion(version));
        }
        let log_len = r.u64()?;
        let log_hash: [u8; 32] = r.take(32)?.try_into().map_err(|_| IndexError::Truncated)?;

        let mut entries = BTreeMap::new();
        for _ in 0..r.u32()? {
            let uid = r.uid()?;
            entries.insert(
                uid,
                Entry {
                    offset: r.u64()?,
                    len: r.u32()?,
                    type_code: r.u8()?,
                },
            );
        }

        let mut adjs = Vec::new();
        for _ in 0..2 {
            let mut adj: BTreeMap<Uid, Vec<(EdgeKind, Uid)>> = BTreeMap::new();
            for _ in 0..r.u32()? {
                let uid = r.uid()?;
                let mut edges = Vec::new();
                for _ in 0..r.u32()? {
                    let kind =
                        EdgeKind::from_code(r.u16()?).ok_or(IndexError::Malformed("edge kind"))?;
                    edges.push((kind, r.uid()?));
                }
                adj.insert(uid, edges);
            }
            adjs.push(adj);
        }
        let rev_adj = adjs.pop().expect("two");
        let fwd_adj = adjs.pop().expect("two");

        let mut labels = BTreeMap::new();
        for _ in 0..r.u32()? {
            let l = Label::new(r.text()?).map_err(|_| IndexError::Malformed("label"))?;
            labels.insert(l, r.uid()?);
        }

        let mut threads = BTreeMap::new();
        for _ in 0..r.u32()? {
            let id = ThreadId::new(r.text()?).map_err(|_| IndexError::Malformed("thread id"))?;
            let owner = AgentId::new(r.text()?).map_err(|_| IndexError::Malformed("agent id"))?;
            threads.insert((id, owner), r.u64()?);
        }

        let mut contentions = Vec::new();
        for _ in 0..r.u32()? {
            contentions.push(r.u64()?);
        }

        let mut cache = BTreeMap::new();
        for _ in 0..r.u32()? {
            let uid = r.uid()?;
            let status = Status::from_u8(r.u8()?).ok_or(IndexError::Malformed("status"))?;
            cache.insert(
                uid,
                Cached {
                    status,
                    salience_q: r.u16()?,
                },
            );
        }

        Ok(Index {
            log_len,
            log_hash,
            entries,
            fwd_adj,
            rev_adj,
            labels,
            threads,
            contentions,
            cache,
        })
    }
}

// ---------------------------------------------------------------------------
// Fixed-width big-endian helpers
// ---------------------------------------------------------------------------

struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    fn new() -> Writer {
        Writer { buf: Vec::new() }
    }
    fn into_bytes(self) -> Vec<u8> {
        self.buf
    }
    fn raw(&mut self, b: &[u8]) {
        self.buf.extend_from_slice(b);
    }
    fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }
    fn text(&mut self, s: &str) {
        self.u16(s.len() as u16);
        self.raw(s.as_bytes());
    }
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Reader<'a> {
        Reader { b, i: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], IndexError> {
        let end = self.i.checked_add(n).ok_or(IndexError::Truncated)?;
        let s = self.b.get(self.i..end).ok_or(IndexError::Truncated)?;
        self.i = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, IndexError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, IndexError> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32, IndexError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn u64(&mut self) -> Result<u64, IndexError> {
        let s = self.take(8)?;
        Ok(u64::from_be_bytes(s.try_into().expect("8 bytes")))
    }
    fn uid(&mut self) -> Result<Uid, IndexError> {
        Ok(Uid::from_bytes(
            self.take(32)?
                .try_into()
                .map_err(|_| IndexError::Truncated)?,
        ))
    }
    fn text(&mut self) -> Result<&'a str, IndexError> {
        let n = self.u16()? as usize;
        core::str::from_utf8(self.take(n)?).map_err(|_| IndexError::Malformed("text"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn sample() -> Index {
        let mut ix = Index {
            log_len: 4096,
            log_hash: [7; 32],
            ..Index::default()
        };
        ix.entries.insert(
            uid(1),
            Entry {
                offset: 0,
                len: 42,
                type_code: 1,
            },
        );
        ix.entries.insert(
            uid(2),
            Entry {
                offset: 42,
                len: 17,
                type_code: 3,
            },
        );
        ix.fwd_adj.insert(uid(2), vec![(EdgeKind::Grounds, uid(1))]);
        ix.rev_adj.insert(uid(1), vec![(EdgeKind::Grounds, uid(2))]);
        ix.labels.insert(Label::new("c/x").unwrap(), uid(1));
        ix.threads.insert(
            (
                ThreadId::new("t/brief").unwrap(),
                AgentId::new("human:vladimir").unwrap(),
            ),
            59,
        );
        ix.contentions.push(101);
        ix.cache.insert(
            uid(1),
            Cached {
                status: Status::Measured,
                salience_q: 614,
            },
        );
        ix
    }

    #[test]
    fn round_trips_through_bytes() {
        let ix = sample();
        assert_eq!(Index::from_bytes(&ix.to_bytes()).unwrap(), ix);
    }

    #[test]
    fn an_empty_index_round_trips() {
        let ix = Index::default();
        assert_eq!(Index::from_bytes(&ix.to_bytes()).unwrap(), ix);
    }

    /// The SM-P3 gate rests on this: the same index must always produce the same bytes,
    /// or "byte-identical rebuild" would be untestable.
    #[test]
    fn encoding_is_deterministic() {
        let ix = sample();
        assert_eq!(ix.to_bytes(), ix.to_bytes());
        assert_eq!(sample().to_bytes(), sample().to_bytes());
    }

    #[test]
    fn the_header_carries_the_magic_and_version() {
        let b = sample().to_bytes();
        assert_eq!(&b[..8], MAGIC);
        assert_eq!(u16::from_be_bytes([b[8], b[9]]), VERSION);
    }

    #[test]
    fn a_foreign_file_is_rejected_by_magic() {
        assert_eq!(
            Index::from_bytes(b"NOTANIDX0000").unwrap_err(),
            IndexError::BadMagic
        );
    }

    #[test]
    fn a_future_version_is_rejected_rather_than_guessed_at() {
        let mut b = sample().to_bytes();
        b[9] = 99;
        assert_eq!(
            Index::from_bytes(&b).unwrap_err(),
            IndexError::UnsupportedVersion(99)
        );
    }

    #[test]
    fn truncation_is_detected_at_every_length() {
        let b = sample().to_bytes();
        for cut in 0..b.len() {
            assert!(
                Index::from_bytes(&b[..cut]).is_err(),
                "cut {cut} was accepted"
            );
        }
        assert!(Index::from_bytes(&b).is_ok());
    }

    #[test]
    fn staleness_is_detected_by_length_or_hash() {
        let ix = sample();
        assert!(ix.matches(4096, &[7; 32]));
        assert!(!ix.matches(4097, &[7; 32]), "a longer log is a stale index");
        assert!(
            !ix.matches(4096, &[8; 32]),
            "different bytes are a stale index"
        );
    }

    #[test]
    fn extension_edge_kinds_survive_the_sidecar() {
        let mut ix = Index::default();
        ix.fwd_adj
            .insert(uid(1), vec![(EdgeKind::Extension(3), uid(2))]);
        let back = Index::from_bytes(&ix.to_bytes()).unwrap();
        assert_eq!(
            back.fwd_adj[&uid(1)],
            vec![(EdgeKind::Extension(3), uid(2))]
        );
    }

    #[test]
    fn a_bad_edge_kind_is_malformed_not_a_panic() {
        let mut ix = Index::default();
        ix.fwd_adj.insert(uid(1), vec![(EdgeKind::Deps, uid(2))]);
        let mut b = ix.to_bytes();
        // Locate the edge by its target uid and patch the two kind bytes before it.
        let target = uid(2).to_bytes();
        let at = b
            .windows(32)
            .position(|w| w == target)
            .expect("the target uid is in the encoding")
            - 2;
        b[at] = 0x00;
        b[at + 1] = 0x11; // 17: past the kernel range, below the extension base
        assert_eq!(
            Index::from_bytes(&b).unwrap_err(),
            IndexError::Malformed("edge kind")
        );
    }

    #[test]
    fn the_cache_holds_a_quantised_salience() {
        let ix = sample();
        let c = ix.cache[&uid(1)];
        assert_eq!(c.status, Status::Measured);
        assert_eq!(c.salience_q as f32 / 1024.0, 614.0 / 1024.0);
        assert_eq!(ix.unit_count(), 1);
    }
}
