//! The unit store's tables (SMYSL-2.8 §3.5.3), and the one fixed-width value among them.
//!
//! Every key is big-endian or raw bytes, so redb's bytewise order is the order the store
//! answers in: a uid key comes back in uid order, an `observed` key in time order, with no sort.

use ::redb::TableDefinition;
use smysl_core::{Status, Uid};

/// The layout this build writes and reads. A store with a larger one is refused (`SMY-E503`);
/// a smaller one is rebuilt (`SMY-W501`), since in index mode the store is derived.
///
/// 2 since TX-P13c step 3: a store describes several logs, and every row says which. 3 since
/// TX-P13d: a log can be primary, its records held by the store in `records`.
pub(super) const LAYOUT: u16 = 3;

/// A log's position among a store's, as its rows carry it: two bytes, big-endian. Assigned in
/// the order the store first saw the logs, not their name order; the name order is `LOGS`'s.
pub(super) type LogIx = u16;

/// `layout`, `mode`, `logs` (`one` or `many`).
pub(super) const META: TableDefinition<'static, &str, &[u8]> = TableDefinition::new("meta");
/// A log's name to its [`LogRow`]: which position it has and what of it the store describes.
pub(super) const LOGS: TableDefinition<'static, &str, &[u8; LOG_ROW_LEN]> =
    TableDefinition::new("logs");
/// `(record hash, log)` to the record's ordinal in that log: what `append` dedups on and the
/// record-set digest is computed over. A record two logs hold has a row in each.
pub(super) const RECORD_HASH: TableDefinition<'static, &[u8; 34], u64> =
    TableDefinition::new("record_hash");
/// A unit's uid to its [`Header`].
pub(super) const HEADER: TableDefinition<'static, &[u8; 32], &[u8; HEADER_LEN]> =
    TableDefinition::new("header");
/// `(uid, log)` for every log holding the unit, in a store of several: what opens the store for
/// some of them, and what says which log a reference into one not opened waits on (`SMY-W500`).
pub(super) const HOLDS: TableDefinition<'static, &[u8; 34], ()> = TableDefinition::new("holds");
/// `(uid, 0 for deps or 1 for grounds, target)`: the edges of every core, so the adjacency is
/// built without reading one.
pub(super) const DEPS: TableDefinition<'static, &[u8; 65], ()> = TableDefinition::new("deps");
/// `(subject, record hash, log)` to the attestation's bytes as the log holds them. The subject
/// is a unit's uid, a relation's rid, or a uid nothing in the store has yet.
pub(super) const ATTEST: TableDefinition<'static, &[u8; 66], &[u8]> =
    TableDefinition::new("attest");
/// Every record that is neither a unit nor an attestation, by `(log, ordinal)`, as the log holds
/// it: the interpretive layer, the library's records and the hot kinds. Proportional to
/// judgements, not to text, which is why it is read whole at open.
pub(super) const INTERP: TableDefinition<'static, &[u8; 10], &[u8]> =
    TableDefinition::new("interp");
/// A primary log's records (TX-P13d), by `(log, offset)` — the offset each would have in the log,
/// so a header's offset finds its core here as it finds it in a file, and the records in key
/// order are the log's bytes. A log in index mode has no rows.
pub(super) const RECORDS: TableDefinition<'static, &[u8; 10], &[u8]> =
    TableDefinition::new("records");
/// `(observed ms, uid)`, so `units_in_observed_order` is a scan.
pub(super) const OBSERVED: TableDefinition<'static, &[u8; 40], ()> =
    TableDefinition::new("observed");
/// `(source reference, uid)`, so `units_with_source_prefix` is a range.
pub(super) const SOURCE_REF: TableDefinition<'static, &[u8], ()> =
    TableDefinition::new("source_ref");

pub(super) const LOG_ROW_LEN: usize = 51;

/// What a store describes of one of its logs: its position, and the length, hash and record
/// count of the prefix it has folded, and whether the store is where its records are.
///
/// For a primary log `log_hash` is not the hash of a file — there is none — but a chain: each
/// append's hash is BLAKE3 of the last one and the bytes appended. It changes exactly when the log
/// does, which is all a stamp needs; a log written back out (`make_index`) is hashed again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LogRow {
    pub(super) ix: LogIx,
    pub(super) log_len: u64,
    pub(super) log_hash: [u8; 32],
    pub(super) records: u64,
    pub(super) primary: bool,
}

impl LogRow {
    pub(super) fn encode(&self) -> [u8; LOG_ROW_LEN] {
        let mut b = [0u8; LOG_ROW_LEN];
        b[0..2].copy_from_slice(&self.ix.to_be_bytes());
        b[2..10].copy_from_slice(&self.log_len.to_be_bytes());
        b[10..42].copy_from_slice(&self.log_hash);
        b[42..50].copy_from_slice(&self.records.to_be_bytes());
        b[50] = u8::from(self.primary);
        b
    }

    pub(super) fn decode(b: &[u8; LOG_ROW_LEN]) -> LogRow {
        LogRow {
            ix: u16::from_be_bytes([b[0], b[1]]),
            log_len: u64::from_be_bytes(b[2..10].try_into().expect("8 bytes")),
            log_hash: b[10..42].try_into().expect("32 bytes"),
            records: u64::from_be_bytes(b[42..50].try_into().expect("8 bytes")),
            primary: b[50] == 1,
        }
    }
}

/// A 32-byte key followed by a log's position: `record_hash`'s and `holds`' keys.
pub(super) fn with_log(k: &[u8; 32], log: LogIx) -> [u8; 34] {
    let mut out = [0u8; 34];
    out[..32].copy_from_slice(k);
    out[32..].copy_from_slice(&log.to_be_bytes());
    out
}

/// The first and last keys [`with_log`] can make of `k`.
pub(super) fn log_range(k: &[u8; 32]) -> ([u8; 34], [u8; 34]) {
    (with_log(k, 0), with_log(k, LogIx::MAX))
}

/// The log position at the end of a key.
pub(super) fn log_of(k: &[u8]) -> LogIx {
    let n = k.len();
    u16::from_be_bytes([k[n - 2], k[n - 1]])
}

/// A `records` key: the same shape as `interp`'s, a log's position and an offset in it.
pub(super) fn records_key(log: LogIx, offset: u64) -> [u8; 10] {
    interp_key(log, offset)
}

pub(super) fn interp_key(log: LogIx, seq: u64) -> [u8; 10] {
    let mut k = [0u8; 10];
    k[..2].copy_from_slice(&log.to_be_bytes());
    k[2..].copy_from_slice(&seq.to_be_bytes());
    k
}

pub(super) const HEADER_LEN: usize = 32;

/// What the store keeps of a unit without its core.
///
/// 32 bytes on disk: the core's log and place in it, its status, when it was observed, and how
/// many attestations it has with the newest hop among them. Not the RFC's 24: the core's
/// offset and length are here rather than in a `core` table of their own, because the only
/// question that table answered was "where is it", and asking it cost a b-tree descent on
/// every `get`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Header {
    /// The log the core is read from: the first to hold the unit.
    pub(super) log: LogIx,
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
        b[26..28].copy_from_slice(&self.log.to_be_bytes());
        b[28..32].copy_from_slice(&self.attested.to_be_bytes());
        b
    }

    pub(super) fn decode(b: &[u8; HEADER_LEN]) -> Header {
        let u64_at = |i: usize| u64::from_be_bytes(b[i..i + 8].try_into().expect("8 bytes"));
        let u32_at = |i: usize| u32::from_be_bytes(b[i..i + 4].try_into().expect("4 bytes"));
        Header {
            log: u16::from_be_bytes([b[26], b[27]]),
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

pub(super) fn attest_key(subject: &Uid, hash: &[u8; 32], log: LogIx) -> [u8; 66] {
    let mut k = [0u8; 66];
    k[..32].copy_from_slice(subject.as_bytes());
    k[32..64].copy_from_slice(hash);
    k[64..].copy_from_slice(&log.to_be_bytes());
    k
}

/// The first and last `attest` keys a subject can have.
pub(super) fn attest_range(subject: &Uid) -> ([u8; 66], [u8; 66]) {
    (
        attest_key(subject, &[0; 32], 0),
        attest_key(subject, &[0xff; 32], LogIx::MAX),
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
                log: 513,
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
                    log: 0,
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
