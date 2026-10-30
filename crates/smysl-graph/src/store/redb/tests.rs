//! The redb store's own tests: faults, crash order and recovery (SMYSL-2.8 §5.3).
//!
//! Here rather than in `tests/` because they open the store through a storage backend of their
//! own, which [`RedbStore::open_db`] takes and nothing public does. What every case asserts is
//! the index-mode promise: whatever the store's file went through, the next open answers as
//! `Store::open` of the log, because the log is the authority.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use ::redb::backends::FileBackend;
use ::redb::{Builder, Database, StorageBackend};
use smysl_core::types::{SourceKind, SourceRef};
use smysl_core::{
    canonical_uid, to_cbor_seq, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind,
    Relation, Rung, Status, UnitCoreBuilder, Withdrawal,
};

use super::{RedbOptions, RedbStore};
use crate::store::{OpenReport, Store, StoreRead};

fn agent() -> AgentId {
    AgentId::new("tool:fault").unwrap()
}

/// A store with every shape the fold distinguishes: cited units with sources and times, claims
/// grounded on them, attestations before and after their units, relations, an attestation of a
/// relation, a withdrawal, and repeats.
fn synthetic(n: usize) -> Vec<Record> {
    let mut out = Vec::new();
    let mut cited = Vec::new();
    let mut claims = Vec::new();
    let mut i = 0usize;
    while out.len() < n {
        let src = SourceRef::new(SourceKind::Doc, format!("bench:f#{i}"))
            .observed_at(1_000 + (i as u64 * 7919) % 5000);
        let core = UnitCoreBuilder::new(KernelType::Prose, format!("reading {i}"), Status::Cited)
            .source(src)
            .build()
            .unwrap();
        let uid = canonical_uid(&core);
        let attest = Attestation::new(
            uid,
            agent(),
            Op::Imported,
            Rung::Document,
            Hlc::new(i as u64, 0, agent()),
        )
        .at_hop(i as u32 % 4);
        // Every fourth attestation reaches the log before its unit.
        if i % 4 == 0 {
            out.push(Record::Attestation(attest));
            out.push(Record::Unit(core));
        } else {
            out.push(Record::Unit(core));
            out.push(Record::Attestation(attest));
        }
        cited.push(uid);
        if i % 5 == 4 {
            let claim =
                UnitCoreBuilder::new(KernelType::Claim, format!("claim {i}"), Status::Derived)
                    .grounds(cited[cited.len() - 3..].to_vec())
                    .build()
                    .unwrap();
            let c = canonical_uid(&claim);
            out.push(Record::Unit(claim));
            if let Some(prev) = claims.last() {
                let rel = Relation::new(RelKind::Backs, c, *prev);
                let rid = rel.uid();
                out.push(Record::Relation(rel));
                if i % 3 == 0 {
                    out.push(Record::Attestation(Attestation::new(
                        rid,
                        agent(),
                        Op::Attested,
                        Rung::Computed,
                        Hlc::new(i as u64, 1, agent()),
                    )));
                }
                if i % 7 == 0 {
                    out.push(Record::Withdrawal(Withdrawal::new(
                        rid,
                        agent(),
                        Hlc::new(i as u64, 2, agent()),
                    )));
                }
            }
            claims.push(c);
        }
        if i % 11 == 10 {
            out.push(out[out.len() / 2].clone());
        }
        i += 1;
    }
    out.truncate(n);
    out
}

/// Everything a store can be asked that a fault could get wrong, as one comparable string.
fn observe(s: &(impl StoreRead + ?Sized)) -> String {
    format!(
        "{} units, digest {:?}, state {:?}, {} duplicates, adjacency {:?}, observed {:?}, \
         sourced {:?}, relations {:?}, withdrawals {}",
        s.unit_count(),
        s.record_set_digest(),
        s.state_hash(),
        s.duplicate_records(),
        s.adjacency(),
        s.units_in_observed_order(),
        s.units_with_source_prefix("bench:f#1"),
        s.relations().collect::<Vec<_>>(),
        s.withdrawals().count(),
    )
}

/// How a write goes wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// The n-th write and every one after it fail: the disk went away.
    Fail(u64),
    /// The n-th write lands half of its bytes, then it and every later one fail: the power went
    /// out mid-write.
    Tear(u64),
}

/// A file backend that counts writes, and fails them from one on.
#[derive(Debug)]
struct Faulty {
    inner: FileBackend,
    writes: Arc<AtomicU64>,
    fault: Option<Fault>,
}

impl StorageBackend for Faulty {
    fn len(&self) -> Result<u64, std::io::Error> {
        self.inner.len()
    }
    fn read(&self, offset: u64, len: usize) -> Result<Vec<u8>, std::io::Error> {
        self.inner.read(offset, len)
    }
    fn set_len(&self, len: u64) -> Result<(), std::io::Error> {
        if self.failing() {
            return Err(std::io::Error::other("injected: set_len after the fault"));
        }
        self.inner.set_len(len)
    }
    fn sync_data(&self, eventual: bool) -> Result<(), std::io::Error> {
        if self.failing() {
            return Err(std::io::Error::other("injected: sync after the fault"));
        }
        self.inner.sync_data(eventual)
    }
    fn write(&self, offset: u64, data: &[u8]) -> Result<(), std::io::Error> {
        let n = self.writes.fetch_add(1, Ordering::SeqCst);
        match self.fault {
            Some(Fault::Fail(at)) if n >= at => Err(std::io::Error::other("injected: write")),
            Some(Fault::Tear(at)) if n == at => {
                self.inner.write(offset, &data[..data.len() / 2])?;
                Err(std::io::Error::other("injected: torn write"))
            }
            Some(Fault::Tear(at)) if n > at => Err(std::io::Error::other("injected: write")),
            _ => self.inner.write(offset, data),
        }
    }
}

impl Faulty {
    fn failing(&self) -> bool {
        let n = self.writes.load(Ordering::SeqCst);
        matches!(self.fault, Some(Fault::Fail(at) | Fault::Tear(at)) if n > at)
    }
}

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-redb-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Open the store for `log` through a faulty backend.
fn open_faulty(
    log: &Path,
    writes: &Arc<AtomicU64>,
    fault: Option<Fault>,
) -> Result<(RedbStore, OpenReport), smysl_core::Error> {
    let path = RedbStore::store_path(log);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(smysl_core::Error::Io)?;
    let backend = Faulty {
        inner: FileBackend::new(file).map_err(super::backend)?,
        writes: writes.clone(),
        fault,
    };
    let db = Builder::new()
        .set_cache_size(1 << 20)
        .create_with_backend(backend)
        .map_err(super::backend)?;
    RedbStore::open_db(db, log, &RedbOptions::default(), OpenReport::default())
}

/// Append a batch to the log, durably: the first half of index mode's write order.
fn append_log(log: &Path, batch: &[Record]) {
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .unwrap();
    f.write_all(&to_cbor_seq(batch)).unwrap();
    f.sync_data().unwrap();
}

/// What redb itself thinks of the file, with nothing of ours holding it: `None` when it will
/// not open as a database at all — a torn write of its very first page does that — which index
/// mode answers by rebuilding.
fn integrity(log: &Path) -> Option<bool> {
    let mut db = Database::create(RedbStore::store_path(log)).ok()?;
    Some(db.check_integrity().unwrap())
}

const BATCH: usize = 100;

/// Run the 1,000-record append sequence, with a fault if one is given, and stop at the first
/// open that fails — a crash is the end of the process. Returns how many writes were made.
fn sequence(log: &Path, records: &[Record], fault: Option<Fault>) -> u64 {
    let writes = Arc::new(AtomicU64::new(0));
    for batch in records.chunks(BATCH) {
        append_log(log, batch);
        if open_faulty(log, &writes, fault).is_err() {
            break;
        }
    }
    writes.load(Ordering::SeqCst)
}

/// Inject `Fault::Fail(n)` and `Fault::Tear(n)` for each `n` that `points` picks out of the
/// sequence's writes, and check each crash recovers. Returns the cases run.
fn faults_at(what: &str, points: impl Fn(u64) -> Vec<u64>) -> usize {
    let records = synthetic(1_000);
    let dir = scratch(what);
    let total = sequence(&dir.join("dry.smy"), &records, None);
    assert!(total > 20, "the sequence made only {total} writes");

    let mut cases = 0;
    let mut unopenable = 0;
    for n in points(total) {
        for fault in [Fault::Fail(n), Fault::Tear(n)] {
            let case = dir.join(format!("case-{n}-{}", matches!(fault, Fault::Tear(_))));
            std::fs::create_dir_all(&case).unwrap();
            let log = case.join("store.smy");
            sequence(&log, &records, Some(fault));

            // The process is gone; the next one opens normally.
            let sound = integrity(&log);
            assert_ne!(sound, Some(false), "{fault:?}: redb's own check fails");
            let (s, report) = RedbStore::open(&log, RedbOptions::default())
                .unwrap_or_else(|e| panic!("{fault:?}: the reopen failed: {e}"));
            if sound.is_none() {
                assert!(
                    report.index_rebuilt,
                    "{fault:?}: an unopenable file is rebuilt"
                );
                unopenable += 1;
            }
            for d in report.report.iter() {
                assert!(
                    matches!(
                        d.code,
                        smysl_core::diag::Code::W501 | smysl_core::diag::Code::W110
                    ),
                    "{fault:?}: the reopen reported {:?}",
                    d.code
                );
            }
            let want = Store::open(&log).unwrap();
            assert_eq!(
                observe(&s),
                observe(&want),
                "{fault:?}: the store is not its log"
            );
            drop(s);
            let _ = std::fs::remove_dir_all(&case);
            cases += 1;
        }
    }
    // Most faults leave a file redb repairs; only damage to its first pages leaves one it
    // cannot open, and that has to stay the exception for this to be testing recovery.
    assert!(
        unopenable * 4 < cases,
        "{unopenable} of {cases} cases were unopenable"
    );
    eprintln!(
        "{cases} fault cases over {total} writes; {unopenable} left a file redb would not open"
    );
    let _ = std::fs::remove_dir_all(&dir);
    cases
}

/// Forty write points, seeded, and the first and last: what every `cargo test` runs.
#[test]
fn a_write_that_fails_or_tears_leaves_a_store_that_reopens_as_its_log() {
    let cases = faults_at("faults", |total| {
        let mut seed = 0x5eed_u64;
        let mut at: Vec<u64> = (0..40)
            .map(|_| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (seed >> 33) % total
            })
            .chain([0, total - 1])
            .collect();
        at.sort_unstable();
        at.dedup();
        at
    });
    assert!(cases >= 60, "{cases} cases");
}

/// Every write of the sequence (§5.3's "for n over every write"): minutes, so on request —
/// `cargo test -p smysl-graph --features store-redb -- --ignored every_write`.
#[test]
#[ignore = "every write of the sequence, twice: minutes; run on request"]
fn every_write_that_fails_or_tears_leaves_a_store_that_reopens_as_its_log() {
    faults_at("every-write", |total| (0..total).collect());
}

/// The crash between the log's `fsync` and the store's commit: the store is behind its log,
/// and the next open folds in exactly the records it missed — not a rebuild.
#[test]
fn a_crash_between_the_log_and_the_commit_replays_exactly_the_tail() {
    let records = synthetic(400);
    let dir = scratch("order");
    let log = dir.join("store.smy");
    append_log(&log, &records[..300]);
    let writes = Arc::new(AtomicU64::new(0));
    drop(open_faulty(&log, &writes, None).unwrap());

    // The log has the batch; the commit that would have recorded it never happens.
    append_log(&log, &records[300..]);
    let at = writes.load(Ordering::SeqCst);
    assert!(open_faulty(&log, &writes, Some(Fault::Fail(at))).is_err());
    assert_eq!(integrity(&log), Some(true));

    let (s, report) = RedbStore::open(&log, RedbOptions::default()).unwrap();
    assert!(!report.index_rebuilt, "behind is caught up, not rebuilt");
    assert_eq!(report.replayed, 100, "exactly the tail");
    assert_eq!(observe(&s), observe(&Store::open(&log).unwrap()));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A store written in a layout this build does not know is refused, not rewritten; one it
/// can no longer open as a database at all is rebuilt.
#[test]
fn a_newer_layout_is_refused_and_a_damaged_file_is_rebuilt() {
    let dir = scratch("layout");
    let log = dir.join("store.smy");
    append_log(&log, &synthetic(50));
    drop(RedbStore::open(&log, RedbOptions::default()).unwrap());
    {
        let db = Database::create(RedbStore::store_path(&log)).unwrap();
        let txn = db.begin_write().unwrap();
        txn.open_table(super::tables::META)
            .unwrap()
            .insert(
                "layout",
                (super::tables::LAYOUT + 1).to_be_bytes().as_slice(),
            )
            .unwrap();
        txn.commit().unwrap();
    }
    let err = RedbStore::open(&log, RedbOptions::default()).unwrap_err();
    assert!(err.to_string().contains("SMY-E503"), "{err}");
    assert_eq!(err.exit_code(), smysl_core::ExitCode::UnsupportedVersion);

    std::fs::write(RedbStore::store_path(&log), b"not a database").unwrap();
    let (s, report) = RedbStore::open(&log, RedbOptions::default()).unwrap();
    assert!(report.index_rebuilt);
    assert!(report
        .report
        .iter()
        .any(|d| d.code == smysl_core::diag::Code::W501));
    assert_eq!(observe(&s), observe(&Store::open(&log).unwrap()));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Pinned cores stay once read; paged ones go when the budget is spent; and answers are the
/// same either way, which the conformance suite shows at scale and this shows for the tiers.
#[test]
fn the_cache_pins_judgements_and_pages_the_rest() {
    let records = synthetic(600);
    let dir = scratch("tiers");
    let log = dir.join("store.smy");
    append_log(&log, &records);
    let opts = RedbOptions {
        core_cache_bytes: 2_000,
        ..Default::default()
    };
    let (s, _) = RedbStore::open(&log, opts).unwrap();
    let first = observe(&s);
    let derived = s
        .units()
        .filter(|(_, u)| u.core.status == Status::Derived)
        .count();
    let stats = s.cache_stats();
    assert!(derived > 0);
    assert!(stats.pinned >= derived, "{stats:?}");
    assert!(stats.paged_bytes <= 2_000, "{stats:?}");
    assert!(stats.paged < s.unit_count() - derived, "{stats:?}");
    assert_eq!(observe(&s), first);
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
