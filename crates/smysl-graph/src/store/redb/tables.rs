//! The unit store's tables (SMYSL-2.8 §3.5.3), and the one fixed-width value among them.
//!
//! Every key is big-endian or raw bytes, so redb's bytewise order is the order the store
//! answers in: a uid key comes back in uid order, an `observed` key in time order, with no sort.

use ::redb::TableDefinition;
use smysl_core::{Status, Uid};

/// The layout this build writes and reads. A store with a larger one is refused (`SMY-E503`);
/// a smaller one is rebuilt (`SMY-W501`), since in index mode the store is derived.
pub(super) const LAYOUT: u16 = 1;

/// `layout`, `mode`, `log_len`, `log_hash`, `records`.
pub(super) const META: TableDefinition<'static, &str, &[u8]> = TableDefinition::new("meta");
/// A record's [`crate::Store::record_hash`] to its ordinal in the log: what `append` dedups on
/// and the record-set digest is computed over.
pub(super) const RECORD_HASH: TableDefinition<'static, &[u8; 32], u64> =
    TableDefinition::new("record_hash");
/// A unit's uid to its [`Header`].
pub(super) const HEADER: TableDefinition<'static, &[u8; 32], &[u8; HEADER_LEN]> =
    TableDefinition::new("header");
/// `(uid, 0 for deps or 1 for grounds, target)`: the edges of every core, so the adjacency is
/// built without reading one.
pub(super) const DEPS: TableDefinition<'static, &[u8; 65], ()> = TableDefinition::new("deps");
/// `(subject, record hash)` to the attestation's bytes as the log holds them. The subject is a
/// unit's uid, a relation's rid, or a uid nothing in the store has yet.
pub(super) const ATTEST: TableDefinition<'static, &[u8; 64], &[u8]> =
    TableDefinition::new("attest");
/// Every record that is neither a unit nor an attestation, by ordinal, as the log holds it:
/// the interpretive layer, the library's records and the hot kinds. Proportional to
/// judgements, not to text, which is why it is read whole at open.
pub(super) const INTERP: TableDefinition<'static, u64, &[u8]> = TableDefinition::new("interp");
/// `(observed ms, uid)`, so `units_in_observed_order` is a scan.
pub(super) const OBSERVED: TableDefinition<'static, &[u8; 40], ()> =
    TableDefinition::new("observed");
/// `(source reference, uid)`, so `units_with_source_prefix` is a range.
pub(super) const SOURCE_REF: TableDefinition<'static, &[u8], ()> =
    TableDefinition::new("source_ref");

pub(super) const HEADER_LEN: usize = 32;

/// What the store keeps of a unit without its core.
///
/// 32 bytes on disk: the core's place in the log, its status, when it was observed, and how
/// many attestations it has with the newest hop among them. Not the RFC's 24: the core's
/// offset and length are here rather than in a `core` table of their own, because the only
/// question that table answered was "where is it", and asking it cost a b-tree descent on
/// every `get`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Header {
    pub(super) offset: u64,
    pub(super) len: u32,
    pub(super) status: u8,
    pub(super) observed: Option<u64>,
    /// Attestations held for the unit; `hop` means something only when this is not zero.
    pub(super) attested: u32,
    pub(super) hop: u32,
}

impl Header {
    pub(super) fn encode(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0..8].copy_from_slice(&self.offset.to_be_bytes());
        b[8..12].copy_from_slice(&self.len.to_be_bytes());
        b[12..16].copy_from_slice(&self.hop.to_be_bytes());
        b[16..24].copy_from_slice(&self.observed.unwrap_or(0).to_be_bytes());
        b[24] = self.status;
        b[25] = u8::from(self.observed.is_some());
        b[28..32].copy_from_slice(&self.attested.to_be_bytes());
        b
    }

    pub(super) fn decode(b: &[u8; HEADER_LEN]) -> Header {
        let u64_at = |i: usize| u64::from_be_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        let u32_at = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().expect("4 bytes"));
        Header {
            offset: u64_at(0),
            len: u32_at(8),
            hop: u32_at(12),
            observed: (b[25] & 1 == 1).then(|| u64_at(16)),
            status: b[24],
            attested: u32_at(28),
        }
    }

    /// Whether its core is kept once read (OQ-69): the statuses of judgements, which every
    /// interpretive operation reads again. Relation endpoints are added at open, because a
    /// relation can arrive after the unit it names.
    pub(super) fn pinned_by_status(&self) -> bool {
        [Status::Speculative, Status::Inferred, Status::Derived]
            .iter()
            .any(|s| s.as_u8() == self.status)
    }
}

pub(super) fn deps_key(uid: &Uid, grounds: bool, target: &Uid) -> [u8; 65] {
    let mut k = [0u8; 65];
    k[..32].copy_from_slice(uid.as_bytes());
    k[32] = u8::from(grounds);
    k[33..].copy_from_slice(target.as_bytes());
    k
}

pub(super) fn attest_key(subject: &Uid, hash: &[u8; 32]) -> [u8; 64] {
    let mut k = [0u8; 64];
    k[..32].copy_from_slice(subject.as_bytes());
    k[32..].copy_from_slice(hash);
    k
}

/// The first and last `attest` keys a subject can have.
pub(super) fn attest_range(subject: &Uid) -> ([u8; 64], [u8; 64]) {
    (
        attest_key(subject, &[0; 32]),
        attest_key(subject, &[0xff; 32]),
    )
}

pub(super) fn observed_key(ms: u64, uid: &Uid) -> [u8; 40] {
    let mut k = [0u8; 40];
    k[..8].copy_from_slice(&ms.to_be_bytes());
    k[8..].copy_from_slice(uid.as_bytes());
    k
}

pub(super) fn source_key(reference: &str, uid: &Uid) -> Vec<u8> {
    let mut k = Vec::with_capacity(reference.len() + 32);
    k.extend_from_slice(reference.as_bytes());
    k.extend_from_slice(uid.as_bytes());
    k
}

/// The reference and uid a `source_ref` key was made of.
pub(super) fn split_source_key(k: &[u8]) -> (&[u8], Uid) {
    let (r, u) = k.split_at(k.len() - 32);
    (r, Uid::from_bytes(u.try_into().expect("32 bytes")))
}

pub(super) fn uid_of(b: &[u8]) -> Uid {
    Uid::from_bytes(b[..32].try_into().expect("32 bytes"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_header_round_trips_with_and_without_a_time() {
        for observed in [None, Some(0), Some(1_700_000_000_000)] {
            let h = Header {
                offset: 1 << 40,
                len: 77,
                status: Status::Cited.as_u8(),
                observed,
                attested: 3,
                hop: 9,
            };
            assert_eq!(Header::decode(&h.encode()), h);
        }
    }

    #[test]
    fn only_the_statuses_of_judgements_pin() {
        let pinned: Vec<Status> = Status::ALL
            .iter()
            .copied()
            .filter(|s| {
                Header {
                    offset: 0,
                    len: 0,
                    status: s.as_u8(),
                    observed: None,
                    attested: 0,
                    hop: 0,
                }
                .pinned_by_status()
            })
            .collect();
        assert_eq!(
            pinned,
            [Status::Speculative, Status::Inferred, Status::Derived]
        );
    }
}
