//! The scale ladder (SMYSL-2.8 §5.4): the §2.8 in-process stages on the `graph` corpus.
//!
//! ```text
//! cargo test --release --test scale_ladder -- --ignored --nocapture
//! SMYSL_SCALE=31102,155510 cargo test --release --test scale_ladder -- --ignored --nocapture
//! ```
//!
//! A measurement, not a gate, for the reason every `scaling.rs` in the workspace gives: timing
//! assertions on shared runners fail for reasons that have nothing to do with the code. It is
//! the baseline TX-P13a's per-step "within ±5%" is read against, and it prints a digest of every
//! derived thread so a port can be shown to change no bytes.
//!
//! **The corpus is §5.4's shape, not its bytes.** `gen_graph.py` writes surface text seeded by
//! Python's `random`; this builds the records directly with a fixed LCG. Same proportions: `n`
//! ten-word `prose` units (`cited`, a `doc` source), one `claim` (`derived`, `grounds` on three
//! earlier prose units) per ten, one `backs` from every claim after the first to an earlier one.
//! The surface step also writes a label binding per unit; this does not, so records are units
//! plus relations.

use std::time::Instant;

use smysl_core::{
    canonical_uid, hash_bytes, to_cbor, AgentId, KernelType, Record, RelKind, Relation, SourceKind,
    SourceRef, Status, ThreadId, ThreadSchema, Uid, UnitCoreBuilder,
};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_retrieve::{Query, Retriever};

const VOC: &str = "and the of to that in he shall unto for i his a lord they be is him not them \
                   it with all thou thy was god which my me said but ye their have thee from as \
                   are when this out were by you up made one king people house come day earth \
                   land son father";

struct Lcg(u64);

impl Lcg {
    fn below(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) % n as u64) as usize
    }
}

/// `n` prose units, a claim per ten, a `backs` per claim after the first.
pub fn graph(n: usize) -> Vec<Record> {
    let voc: Vec<&str> = VOC.split_whitespace().collect();
    let mut rng = Lcg(7);
    let sentence = |rng: &mut Lcg| {
        let mut w: Vec<String> = (0..10)
            .map(|_| voc[rng.below(voc.len())].to_string())
            .collect();
        w[0] = w[0][..1].to_uppercase() + &w[0][1..];
        format!("{}.", w.join(" "))
    };
    let mut records = Vec::new();
    let mut prose: Vec<Uid> = Vec::new();
    let mut claims: Vec<Uid> = Vec::new();
    for i in 0..n {
        let text = sentence(&mut rng);
        let u = UnitCoreBuilder::new(KernelType::Prose, text, Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, format!("bench:g#{i}")))
            .build()
            .unwrap();
        prose.push(canonical_uid(&u));
        records.push(Record::Unit(u));
        if i % 10 == 9 {
            let mut g: Vec<Uid> = (0..3).map(|_| prose[rng.below(i + 1)]).collect();
            g.sort();
            g.dedup();
            let text = sentence(&mut rng);
            let c = UnitCoreBuilder::new(KernelType::Claim, text, Status::Derived)
                .grounds(g)
                .build()
                .unwrap();
            claims.push(canonical_uid(&c));
            records.push(Record::Unit(c));
        }
    }
    for c in 1..claims.len() {
        let to = claims[rng.below(c)];
        records.push(Record::Relation(Relation::new(
            RelKind::Backs,
            claims[c],
            to,
        )));
    }
    records
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// Median of three.
fn median(f: impl Fn() -> f64) -> f64 {
    let mut v = [f(), f(), f()];
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[1]
}

fn short(h: [u8; 32]) -> String {
    h[..6].iter().map(|b| format!("{b:02x}")).collect()
}

const SCHEMAS: [ThreadSchema; 5] = [
    ThreadSchema::Analysis,
    ThreadSchema::Narrative,
    ThreadSchema::Brief,
    ThreadSchema::Qa,
    ThreadSchema::Plan,
];

#[test]
#[ignore = "a measurement, not a gate"]
fn ladder() {
    let sizes: Vec<usize> = std::env::var("SMYSL_SCALE")
        .unwrap_or_else(|_| "31102,155510".into())
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    for n in sizes {
        let records = graph(n);
        let units = records
            .iter()
            .filter(|r| matches!(r, Record::Unit(_)))
            .count();
        let bytes: Vec<u8> = records.iter().flat_map(to_cbor).collect();
        println!(
            "\ngraph-{n}: {units} units, {} relations, {} records, {:.1} MB CBOR",
            records.len() - units,
            records.len(),
            bytes.len() as f64 / 1e6
        );
        let t = Instant::now();
        let (decoded, _) = smysl_core::from_cbor_seq(&bytes).unwrap();
        println!("  {:<28} {:>9.1} ms", "from_cbor_seq", ms(t));
        let load = median(|| {
            let t = Instant::now();
            std::hint::black_box(Store::from_records(decoded.clone()));
            ms(t)
        });
        println!("  {:<28} {:>9.1} ms", "Store::from_records", load);
        let store = Store::from_records(decoded);

        let t = Instant::now();
        let report = smysl_check::check(&store, smysl_check::CheckOptions::default());
        println!(
            "  {:<28} {:>9.1} ms  ({} diagnostics)",
            "check",
            ms(t),
            report.diagnostics.len()
        );
        let t = Instant::now();
        let sal = salience(&store, &SalienceRequest::default());
        println!("  {:<28} {:>9.1} ms", "salience", ms(t));
        let mut req = smysl_pack::PackRequest::default();
        req.budget = 4000;
        let t = Instant::now();
        let packed = smysl_pack::pack(&store, &sal, &req).unwrap();
        println!(
            "  {:<28} {:>9.1} ms  ({} units)",
            "pack (budget 4000)",
            ms(t),
            packed.selection.len()
        );
        for schema in SCHEMAS {
            let opts = smysl_thread::DeriveOptions::new(
                ThreadId::new("t/ladder").unwrap(),
                AgentId::new("tool:ladder").unwrap(),
            );
            let t = Instant::now();
            let (thread, _) = smysl_thread::derive_thread(&store, schema, &opts);
            let took = ms(t);
            println!(
                "  {:<28} {:>9.1} ms  thread {}",
                format!("derive_thread ({schema:?})"),
                took,
                short(hash_bytes(&to_cbor(&Record::Thread(thread))))
            );
        }
        let t = Instant::now();
        let bm = smysl_retrieve::Bm25::index(&store);
        println!("  {:<28} {:>9.1} ms", "Bm25::index", ms(t));
        let t = Instant::now();
        let hits = bm.search(&Query::new("lord king", 10));
        println!(
            "  {:<28} {:>9.1} ms  ({} hits)",
            "Bm25 search",
            ms(t),
            hits.len()
        );
        let one = UnitCoreBuilder::new(KernelType::Prose, "One more.", Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, "bench:g#append"))
            .build()
            .unwrap();
        let mut grown = store.clone();
        let t = Instant::now();
        grown.append(&[Record::Unit(one)]).unwrap();
        println!("  {:<28} {:>9.1} ms", "append 1 record", ms(t));
        // The adjacency is lazy since TX-P13a step 3: the first read after an append pays for
        // it, and this is that bill, so the append line above is not read as the whole cost.
        let t = Instant::now();
        grown.prepare();
        println!("  {:<28} {:>9.1} ms", "first read after it", ms(t));
        // Staging (TX-P13a step 10): fifty new units checked against the store they would join.
        // Until that step this copied every record of the store into a new one first.
        let fifty: Vec<smysl_core::UnitCore> = (0..50)
            .map(|i| {
                UnitCoreBuilder::new(
                    KernelType::Prose,
                    format!("Staged unit {i}."),
                    Status::Cited,
                )
                .source(SourceRef::new(SourceKind::Doc, format!("bench:stage#{i}")))
                .build()
                .unwrap()
            })
            .collect();
        let attest = smysl_ingest::Attest::new(
            AgentId::new("tool:ladder").unwrap(),
            smysl_core::Rung::Document,
            smysl_core::Hlc::new(1, 0, AgentId::new("tool:ladder").unwrap()),
        );
        let t = Instant::now();
        let staged =
            smysl_ingest::stage::prepare(&store, fifty, Vec::new(), Default::default(), &attest);
        println!(
            "  {:<28} {:>9.1} ms  ({} records, {} diagnostics)",
            "stage 50 units",
            ms(t),
            staged.records().len(),
            staged.report.diagnostics.len()
        );
        let t = Instant::now();
        std::hint::black_box(store.units_in_observed_order());
        println!("  {:<28} {:>9.1} ms", "units_in_observed_order", ms(t));
        println!(
            "  state {}  record set {}",
            short(store.state_hash()),
            short(store.record_set_digest())
        );
    }
}

/// The log of graph-`n`, written once under `dir`, and its unit store built from it.
#[cfg(feature = "store-redb")]
fn graph_log(dir: &std::path::Path, n: usize) -> std::path::PathBuf {
    let path = dir.join(format!("graph-{n}.smy"));
    if !path.exists() {
        // Each record's encoding in order, which is `Store::log_bytes` of a store with no
        // repeats, without the store: at 10⁶ units the in-memory store is the 3 GB the redb
        // backend exists not to need, and building it here would be measuring the harness.
        use std::io::Write as _;
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        for r in graph(n) {
            f.write_all(&to_cbor(&r)).unwrap();
        }
        f.flush().unwrap();
    }
    drop(smysl::redb::RedbStore::open(&path, smysl::redb::RedbOptions::default()).unwrap());
    path
}

/// `check` on one backend, in this process: what [`peak_rss`]'s children run.
#[cfg(feature = "store-redb")]
fn check_on(backend: &str, log: &std::path::Path) -> usize {
    let mut opts = smysl_check::CheckOptions::default();
    // One pass alone, by its position in the pipeline: what finds which pass holds the peak.
    if let Ok(p) = std::env::var("SMYSL_RSS_PASS") {
        use smysl_check::Pass::*;
        let all = [
            Integrity,
            Shape,
            Closure,
            Granularity,
            Epistemics,
            Trust,
            Extension,
            CommitmentSupport,
            Library,
            Time,
        ];
        opts.only = match p.strip_prefix("..") {
            // `..k`: the pipeline up to and including pass k.
            Some(k) => all[..=k.parse::<usize>().unwrap()].to_vec(),
            None => vec![all[p.parse::<usize>().unwrap()]],
        };
    }
    match backend {
        "memory" => smysl_check::check(&Store::open(log).unwrap(), opts)
            .diagnostics
            .len(),
        _ => {
            let mut ropts = smysl::redb::RedbOptions::default();
            if let Ok(b) = std::env::var("SMYSL_RSS_CACHE") {
                ropts.core_cache_bytes = b.parse().unwrap();
            }
            let (s, _) = smysl::redb::RedbStore::open(log, ropts).unwrap();
            // Another stage in place of `check`, to say which one a whole run's peak is.
            match std::env::var("SMYSL_RSS_STAGE").as_deref() {
                Ok("salience") => salience(&s, &SalienceRequest::default()).scores.len(),
                Ok("derive_thread") => {
                    let t = smysl_thread::derive_thread(
                        &s,
                        ThreadSchema::Analysis,
                        &smysl_thread::DeriveOptions::new(
                            ThreadId::new("t/ladder").unwrap(),
                            AgentId::new("tool:ladder").unwrap(),
                        ),
                    );
                    std::hint::black_box(t);
                    0
                }
                Ok("bm25") => {
                    std::hint::black_box(smysl_retrieve::Bm25::index(&s));
                    0
                }
                _ => smysl_check::check(&s, opts).diagnostics.len(),
            }
        }
    }
}

/// Peak resident set of a process that opens `log` on `backend` and runs `check`, in bytes.
///
/// A child per measurement, because a peak is a property of a process: measured in this one it
/// would be the largest thing the ladder ever held. `time -l` on macOS reports bytes and GNU
/// `time -v` kilobytes; neither needs a dependency.
#[cfg(feature = "store-redb")]
fn peak_rss(backend: &str, log: &std::path::Path) -> Option<u64> {
    let exe = std::env::current_exe().ok()?;
    let mac = cfg!(target_os = "macos");
    let out = std::process::Command::new("/usr/bin/time")
        .arg(if mac { "-l" } else { "-v" })
        .arg(&exe)
        .args(["--exact", "backends", "--ignored", "--nocapture"])
        .env("SMYSL_RSS_CHILD", backend)
        .env("SMYSL_RSS_LOG", log)
        .output()
        .ok()?;
    let err = String::from_utf8_lossy(&out.stderr);
    err.lines().find_map(|l| {
        let l = l.trim();
        if mac {
            l.strip_suffix("maximum resident set size")?
                .trim()
                .parse()
                .ok()
        } else {
            l.strip_prefix("Maximum resident set size (kbytes):")?
                .trim()
                .parse::<u64>()
                .ok()
                .map(|kb| kb * 1024)
        }
    })
}

/// The ladder per backend (TX-P13b step 1): open, the read-heavy stages, the core cache's hit
/// rate, and the peak resident set of `check` — whose ratio is the step's exit (≤ 40% of the
/// in-memory backend on graph-155510).
///
/// ```text
/// cargo test --release --features store-redb --test scale_ladder -- --ignored --nocapture backends
/// SMYSL_SCALE=1000000 SMYSL_BACKENDS=redb cargo test --release --features store-redb \
///     --test scale_ladder -- --ignored --nocapture backends
/// ```
///
/// `SMYSL_BACKENDS=redb` runs the redb backend alone, never building the in-memory store: TX-P13b
/// step 2's exit is that the ladder at 10⁶ units completes on a 7 GB machine with it, which a
/// run that also held the 3 GB memory store beside it could not show.
#[cfg(feature = "store-redb")]
#[test]
#[ignore = "a measurement, not a gate"]
fn backends() {
    use smysl::redb::{RedbOptions, RedbStore};
    use smysl_graph::StoreRead;

    if let (Ok(backend), Ok(log)) = (
        std::env::var("SMYSL_RSS_CHILD"),
        std::env::var("SMYSL_RSS_LOG"),
    ) {
        let log = std::path::Path::new(&log);
        if std::env::var("SMYSL_RSS_OPEN_ONLY").is_ok() {
            // What the open holds, without `check`'s working set: the floor under the peak.
            if backend == "memory" {
                let s = Store::open(log).unwrap();
                s.prepare();
                std::hint::black_box(&s);
            } else {
                let (s, _) = RedbStore::open(log, RedbOptions::default()).unwrap();
                std::hint::black_box(s.adjacency());
            }
            return;
        }
        println!("{} diagnostics", check_on(&backend, log));
        return;
    }
    let sizes: Vec<usize> = std::env::var("SMYSL_SCALE")
        .unwrap_or_else(|_| "31102,155510".into())
        .split(',')
        .map(|s| s.trim().parse().unwrap())
        .collect();
    // `SMYSL_BACKENDS_DIR` keeps the logs and stores, for measuring a child by hand.
    let keep = std::env::var("SMYSL_BACKENDS_DIR").ok();
    let both = std::env::var("SMYSL_BACKENDS").as_deref() != Ok("redb");
    let dir = keep
        .clone()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("smysl-backends-{}", std::process::id()))
        });
    std::fs::create_dir_all(&dir).unwrap();
    for n in sizes {
        let log = graph_log(&dir, n);
        let store_bytes = std::fs::metadata(RedbStore::store_path(&log))
            .unwrap()
            .len();
        println!(
            "\ngraph-{n}: log {:.1} MB, unit store {:.1} MB",
            std::fs::metadata(&log).unwrap().len() as f64 / 1e6,
            store_bytes as f64 / 1e6
        );
        let t = Instant::now();
        let mut opts = RedbOptions::default();
        opts.force_rebuild = true;
        drop(RedbStore::open(&log, opts).unwrap());
        println!("  {:<34} {:>9.1} ms", "redb: build from the log", ms(t));

        let memory = both.then(|| {
            let t = Instant::now();
            let memory = Store::open(&log).unwrap();
            println!("  {:<34} {:>9.1} ms", "memory: Store::open", ms(t));
            memory
        });
        let t = Instant::now();
        let (redb, _) = RedbStore::open(&log, RedbOptions::default()).unwrap();
        println!("  {:<34} {:>9.1} ms", "redb: open, current", ms(t));
        if let Some(m) = &memory {
            assert_eq!(redb.record_set_digest(), m.record_set_digest());
        }

        let opts = smysl_check::CheckOptions::default();
        for (name, run) in [
            ("check", 0usize),
            ("salience", 1),
            ("derive_thread (Analysis)", 2),
            ("Bm25::index", 3),
            ("units_in_observed_order", 4),
        ] {
            let mut took = [0.0f64; 2];
            for (b, slot) in took.iter_mut().enumerate() {
                if b == 0 && memory.is_none() {
                    continue;
                }
                let t = Instant::now();
                macro_rules! on {
                    ($s:expr) => {
                        match run {
                            0 => drop(std::hint::black_box(smysl_check::check($s, opts.clone()))),
                            1 => drop(std::hint::black_box(salience(
                                $s,
                                &SalienceRequest::default(),
                            ))),
                            2 => drop(std::hint::black_box(smysl_thread::derive_thread(
                                $s,
                                ThreadSchema::Analysis,
                                &smysl_thread::DeriveOptions::new(
                                    ThreadId::new("t/ladder").unwrap(),
                                    AgentId::new("tool:ladder").unwrap(),
                                ),
                            ))),
                            3 => drop(std::hint::black_box(smysl_retrieve::Bm25::index($s))),
                            _ => drop(std::hint::black_box(
                                smysl_graph::StoreRead::units_in_observed_order($s),
                            )),
                        }
                    };
                }
                match (b, &memory) {
                    (0, Some(m)) => on!(m),
                    _ => on!(&redb),
                }
                *slot = ms(t);
            }
            match &memory {
                Some(_) => println!(
                    "  {:<34} {:>9.1} ms memory  {:>9.1} ms redb",
                    name, took[0], took[1]
                ),
                None => println!("  {:<34} {:>9.1} ms redb", name, took[1]),
            }
        }
        let c = redb.cache_stats();
        println!(
            "  core cache: {} hits, {} misses ({:.0}% hit), {} pinned, {} paged in {:.1} MB",
            c.hits,
            c.misses,
            100.0 * c.hits as f64 / (c.hits + c.misses).max(1) as f64,
            c.pinned,
            c.paged,
            c.paged_bytes as f64 / 1e6
        );
        drop((memory, redb));

        // One record appended, per backend (§5.4): the writer's whole append — the lock, the
        // log `fsync`ed, the store's commit — and the in-memory store's, which writes the log
        // and does not sync it. A fresh unit each run, so a kept directory gets a new record
        // rather than a duplicate.
        let one = |tag: &str| {
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            vec![Record::Unit(
                UnitCoreBuilder::new(
                    KernelType::Claim,
                    format!("appended by {tag} at {n}"),
                    Status::Speculative,
                )
                .build()
                .unwrap(),
            )]
        };
        let (mut shard, _) = smysl::redb::RedbShard::open_rw(&log, RedbOptions::default()).unwrap();
        let t = Instant::now();
        assert_eq!(shard.append(&one("redb")).unwrap().added, 1);
        let redb_append = ms(t);
        drop(shard);
        if both {
            let mut m = Store::open(&log).unwrap();
            let t = Instant::now();
            assert_eq!(m.append(&one("memory")).unwrap().added, 1);
            println!(
                "  {:<34} {:>9.1} ms memory  {:>9.1} ms redb",
                "single-record append",
                ms(t),
                redb_append
            );
        } else {
            println!(
                "  {:<34} {:>9.1} ms redb",
                "single-record append", redb_append
            );
        }

        let memory_rss = if both { peak_rss("memory", &log) } else { None };
        match (memory_rss, peak_rss("redb", &log)) {
            (Some(m), Some(r)) => println!(
                "  peak RSS of check: memory {:.1} MB, redb {:.1} MB — {:.0}%",
                m as f64 / 1e6,
                r as f64 / 1e6,
                100.0 * r as f64 / m as f64
            ),
            (None, Some(r)) => println!("  peak RSS of check: redb {:.1} MB", r as f64 / 1e6),
            _ => println!("  peak RSS of check: /usr/bin/time not available"),
        }
    }
    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
