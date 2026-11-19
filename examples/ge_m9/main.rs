//! GE-M9 (RFC SMYSL-3 §14, rule TL): does retention keep the round trip?
//!
//! The RFC's design is six months of GE-M1-scale data, compressed into a test, then: evict per
//! rule TL; export, import, and merge with a peer that has not evicted. **It blocks TL-P8 on any
//! P1 failure over records, or on any substrate resurrection that changes a unit.**
//!
//! **What this runs.** GE-M1's cluster is not on this machine, so the data is simulated, as
//! GE-M4's is, and the report says so:
//!
//! - `--series` gauges from one scraper, every `--interval` seconds with up to 999 ms of jitter,
//!   for `--days` days, with a stale marker now and then. They go into library A's substrate,
//!   one day at a time.
//! - Each series' units are committed once, then one exact window per series per day, attested
//!   by rule TW, as `tel window --yes` commits them.
//!
//! **The steps**, each checked:
//!
//! 1. **A, before.** Its records' digest (the rsd of `lib export --scope all`, with P1 and P3
//!    verified) and its units. Every window recomputes clean.
//! 2. **Peer B** is a copy of A, which keeps everything.
//! 3. **A evicts** everything older than `--keep` days, at the end of the data. Its records do
//!    not change, the windows of the evicted days are `SMY-W605`, and nothing is `SMY-E604`.
//! 4. **A exports** with its substrate, and **C**, a fresh library, imports it. C's records are
//!    A's, and C has the same gaps.
//! 5. **C merges B's export.** Its records are unchanged, every window recomputes to the unit
//!    it is (no `SMY-E604`), and none is `SMY-W605`.
//! 6. **A merges B's export.** The same, and A has no eviction record left.
//! 7. **Exchange is a set.** C imports A's export again and adds nothing. D imports B's export
//!    and then A's, the other order, and holds the same samples as B and as C.
//! 8. **A control**, so that a pass means the checks can fail: a peer holding one sample more,
//!    on an evicted day, merged into a copy of C's first state, must make that day's window
//!    `SMY-E604`, and still change no record.
//!
//! Exit 0 when every check holds, 1 otherwise. `make ge-m9` runs it with the RFC's six months.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use smysl::lib::{
    export_library, import, snapshot, verify_collection, write_collection, ExportOptions,
    ImportOptions, Input, Selection, With, INDEX_FILE,
};
use smysl::schemas::tel;
use smysl::series::redb::RedbBackend;
use smysl::series::{exchange, Sample, SeriesInfo, Substrate};
use smysl::stage::{prepare_attested, Attesting};
use smysl::tel::evidence;
use smysl::text::library::Library;
use smysl::{
    canonical_uid, AgentId, Attestation, Code, Decimal, Hlc, Op, RelKind, Relation, Rung,
    StoreRead, Uid, UnitCore,
};

const DAY: u64 = 86_400_000;
/// 2025-07-09T00:00Z: a UTC midnight, so a day of data is a day file.
const T0: u64 = 1_752_019_200_000;

/// splitmix64: a fixed, seedable sequence, so a run is reproducible from its arguments.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

struct Args {
    dir: PathBuf,
    series: u64,
    days: u64,
    interval_s: u64,
    keep: u64,
    seed: u64,
}

fn args() -> Args {
    let mut a = Args {
        dir: PathBuf::from("target/ge-m9"),
        series: 20,
        days: 182,
        interval_s: 60,
        keep: 30,
        seed: 9,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let v = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let n = || v.parse::<u64>().unwrap_or_else(|_| panic!("{flag}: `{v}`"));
        match flag.as_str() {
            "--dir" => a.dir = PathBuf::from(&v),
            "--series" => a.series = n(),
            "--days" => a.days = n(),
            "--interval" => a.interval_s = n(),
            "--keep" => a.keep = n(),
            "--seed" => a.seed = n(),
            _ => panic!("unknown flag {flag}"),
        }
    }
    assert!(a.keep < a.days, "--keep must be shorter than --days");
    a
}

/// Windows by rule TW; everything else as the tool's computation.
struct By {
    agent: AgentId,
    now: Hlc,
    tw: BTreeMap<Uid, (Op, Rung)>,
}

impl Attesting for By {
    fn for_unit(&self, uid: Uid) -> Attestation {
        let (op, rung) = self
            .tw
            .get(&uid)
            .copied()
            .unwrap_or((Op::Imported, Rung::Computed));
        Attestation::new(uid, self.agent.clone(), op, rung, self.now.clone())
    }
}

fn commit(lib: &mut Library, units: Vec<UnitCore>, relations: Vec<Relation>, at: u64) {
    let agent = AgentId::new("tool:smysl-tel").unwrap();
    let mut tw = BTreeMap::new();
    for u in &units {
        if u.schema.as_str() != tel::WINDOW {
            continue;
        }
        if let Some(f) = u
            .payload
            .as_deref()
            .and_then(|p| tel::window_fields(p).ok())
        {
            let (_, op, rung) = tel::rule_tw(&f).expect("a substrate window");
            tw.insert(canonical_uid(u), (op, rung));
        }
    }
    let by = By {
        agent: agent.clone(),
        now: Hlc::new(at, 0, agent),
        tw,
    };
    let staged = prepare_attested(
        lib.store(),
        units,
        relations,
        BTreeMap::new(),
        vec![tel::decl()],
        &by,
    );
    assert!(
        !staged.has_errors(),
        "a batch did not check clean: {:?}",
        staged.report.diagnostics
    );
    lib.append_records(&staged.records()).expect("the append");
}

fn series_path(root: &Path) -> PathBuf {
    root.join("index").join("series.redb")
}

fn substrate(root: &Path) -> Substrate<RedbBackend> {
    let p = series_path(root);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    Substrate::new(RedbBackend::open(&p).expect("series.redb"))
}

/// What a library is, for the checks: its records' digest and P1/P3, its units, and how its
/// windows recompute.
#[derive(Debug, Clone, PartialEq, Eq)]
struct State {
    rsd: [u8; 32],
    verified: bool,
    units: BTreeSet<Uid>,
    relations: usize,
    attestations: usize,
    w605: usize,
    e604: usize,
}

fn state(root: &Path) -> State {
    let lib = Library::open(root).expect("the library");
    let snap = snapshot(&lib, &With::new()).expect("a snapshot");
    let c = export_library(&lib, &snap, &Selection::all(&snap), &ExportOptions::new())
        .expect("an export");
    let s = lib.store();
    let units: BTreeSet<Uid> = StoreRead::units(s).map(|(u, _)| *u).collect();
    let windows: Vec<&UnitCore> = StoreRead::units(s)
        .map(|(_, u)| &u.core)
        .filter(|c| c.schema.as_str() == tel::WINDOW)
        .collect();
    let d = evidence::verify(windows, &substrate(root));
    State {
        rsd: c.rsd(),
        verified: verify_collection(&c).holds(),
        units,
        relations: s.relations().count(),
        attestations: StoreRead::units(s).map(|(_, u)| u.attestations.len()).sum(),
        w605: d.iter().filter(|d| d.code == Code::W605).count(),
        e604: d.iter().filter(|d| d.code == Code::E604).count(),
    }
}

/// The samples a substrate holds, as one digest: every (series, instrument)'s whole window.
fn samples(root: &Path) -> [u8; 32] {
    let sub = substrate(root);
    let mut all = Vec::new();
    for (s, _) in sub.series(None).unwrap() {
        for i in sub.instruments(&s).unwrap() {
            all.extend_from_slice(s.as_bytes());
            all.extend_from_slice(i.as_bytes());
            all.extend_from_slice(&sub.window(&s, &i, 0, u64::MAX).unwrap().digest);
        }
    }
    smysl::hash_bytes(&all)
}

/// `lib export --scope all --with-substrate -o out`.
fn export_to(root: &Path, out: &Path) -> (bool, exchange::Listing) {
    let lib = Library::open(root).unwrap();
    let snap = snapshot(&lib, &With::new()).unwrap();
    let c = export_library(&lib, &snap, &Selection::all(&snap), &ExportOptions::new()).unwrap();
    let ok = verify_collection(&c).holds();
    write_collection(&c, out).unwrap();
    let listing = exchange::export(&substrate(root), &out.join(exchange::DIR)).unwrap();
    (ok, listing)
}

/// `lib import exp --accept`, then its substrate.
fn import_from(root: &Path, exp: &Path) -> exchange::ImportReport {
    let mut lib = Library::create(root).unwrap();
    let r = import(
        &mut lib,
        &[Input::Index(exp.join(INDEX_FILE))],
        &ImportOptions::new().accept(true),
    )
    .expect("the import");
    assert!(
        r.files.iter().all(|f| !f.rejected),
        "an export file was rejected"
    );
    drop(lib);
    exchange::import(&mut substrate(root), &exp.join(exchange::DIR)).expect("the substrate")
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if e.file_type().unwrap().is_dir() {
            copy_dir(&src, &dst);
        } else {
            std::fs::copy(&src, &dst).unwrap();
        }
    }
}

fn walk(p: &Path) -> u64 {
    match std::fs::read_dir(p) {
        Ok(d) => d.flatten().map(|e| walk(&e.path())).sum(),
        Err(_) => std::fs::metadata(p).map(|m| m.len()).unwrap_or(0),
    }
}

struct Verdict {
    failed: Vec<String>,
}

impl Verdict {
    fn check(&mut self, what: &str, ok: bool) {
        println!("  {}  {what}", if ok { "ok  " } else { "FAIL" });
        if !ok {
            self.failed.push(what.to_string());
        }
    }
}

fn main() {
    let a = args();
    let _ = std::fs::remove_dir_all(&a.dir);
    let (lib_a, lib_b, lib_c, lib_d) = (
        a.dir.join("a"),
        a.dir.join("b"),
        a.dir.join("c"),
        a.dir.join("d"),
    );
    let (exp_a, exp_b) = (a.dir.join("exp-a"), a.dir.join("exp-b"));
    let start = Instant::now();
    println!("GE-M9 (RFC SMYSL-3 §14, rule TL): does retention keep the round trip?");
    println!(
        "simulated: GE-M1's cluster is not here. {} gauges, every {} s, {} days; A keeps {} days",
        a.series, a.interval_s, a.days, a.keep
    );

    // The data, and A's records.
    let mut lib = Library::create(&lib_a).expect("library A");
    let inst_unit = tel::instrument("scraper", "prom-scrape:ge-m9/h:9100").unwrap();
    let inst = canonical_uid(&inst_unit);
    let keys: Vec<String> = (0..a.series)
        .map(|n| format!("ge_m9_gauge{{n=\"{n}\"}}"))
        .collect();
    let series: Vec<(Uid, UnitCore)> = keys
        .iter()
        .map(|k| {
            let u = tel::series("prom", k).unwrap();
            (canonical_uid(&u), u)
        })
        .collect();
    let mut first = vec![inst_unit];
    first.extend(series.iter().map(|(_, u)| u.clone()));
    commit(&mut lib, first, Vec::new(), T0);
    let mut rng = Rng(a.seed);
    let mut level: Vec<i64> = (0..a.series).map(|_| rng.below(1000) as i64).collect();
    let mut sub = substrate(&lib_a);
    let mut n_samples = 0u64;
    let step = a.interval_s * 1000;
    for day in 0..a.days {
        let from = T0 + day * DAY;
        let mut windows = Vec::new();
        let mut edges = Vec::new();
        for (i, (uid, _)) in series.iter().enumerate() {
            let mut day_samples = Vec::new();
            let mut t = from;
            while t < from + DAY {
                let at = t + rng.below(1000);
                if rng.below(2000) == 0 {
                    day_samples.push(Sample::stale(at));
                } else {
                    if rng.below(20) == 0 {
                        level[i] += rng.below(21) as i64 - 10;
                    }
                    day_samples.push(Sample::value(at, Decimal::from_i64(level[i])));
                }
                t += step;
            }
            n_samples += day_samples.len() as u64;
            let info = SeriesInfo {
                scheme: "prom".into(),
                key: keys[i].clone(),
                class: "gauge".into(),
            };
            sub.add(uid, &inst, &info, &day_samples)
                .expect("the samples");
            let w = evidence::materialise(&sub, uid, &inst, from, from + DAY).expect("a window");
            edges.push(Relation::new(
                RelKind::Extension(tel::OF_SERIES.to_string()),
                canonical_uid(&w.unit),
                *uid,
            ));
            windows.push(w.unit);
        }
        commit(&mut lib, windows, edges, from + DAY);
    }
    drop(sub);
    drop(lib);
    let end = T0 + a.days * DAY;
    println!(
        "data      {n_samples} samples, {} windows, written in {:.1} s",
        a.series * a.days,
        start.elapsed().as_secs_f64()
    );

    let mut v = Verdict { failed: Vec::new() };
    let gaps = (a.series * (a.days - a.keep)) as usize;

    println!("1. A, before");
    let a0 = state(&lib_a);
    v.check("P1 and P3 hold over A's records", a0.verified);
    v.check(
        "every window recomputes clean",
        a0.w605 == 0 && a0.e604 == 0,
    );

    println!("2. peer B, a copy of A that keeps everything");
    copy_dir(&lib_a, &lib_b);
    let samples_b = samples(&lib_b);

    println!("3. A evicts what is older than {} days", a.keep);
    let r = substrate(&lib_a)
        .evict(end - a.keep * DAY, end)
        .expect("the eviction");
    println!(
        "   {} chunk(s), {} sample(s) of {} series",
        r.chunks, r.samples, r.series
    );
    let a1 = state(&lib_a);
    let records = |s: &State| (s.rsd, s.units.clone(), s.relations, s.attestations);
    v.check(
        "A's records are unchanged (rule TL)",
        records(&a1) == records(&a0),
    );
    v.check(
        &format!(
            "the {} evicted days' windows are SMY-W605 ({} of {gaps})",
            a.days - a.keep,
            a1.w605
        ),
        a1.w605 == gaps,
    );
    v.check("no window is SMY-E604", a1.e604 == 0);

    println!("4. A exports with its substrate; C imports it");
    let (ok, listing) = export_to(&lib_a, &exp_a);
    println!(
        "   {} day file(s), {} listed evicted, {:.1} MB",
        listing.held.len(),
        listing.evicted.len(),
        walk(&exp_a.join(exchange::DIR)) as f64 / 1e6
    );
    v.check("the export verifies: P1 and P3", ok);
    v.check(
        "it lists every evicted day",
        listing.evicted.len() as u64 == a.series * (a.days - a.keep),
    );
    import_from(&lib_c, &exp_a);
    let c1 = state(&lib_c);
    v.check("C's records are A's", records(&c1) == records(&a0));
    v.check("C has A's gaps", c1.w605 == gaps && c1.e604 == 0);

    println!("5. C merges B's export");
    let (ok, _) = export_to(&lib_b, &exp_b);
    v.check("B's export verifies: P1 and P3", ok);
    let r = import_from(&lib_c, &exp_b);
    println!("   {} sample(s) back, {} already held", r.added, r.already);
    let c2 = state(&lib_c);
    v.check(
        "C's records are unchanged by the resurrection",
        records(&c2) == records(&a0),
    );
    v.check(
        "every window recomputes to the unit it is: no SMY-E604, no SMY-W605",
        c2.e604 == 0 && c2.w605 == 0,
    );
    v.check("C holds B's samples", samples(&lib_c) == samples_b);

    println!("6. A merges B's export");
    import_from(&lib_a, &exp_b);
    let a2 = state(&lib_a);
    v.check("A's records are unchanged", records(&a2) == records(&a0));
    v.check(
        "every window recomputes clean",
        a2.e604 == 0 && a2.w605 == 0,
    );
    v.check(
        "no eviction record is left",
        substrate(&lib_a).evictions().unwrap().is_empty(),
    );
    v.check("A holds B's samples", samples(&lib_a) == samples_b);

    println!("7. exchange is a set");
    let r = import_from(&lib_c, &exp_a);
    v.check("C imports A's export again and adds nothing", r.added == 0);
    v.check(
        "C's records are still A's",
        records(&state(&lib_c)) == records(&a0),
    );
    import_from(&lib_d, &exp_b);
    import_from(&lib_d, &exp_a);
    let d = state(&lib_d);
    v.check(
        "D, in the other order, has the same records and samples",
        records(&d) == records(&a0) && samples(&lib_d) == samples_b,
    );

    println!("8. control: a peer whose substrate differs by one sample");
    // E holds A's records and evicted substrate, and merges a peer that holds one sample B never
    // did, on an evicted day. The resurrection is then not the substrate the windows were
    // computed from, and the check this experiment rests on must say so.
    let (lib_e, lib_f, exp_f) = (a.dir.join("e"), a.dir.join("f"), a.dir.join("exp-f"));
    import_from(&lib_e, &exp_a);
    copy_dir(&lib_b, &lib_f);
    let s0 = &series[0].0;
    let info = SeriesInfo {
        scheme: "prom".into(),
        key: keys[0].clone(),
        class: "gauge".into(),
    };
    substrate(&lib_f)
        .add(
            s0,
            &inst,
            &info,
            &[Sample::value(T0 + 1, Decimal::from_i64(-1))],
        )
        .unwrap();
    export_to(&lib_f, &exp_f);
    import_from(&lib_e, &exp_f);
    let e = state(&lib_e);
    v.check(
        &format!("the changed window is caught: {} SMY-E604", e.e604),
        e.e604 == 1,
    );
    v.check("and E's records are still A's", records(&e) == records(&a0));

    println!(
        "on disk   {:.1} MB under {}, {:.1} s in all",
        walk(&a.dir) as f64 / 1e6,
        a.dir.display(),
        start.elapsed().as_secs_f64()
    );
    if v.failed.is_empty() {
        println!("verdict: retention keeps the round trip; TL-P8's gate holds");
    } else {
        println!(
            "verdict: BLOCKS TL-P8: {} check(s) failed: {}",
            v.failed.len(),
            v.failed.join("; ")
        );
        std::process::exit(1);
    }
}
