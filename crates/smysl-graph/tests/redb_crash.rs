//! The unit store across processes (SMYSL-2.8 §5.3): killed mid-append, and held by another.
//!
//! The tests that need a second process run this binary again as it — the `child` test below,
//! which does nothing unless the parent set `SMYSL_REDB_CHILD` — so "another process" is
//! another process, and not a second open in this one: since TX-P13b step 2 opens in one
//! process share one database, and would not show what another process sees.
//!
//! The kill test `SIGKILL`s a writing child at seeded moments. Whatever moment that was, the
//! next open must answer as `Store::open` of the log and report nothing worse than a torn tail
//! (`SMY-W110`) or a rebuild (`SMY-W501`).
#![cfg(feature = "store-redb")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use smysl_core::diag::Code;
use smysl_core::{
    canonical_uid, to_cbor_seq, AgentId, Attestation, Hlc, KernelType, Op, Record, Rung, Status,
    UnitCoreBuilder,
};
use smysl_graph::lock::Lock;
use smysl_graph::redb::{RedbOptions, RedbShard, RedbStore};
use smysl_graph::{AnyStore, Store, StoreRead};

const BATCH: usize = 50;

fn scratch(what: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-redb-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Batch `b`: fifty units, each attested, the attestation half the time ahead of its unit.
fn batch(b: usize) -> Vec<Record> {
    let agent = AgentId::new("tool:kill").unwrap();
    let mut out = Vec::new();
    for i in 0..BATCH {
        let n = b * BATCH + i;
        let core =
            UnitCoreBuilder::new(KernelType::Claim, format!("claim {n}"), Status::Speculative)
                .build()
                .unwrap();
        let a = Attestation::new(
            canonical_uid(&core),
            agent.clone(),
            Op::Authored,
            Rung::Computed,
            Hlc::new(n as u64, 0, agent.clone()),
        );
        if n % 2 == 0 {
            out.push(Record::Attestation(a));
            out.push(Record::Unit(core));
        } else {
            out.push(Record::Unit(core));
            out.push(Record::Attestation(a));
        }
    }
    out
}

/// Late attestations of batch `b`'s units, by a second agent: records that change what an
/// existing unit answers, which is what a snapshot must not see.
fn late(b: usize, round: u64) -> Vec<Record> {
    let agent = AgentId::new("tool:late").unwrap();
    batch(b)
        .into_iter()
        .filter_map(|r| match r {
            Record::Unit(core) => Some(Record::Attestation(Attestation::new(
                canonical_uid(&core),
                agent.clone(),
                Op::Attested,
                Rung::Computed,
                Hlc::new(1_000_000 + round, 0, agent.clone()),
            ))),
            _ => None,
        })
        .collect()
}

fn code_of(e: &smysl_core::Error) -> Option<Code> {
    match e {
        smysl_core::Error::Lib(l) => l.code(),
        _ => None,
    }
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

fn wait_for(path: &Path) {
    let started = Instant::now();
    while !path.exists() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the child never got ready"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// What a child does, by `SMYSL_REDB_CHILD`, on the log at `SMYSL_REDB_LOG`.
fn child_main(mode: &str, log: &Path) {
    let out = env("SMYSL_REDB_OUT").map(PathBuf::from);
    let ready = || {
        if let Some(o) = &out {
            std::fs::write(o, "ready").unwrap();
        }
    };
    match mode {
        // The writer, through the writer: one shard, batches of fifty, until killed.
        "kill-shard" => {
            let (mut shard, _) = RedbShard::open_rw(log, RedbOptions::default()).unwrap();
            for b in 0..=usize::MAX {
                shard.append(&batch(b)).unwrap();
            }
        }
        // A writer that does not take the lock — a `Store` on the same log — writing each
        // batch in two halves so a kill can land between them, and the store catching up.
        "kill-raw" => {
            for b in 0..=usize::MAX {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log)
                    .unwrap();
                let bytes = to_cbor_seq(&batch(b));
                let (first, rest) = bytes.split_at(bytes.len() / 2);
                f.write_all(first).unwrap();
                // Long enough that some kills land here; the store's open is most of the time.
                std::thread::sleep(Duration::from_millis(15));
                f.write_all(rest).unwrap();
                f.sync_data().unwrap();
                drop(RedbStore::open(log, RedbOptions::default()).unwrap());
            }
        }
        // Try to become the writer; say how that went; hold the lock a while if it worked.
        "rw-hold" => {
            let said = match RedbShard::open_rw(log, RedbOptions::default()) {
                Ok((shard, _)) => {
                    std::fs::write(out.as_ref().unwrap(), "ok").unwrap();
                    std::thread::sleep(Duration::from_millis(1500));
                    drop(shard);
                    return;
                }
                Err(e) => match code_of(&e) {
                    Some(Code::E445) => format!("E445 {e}"),
                    _ => format!("other {e}"),
                },
            };
            std::fs::write(out.as_ref().unwrap(), said).unwrap();
        }
        // A writer that keeps writing: four batches, then ready, then a batch every 20 ms.
        "write-on" => {
            let (mut shard, _) = RedbShard::open_rw(log, RedbOptions::default()).unwrap();
            for b in 0..4 {
                shard.append(&batch(b)).unwrap();
            }
            ready();
            for b in 4..=usize::MAX {
                shard.append(&batch(b)).unwrap();
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        // A reader that holds the store for `SMYSL_REDB_HOLD_MS`.
        "hold" => {
            let (store, _) = RedbStore::open(log, RedbOptions::default()).unwrap();
            ready();
            let ms = env("SMYSL_REDB_HOLD_MS").unwrap().parse().unwrap();
            std::thread::sleep(Duration::from_millis(ms));
            drop(store);
        }
        other => panic!("no child mode {other}"),
    }
}

#[test]
fn child() {
    if let (Some(mode), Some(log)) = (env("SMYSL_REDB_CHILD"), env("SMYSL_REDB_LOG")) {
        child_main(&mode, Path::new(&log));
    }
}

fn spawn(mode: &str, log: &Path, extra: &[(&str, String)]) -> std::process::Child {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", "child", "--nocapture", "--test-threads=1"])
        .env("SMYSL_REDB_CHILD", mode)
        .env("SMYSL_REDB_LOG", log)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd.spawn().unwrap()
}

/// A small seeded generator, so the kill points are the same on every run.
fn lcg(seed: &mut u64) -> u64 {
    *seed = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    *seed >> 33
}

/// redb's own check of the file, on a store nothing has open.
fn integrity(log: &Path) -> bool {
    redb::Database::open(RedbStore::store_path(log))
        .unwrap()
        .check_integrity()
        .unwrap()
}

/// The answers a store gives that a reopen must give again.
fn assert_is_its_log(s: &impl StoreRead, log: &Path, what: &str) {
    let want = Store::open(log).unwrap();
    assert_eq!(
        s.record_set_digest(),
        want.record_set_digest(),
        "{what}: the store is not its log"
    );
    assert_eq!(s.unit_count(), want.unit_count(), "{what}");
    assert_eq!(s.state_hash(), want.state_hash(), "{what}");
}

/// §5.3's kill test, through both writers: even points the shard, odd points a writer that
/// does not lock with the store catching up behind it.
///
/// A killed shard leaves its lock, naming the dead writer — the crash notice the lock file is
/// for. Breaking it and opening for writing cuts a torn tail, and what is appended then reads
/// back.
#[test]
fn a_writer_killed_anywhere_leaves_a_store_that_reopens_as_its_log() {
    let dir = scratch("kill");
    let mut seed = 0x5eed_u64;
    let mut torn = 0;
    let mut written = 0;
    for point in 0..20 {
        let log = dir.join(format!("kill-{point}.smy"));
        let mode = if point % 2 == 0 {
            "kill-shard"
        } else {
            "kill-raw"
        };
        let mut child = spawn(mode, &log, &[]);
        // Long enough to have written something on most runs, at a moment nobody chose.
        std::thread::sleep(Duration::from_millis(150 + lcg(&mut seed) % 600));
        child.kill().unwrap();
        child.wait().unwrap();

        let (s, report) = RedbStore::open(&log, RedbOptions::default())
            .unwrap_or_else(|e| panic!("kill point {point}: the reopen failed: {e}"));
        for d in report.report.iter() {
            assert!(
                matches!(d.code, Code::W110 | Code::W501),
                "kill point {point}: reported {:?}",
                d.code
            );
        }
        torn += usize::from(report.trailing_bytes > 0);
        written += s.unit_count();
        assert_is_its_log(&s, &log, &format!("kill point {point}"));
        drop(s);
        assert!(
            integrity(&log),
            "kill point {point}: redb's integrity check"
        );

        if mode == "kill-shard" {
            let lock = RedbShard::lock_path(&log);
            let holder = Lock::holder(&lock).expect("a killed writer leaves its lock");
            assert_eq!(holder.pid, child.id(), "kill point {point}");
            let e = RedbShard::open_rw(&log, RedbOptions::default()).unwrap_err();
            assert_eq!(code_of(&e), Some(Code::E445), "kill point {point}: {e}");
            Lock::break_lock(&lock).unwrap();
            let (mut shard, _) = RedbShard::open_rw(&log, RedbOptions::default()).unwrap();
            shard.append(&batch(10_000)).unwrap();
            assert_is_its_log(&shard.reader().unwrap(), &log, &format!("after {point}"));
        }
    }
    // Not vacuous: the writers got somewhere before they were stopped.
    assert!(
        written >= 20 * BATCH,
        "{written} units across 20 kill points"
    );
    eprintln!("20 kill points, {written} units written, {torn} with a torn tail");
    let _ = std::fs::remove_dir_all(&dir);
}

/// §5.3: two processes open the same shard for writing; exactly one gets `SMY-E445`, and it
/// is told who the other is.
#[test]
fn of_two_writers_exactly_one_is_refused_and_told_who_holds_it() {
    let dir = scratch("single");
    let log = dir.join("single.smy");
    let outs = [dir.join("a.out"), dir.join("b.out")];
    let mut children: Vec<_> = outs
        .iter()
        .map(|o| {
            spawn(
                "rw-hold",
                &log,
                &[("SMYSL_REDB_OUT", o.display().to_string())],
            )
        })
        .collect();
    let pids: Vec<u32> = children.iter().map(|c| c.id()).collect();
    for c in &mut children {
        assert!(c.wait().unwrap().success());
    }
    let said: Vec<String> = outs
        .iter()
        .map(|o| std::fs::read_to_string(o).unwrap())
        .collect();
    let ok = said.iter().filter(|s| *s == "ok").count();
    let refused: Vec<&String> = said.iter().filter(|s| s.starts_with("E445")).collect();
    assert_eq!((ok, refused.len()), (1, 1), "{said:?}");
    let winner = pids[said.iter().position(|s| s == "ok").unwrap()];
    assert!(
        refused[0].contains(&winner.to_string()),
        "names the holder: {}",
        refused[0]
    );
    assert!(!RedbShard::lock_path(&log).exists(), "released on exit");
    let _ = std::fs::remove_dir_all(&dir);
}

/// §5.3: eight readers on snapshots while the writer appends. Each answers from the moment it
/// was taken — the records, the units, and what a unit's attestations are, read from the
/// store's table with the cache off so nothing is answered from memory — however much is
/// appended meanwhile.
#[test]
fn eight_readers_each_answer_from_one_snapshot_while_the_writer_appends() {
    let dir = scratch("readers");
    let log = dir.join("readers.smy");
    let mut opts = RedbOptions::default();
    opts.core_cache_bytes = 0;
    let (mut shard, _) = RedbShard::open_rw(&log, opts).unwrap();
    for b in 0..4 {
        shard.append(&batch(b)).unwrap();
    }
    let probe = batch(0)
        .into_iter()
        .find_map(|r| match r {
            Record::Unit(core) => Some(canonical_uid(&core)),
            _ => None,
        })
        .unwrap();

    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut readers = Vec::new();
    for r in 0..8u64 {
        // Each taken at a different point in the writing, so the snapshots differ.
        shard.append(&late(0, r)).unwrap();
        let snap = shard.reader().unwrap();
        let done = done.clone();
        readers.push(std::thread::spawn(move || {
            let first = (
                snap.record_set_digest(),
                snap.units().count(),
                snap.get(&probe).unwrap().attestations.len(),
            );
            let mut looks = 0;
            while !done.load(std::sync::atomic::Ordering::Relaxed) || looks < 3 {
                let now = (
                    snap.record_set_digest(),
                    snap.units().count(),
                    snap.get(&probe).unwrap().attestations.len(),
                );
                assert_eq!(now, first, "reader {r} moved");
                looks += 1;
            }
            // And the moment it answers from is a prefix of the log: an in-memory open of
            // exactly that prefix agrees with it.
            let bytes = snap.log_bytes();
            let (records, _) = smysl_core::from_cbor_seq(&bytes).unwrap();
            let want = Store::from_records(records);
            assert_eq!(snap.record_set_digest(), want.record_set_digest());
            assert_eq!(snap.state_hash(), want.state_hash());
            (r, first.2, looks)
        }));
    }
    for b in 4..24 {
        shard.append(&batch(b)).unwrap();
        shard.append(&late(b % 4, 100 + b as u64)).unwrap();
    }
    done.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut seen = Vec::new();
    for h in readers {
        let (r, attested, looks) = h.join().unwrap();
        assert!(looks >= 3);
        seen.push((r, attested));
    }
    // Reader r was taken after r + 1 late attestations of the probe, beside its own one.
    for (r, attested) in seen {
        assert_eq!(attested, 2 + r as usize, "reader {r}");
    }
    assert_is_its_log(&shard.reader().unwrap(), &log, "after the writes");
    drop(shard);
    assert!(integrity(&log));
    let _ = std::fs::remove_dir_all(&dir);
}

/// §5.3: a reader in another process while a writer is writing falls back to the log within
/// the wait, says so with `SMY-W504`, and answers as an in-memory open of the prefix it read.
#[test]
fn a_reader_meeting_a_writer_in_another_process_reads_the_log_prefix() {
    let dir = scratch("cross");
    let log = dir.join("cross.smy");
    let ready = dir.join("ready");
    let mut writer = spawn(
        "write-on",
        &log,
        &[("SMYSL_REDB_OUT", ready.display().to_string())],
    );
    wait_for(&ready);
    let mut opts = RedbOptions::default();
    opts.reader_wait_ms = 200;
    let started = Instant::now();
    let (s, report) = AnyStore::open_redb(&log, opts).unwrap();
    let waited = started.elapsed();
    writer.kill().unwrap();
    writer.wait().unwrap();

    let m = s.as_memory().expect("fell back to the log");
    assert!(report.report.iter().any(|d| d.code == Code::W504));
    assert!(waited < Duration::from_secs(5), "waited {waited:?}");
    let read = m.log_bytes();
    assert!(m.unit_count() >= 4 * BATCH, "{} units", m.unit_count());
    let now = std::fs::read(&log).unwrap();
    assert_eq!(&now[..read.len()], &read[..], "a prefix of the log");
    let (records, _) = smysl_core::from_cbor_seq(&read).unwrap();
    let want = Store::from_records(records);
    assert_eq!(m.record_set_digest(), want.record_set_digest());
    assert_eq!(m.state_hash(), want.state_hash());
    let _ = std::fs::remove_dir_all(&dir);
}

fn a_log(dir: &Path) -> PathBuf {
    let log = dir.join("held.smy");
    let records: Vec<Record> = (0..4).flat_map(batch).collect();
    std::fs::write(&log, to_cbor_seq(&records)).unwrap();
    // Built once here, so the child's open is an open and not a build.
    drop(RedbStore::open(&log, RedbOptions::default()).unwrap());
    log
}

/// OQ-67's answer under redb 2.6: a store another process has open is waited for, then read
/// from the log, with `SMY-W504` — the same answers, never a wait longer than asked.
#[test]
fn a_store_held_elsewhere_is_read_from_its_log_after_the_wait() {
    let dir = scratch("held");
    let log = a_log(&dir);
    let ready = dir.join("ready");
    let mut holder = spawn(
        "hold",
        &log,
        &[
            ("SMYSL_REDB_OUT", ready.display().to_string()),
            ("SMYSL_REDB_HOLD_MS", "10000".into()),
        ],
    );
    wait_for(&ready);

    let mut opts = RedbOptions::default();
    opts.reader_wait_ms = 200;
    let started = Instant::now();
    let (s, report) = AnyStore::open_redb(&log, opts).unwrap();
    let waited = started.elapsed();
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert!(s.as_memory().is_some(), "fell back to the log");
    assert!(report.report.iter().any(|d| d.code == Code::W504));
    assert!(waited >= Duration::from_millis(200), "waited {waited:?}");
    assert!(waited < Duration::from_secs(5), "waited {waited:?}");
    assert_is_its_log(&s, &log, "the fallback");
    let _ = std::fs::remove_dir_all(&dir);
}

/// And a store released within the wait is opened, with nothing to report.
#[test]
fn a_store_released_within_the_wait_is_opened() {
    let dir = scratch("released");
    let log = a_log(&dir);
    let ready = dir.join("ready");
    let mut holder = spawn(
        "hold",
        &log,
        &[
            ("SMYSL_REDB_OUT", ready.display().to_string()),
            ("SMYSL_REDB_HOLD_MS", "300".into()),
        ],
    );
    wait_for(&ready);
    let (s, report) = AnyStore::open_redb(&log, RedbOptions::default()).unwrap();
    holder.wait().unwrap();
    assert!(s.as_memory().is_none(), "opened the unit store");
    assert!(report.report.is_empty(), "{:?}", report.report);
    assert_is_its_log(&s, &log, "the store");
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}

/// In one process, opens share the database: a reader and the writer of the same store do not
/// hold each other off, and a store opened while the writer has it sees what it has written.
#[test]
fn in_one_process_readers_and_the_writer_share_the_store() {
    let dir = scratch("same");
    let log = dir.join("same.smy");
    let (mut shard, _) = RedbShard::open_rw(&log, RedbOptions::default()).unwrap();
    shard.append(&batch(0)).unwrap();
    let (a, ra) = RedbStore::open(&log, RedbOptions::default()).unwrap();
    let (b, _) = RedbStore::open(&log, RedbOptions::default()).unwrap();
    assert!(ra.report.is_empty(), "{:?}", ra.report);
    assert_eq!(a.unit_count(), BATCH);
    shard.append(&batch(1)).unwrap();
    assert_eq!(a.unit_count(), BATCH, "a snapshot");
    assert_eq!(b.unit_count(), BATCH);
    let (c, _) = RedbStore::open(&log, RedbOptions::default()).unwrap();
    assert_eq!(c.unit_count(), 2 * BATCH);
    // A second writer in the same process is a second writer.
    let e = RedbShard::open_rw(&log, RedbOptions::default()).unwrap_err();
    assert_eq!(code_of(&e), Some(Code::E445));
    drop((a, b, c, shard));
    assert!(integrity(&log));
    let _ = std::fs::remove_dir_all(&dir);
}
