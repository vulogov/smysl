//! AC-8's contract (RFC SMYSL-3 §17.1): a store's kept check, given a batch, reports what the
//! full check of the store with the batch reports. Exactly: the same diagnostics, the same
//! messages, the same counts.
//!
//! The oracle is [`check`] itself, twice over: over an [`Overlay`] of the store and the batch,
//! which is what staging checked before AC-8, and over [`Store::from_records`] of everything,
//! which is what the store will be. The cache must agree with both after every batch, and after
//! advancing, its report must be the full check of the store it now describes.
//!
//! What is dealt into batches:
//!
//! - every fixture store the check is pinned on (`fixtures/conformance/check/`, its `time/`,
//!   `fixtures/library/check/`, `fixtures/corpus/`, the tutorial's, `fixtures/tel/check/`),
//!   one by one and all together;
//! - generated stores, from a seed.
//!
//! Each is dealt many ways: a random part is the store the cache is built on, and the rest
//! arrives in one to five batches, in a random order. A record can arrive twice, in two
//! batches, as a retried append does.

use std::path::{Path, PathBuf};

use smysl_check::incremental::{Checked, Path as Route};
use smysl_check::{check, CheckOptions};
use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, from_cbor_seq, AgentId, Attestation, Hlc, KernelType, Op, Record, RelKind,
    Relation, Rung, SourceKind, SourceRef, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::{Overlay, Store};

/// splitmix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}

/// Every fixture store under `dir` (not recursing), by name.
fn stores_in(dir: &str) -> Vec<(String, Vec<Record>)> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root().join(dir))
        .unwrap_or_else(|e| panic!("{dir}: {e}"))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "smy" || x == "cbor"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let records = if p.extension().is_some_and(|x| x == "smy") {
            let src = std::fs::read_to_string(&p).unwrap();
            match parse_surface(&src) {
                Ok(parsed) => parsed.records,
                Err(_) => continue,
            }
        } else {
            match from_cbor_seq(&std::fs::read(&p).unwrap()) {
                Ok((records, _)) => records,
                Err(_) => continue,
            }
        };
        out.push((
            format!("{dir}/{}", p.file_name().unwrap().to_string_lossy()),
            records,
        ));
    }
    out
}

fn fixtures() -> Vec<(String, Vec<Record>)> {
    let mut out = Vec::new();
    for dir in [
        "conformance/check",
        "conformance/check/time",
        "library/check",
        "corpus",
        "tutorial/ch01-03-foundations",
        "tutorial/ch04-05-creating-formatting",
        "tutorial/ch19-21-verify",
        "tutorial/ch25-27-library-walkthrough",
        "dossier",
        "tel/check",
    ] {
        out.extend(stores_in(dir));
    }
    assert!(
        out.len() >= 50,
        "only {} fixture stores were read",
        out.len()
    );
    out
}

/// The report, as two can be compared: sorted, as `check` leaves it.
fn sorted(mut r: smysl_core::diag::Report) -> smysl_core::diag::Report {
    r.sort();
    r
}

/// Deal `records` into a base and batches, and hold the contract at every batch. Returns how
/// many batches took the incremental path.
fn deal(name: &str, records: &[Record], rng: &mut Rng, opts: &CheckOptions) -> (usize, usize) {
    let batches_n = 1 + rng.below(5);
    let mut base = Vec::new();
    let mut batches: Vec<Vec<Record>> = vec![Vec::new(); batches_n];
    for r in records {
        if rng.chance(2) {
            base.push(r.clone());
        } else {
            batches[rng.below(batches_n)].push(r.clone());
        }
        if rng.chance(12) {
            batches[rng.below(batches_n)].push(r.clone());
        }
    }
    let mut store = Store::from_records(base.clone());
    let mut held = base;
    let mut checked = Checked::new(&store, opts);
    assert_eq!(
        sorted(checked.report().clone()),
        check(&store, opts.clone()),
        "{name}: the cache's first report is not the full check"
    );
    let (mut incremental, mut total) = (0, 0);
    for (i, batch) in batches.into_iter().enumerate() {
        let got = checked.with_batch(&store, batch.clone(), opts);
        let over = check(&Overlay::new(&store, batch.clone()), opts.clone());
        held.extend(batch);
        let whole = Store::from_records(held.clone());
        let full = check(&whole, opts.clone());
        assert_eq!(
            sorted(got.report.clone()),
            over,
            "{name}, batch {i} ({:?}): the cache disagrees with the check over the overlay",
            got.path
        );
        assert_eq!(over, full, "{name}, batch {i}: overlay and store disagree");
        total += 1;
        if got.path == Route::Incremental {
            incremental += 1;
        }
        checked.advance(got);
        store = whole;
    }
    (incremental, total)
}

#[test]
fn the_kept_check_agrees_with_the_full_check_on_every_fixture() {
    let all = fixtures();
    let opts = CheckOptions::default();
    let mut rng = Rng(8);
    for (name, records) in &all {
        for _ in 0..8 {
            deal(name, records, &mut rng, &opts);
        }
    }
    // All of them at once: a store of many shapes, dealt the same way.
    let every: Vec<Record> = all.iter().flat_map(|(_, r)| r.iter().cloned()).collect();
    for _ in 0..12 {
        deal("every fixture", &every, &mut rng, &opts);
    }
}

#[test]
fn changed_options_are_checked_in_full_and_adopted() {
    let (_, records) = stores_in("conformance/check")
        .into_iter()
        .find(|(n, _)| n.ends_with("short-body.smy"))
        .expect("the fixture");
    let store = Store::from_records(records);
    let checked = Checked::new(&store, &CheckOptions::default());
    let fine = CheckOptions::default().with_granularity(smysl_core::GranularityProfile::fine());
    let got = checked.with_batch(&store, Vec::new(), &fine);
    assert_eq!(
        got.path,
        Route::Full(smysl_check::incremental::Fallback::Options)
    );
    assert_eq!(got.report, check(&store, fine.clone()));
    let mut checked = checked;
    checked.advance(got);
    assert_eq!(checked.report(), &check(&store, fine));
}

// ---- generated stores --------------------------------------------------------------------------

fn agent(n: usize) -> AgentId {
    AgentId::new(format!("human:a{}", n % 4)).unwrap()
}

/// A store of kernel units with grounds, edges of every kernel kind (cycles included) and
/// attestations at every rung. Later AC-8 steps grow it as each incremental path lands.
fn generate(rng: &mut Rng, size: usize) -> Vec<Record> {
    let mut records = Vec::new();
    let mut uids: Vec<Uid> = Vec::new();
    for i in 0..size {
        let gist = format!("generated unit {}", rng.below(10_000));
        let built = match rng.below(4) {
            0 => UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Measured)
                .source(SourceRef::new(SourceKind::Metric, "m"))
                .build(),
            1 => UnitCoreBuilder::new(KernelType::Hypothesis, gist, Status::Speculative).build(),
            _ if uids.is_empty() => {
                UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative).build()
            }
            _ => {
                let n = 1 + rng.below(uids.len().min(3));
                let mut grounds: Vec<Uid> = (0..n).map(|_| uids[rng.below(uids.len())]).collect();
                // Sometimes a ground the store does not hold yet, or never will.
                if rng.chance(8) {
                    grounds.push(Uid::from_bytes([rng.below(256) as u8; 32]));
                }
                let status = [Status::Inferred, Status::Derived, Status::Measured][rng.below(3)];
                UnitCoreBuilder::new(KernelType::Claim, gist, status)
                    .grounds(grounds)
                    .build()
            }
        };
        let Ok(core) = built else { continue };
        let uid = canonical_uid(&core);
        uids.push(uid);
        records.push(Record::Unit(core));
        for _ in 0..rng.below(3) {
            let a = agent(i + rng.below(3));
            records.push(Record::Attestation(Attestation::new(
                uid,
                a.clone(),
                [Op::Authored, Op::Imported, Op::Transformed][rng.below(3)],
                [Rung::Computed, Rung::Document, Rung::Web, Rung::Model][rng.below(4)],
                Hlc::new(rng.below(100) as u64, rng.below(4) as u32, a),
            )));
        }
    }
    let kinds = [
        RelKind::Rebuts,
        RelKind::Supersedes,
        RelKind::Causes,
        RelKind::Sequences,
        RelKind::Elaborates,
        RelKind::Retracts,
    ];
    for _ in 0..size {
        if uids.len() < 2 {
            break;
        }
        let from = uids[rng.below(uids.len())];
        let to = uids[rng.below(uids.len())];
        if from != to {
            records.push(Record::Relation(Relation::new(
                kinds[rng.below(kinds.len())].clone(),
                from,
                to,
            )));
        }
    }
    records
}

#[test]
fn the_kept_check_agrees_with_the_full_check_on_generated_stores() {
    let opts = CheckOptions::default();
    let mut rng = Rng(17);
    for case in 0..60 {
        let size = 2 + rng.below(30);
        let records = generate(&mut rng, size);
        for _ in 0..3 {
            deal(&format!("generated {case}"), &records, &mut rng, &opts);
        }
    }
}
