//! A stand-in for OKO's proof of concept (RFC SMYSL-3 §24, TL-P6's exit criterion): one
//! on-call rotation's ledger, written through the library API the way §24 says OKO will.
//!
//! **What this is not.** OKO is a separate project, and its code is not here. TL-P6's criterion
//! is that *OKO* writes a rotation ledger, and that stays owed. This drives every mechanism §24
//! maps an OKO decision to, so that when OKO arrives the smysl side has run end to end at a
//! rotation's size:
//!
//! | OKO decision | here |
//! |---|---|
//! | one ledger store per on-call rotation | one library, `<dir>/<rotation>`; `rotation` on each investigation |
//! | investigations as views | an `x.tel/investigation`, and a View rooted at it (D-T8) |
//! | humans by configured handle; model agents by provider and model hash | `human:<handle>` and `model:ollama/<hash>` attest what each wrote |
//! | full redacted command output kept | each probe's and action's output is a part, redacted before it is stored |
//! | confirmation on unsafe commands | `x.tel/action.confirmed_by` |
//! | Zabbix 6/7/8 demo source | `fixtures/tel/zabbix/`'s `trend.get` export, handed to `zabbix/1` |
//!
//! **What it writes.** The Zabbix trends first: series, their metas, the server and the agents,
//! lineage, and the `cited` platform windows (rule TW). Then two statistics (RFC SMYSL-5,
//! TI-P4) over GE-I1's driver and follower (`fixtures/tel/info/`): an entropy, and a lag scan
//! of their mutual information with 50 surrogates, `derived` as rule TI asks. Then
//! `--investigations` incidents, each one ledger operation at a time, as OKO appends them:
//!
//! - open (a person or a model agent), and its view;
//! - consult two to five windows;
//! - one or two probes, with redacted output;
//! - one to three hypotheses, each `explains` a consulted window and quotes its `max`. One in
//!   `--fabricate` of them quotes a number the window does not hold, as a model that invents a
//!   value would; rule TQ caps it at `speculative`;
//! - sometimes an action, confirmed by a human when the agent was a model;
//! - close.
//!
//! The first investigation also consults the two statistics, and writes a hypothesis quoting
//! the entropy's `value`, the scan's `null_ge` and an element of its `values` at their stored
//! six places, which verifies, and one quoting the entropy to two places, as a model that
//! rounds would: a planted wrong quote, which rule TQ must catch as it catches the others.
//!
//! **What it measures.** AC-8 is on OKO's critical path (§24): OKO appends one record batch and
//! reads at once. Each operation is timed as a commit (stage, check, append) and the read that
//! follows it, and the report gives the percentiles over the first and the last tenth of the
//! run, so growth with the ledger's size shows.
//!
//! Each operation is staged with `prepare_checked`, against the store's kept check (AC-8, RFC
//! SMYSL-3 §17.1). `--verify` also checks it in full and asserts the two agree; the oracle's time
//! is not counted.
//!
//! **Checks at the end:** the library is reopened, `smysl check` runs over it, every
//! fabricated quote is a `SMY-W606` and nothing else is, and one investigation's report (§11.2,
//! §22.2) is printed.
//!
//! `make oko-ledger` runs it with the defaults: 1,500 investigations, about 2 × 10⁴ records,
//! under `target/oko-ledger`, removed afterwards.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use smysl::incremental::{self, Checked};
use smysl::schemas::tel;
use smysl::stage::{prepare_checked, Attesting};
use smysl::tel::investigate as inv;
use smysl::text::library::Library;
use smysl::{
    canonical_uid, AgentId, Attestation, CheckOptions, Hlc, Op, PartText, Record, RelKind,
    Relation, Rung, StoreRead, Uid, UnitCore,
};

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

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// In `lo..=hi`.
    fn between(&mut self, lo: usize, hi: usize) -> usize {
        lo + self.below(hi - lo + 1)
    }
}

struct Args {
    dir: PathBuf,
    rotation: String,
    investigations: usize,
    fabricate: usize,
    seed: u64,
    /// Hold AC-8's contract after every operation: the kept check against the full one.
    verify: bool,
}

fn args() -> Args {
    let mut a = Args {
        dir: PathBuf::from("target/oko-ledger"),
        rotation: "sre-eu".into(),
        investigations: 1_500,
        fabricate: 5,
        seed: 24,
        verify: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        if flag == "--verify" {
            a.verify = true;
            continue;
        }
        let v = it.next().unwrap_or_else(|| panic!("{flag} needs a value"));
        let n = || v.parse::<u64>().unwrap_or_else(|_| panic!("{flag}: `{v}`"));
        match flag.as_str() {
            "--dir" => a.dir = PathBuf::from(&v),
            "--rotation" => a.rotation = v.clone(),
            "--investigations" => a.investigations = n() as usize,
            "--fabricate" => a.fabricate = n().max(1) as usize,
            "--seed" => a.seed = n(),
            other => panic!("unknown argument {other}"),
        }
    }
    a
}

/// 2026-01-16T10:00:00Z: the rotation starts here, an investigation every 20 minutes.
const T0: u64 = 1_768_557_600_000;
const MIN: u64 = 60_000;

fn fixture(rel: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel/zabbix")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// One agent for a whole batch, at one op and rung, except the windows, which rule TW attests.
struct By {
    agent: AgentId,
    op: Op,
    rung: Rung,
    now: Hlc,
    tw: BTreeMap<Uid, (Op, Rung)>,
}

impl Attesting for By {
    fn for_unit(&self, uid: Uid) -> Attestation {
        let (op, rung) = self.tw.get(&uid).copied().unwrap_or((self.op, self.rung));
        Attestation::new(uid, self.agent.clone(), op, rung, self.now.clone())
    }
}

/// The ledger, and the timings of what was done to it.
struct Ledger {
    lib: Library,
    commit_us: Vec<u128>,
    /// The commit's first half: staging and checking the batch against the store.
    stage_us: Vec<u128>,
    read_us: Vec<u128>,
    records: usize,
    /// The store's check, kept (AC-8): each batch is checked against it.
    checked: Checked,
    /// Batches checked incrementally, and in full.
    paths: [usize; 2],
    verify: bool,
}

impl Ledger {
    /// Stage, check and append one batch, then read the first unit back, as OKO's next screen
    /// would.
    fn commit(
        &mut self,
        by: &By,
        units: Vec<UnitCore>,
        relations: Vec<Relation>,
        extra: Vec<Record>,
    ) -> Vec<Uid> {
        let uids: Vec<Uid> = units.iter().map(canonical_uid).collect();
        let t = Instant::now();
        let mut staged = prepare_checked(
            self.lib.store(),
            &self.checked,
            units,
            relations,
            BTreeMap::new(),
            vec![tel::decl()],
            by,
            extra,
        );
        assert!(
            !staged.has_errors(),
            "a ledger operation did not check clean: {:?}",
            staged.report.diagnostics
        );
        let staged_us = t.elapsed().as_micros();
        let records = staged.records();
        let batch = staged.checked.take().expect("prepare_checked checks");
        // Not timed: the commit is what OKO pays, and the oracle is not part of it.
        let paused = Instant::now();
        if self.verify {
            let full = smysl::check(
                &smysl::Overlay::new(self.lib.store(), records.clone()),
                CheckOptions::default(),
            );
            assert_eq!(
                batch.report, full,
                "AC-8: the kept check is not the full one"
            );
        }
        self.paths[usize::from(batch.path != incremental::Path::Incremental)] += 1;
        self.records += records.len();
        let unpaid = paused.elapsed();
        self.lib.append_records(&records).expect("the append");
        self.checked.advance(batch);
        self.commit_us.push((t.elapsed() - unpaid).as_micros());
        self.stage_us.push(staged_us);
        let t = Instant::now();
        if let Some(u) = uids.first() {
            assert!(StoreRead::get(self.lib.store(), u).is_some(), "read back");
        }
        self.read_us.push(t.elapsed().as_micros());
        uids
    }

    /// A probe's or an action's output, redacted and kept as a part (§11.3, §24).
    fn output(&self, text: &str) -> String {
        let redacted = text
            .split_whitespace()
            .map(|w| match w.split_once('=') {
                Some((k, _)) if ["token", "password", "secret"].contains(&k) => {
                    format!("{k}=[REDACTED]")
                }
                _ => w.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        let part = PartText::new(redacted.into_bytes());
        self.lib.objects().put_part(&part).expect("the part");
        part.tid.canonical()
    }
}

fn pct(v: &[u128], p: f64) -> u128 {
    let mut s = v.to_vec();
    s.sort_unstable();
    s[((s.len() as f64 - 1.0) * p).round() as usize]
}

fn main() {
    let a = args();
    let root = a.dir.join(&a.rotation);
    let _ = std::fs::remove_dir_all(&a.dir);
    std::fs::create_dir_all(&a.dir).expect("the directory");
    let lib = Library::create(&root).expect("a library per rotation");
    let checked = Checked::new(lib.store(), &CheckOptions::default());
    let mut ledger = Ledger {
        lib,
        commit_us: Vec::new(),
        stage_us: Vec::new(),
        read_us: Vec::new(),
        records: 0,
        checked,
        paths: [0, 0],
        verify: a.verify,
    };
    println!(
        "OKO ledger stand-in (RFC SMYSL-3 §24): OKO itself is not here; this is smysl's side."
    );
    println!(
        "rotation {}, {} investigations, seed {}, under {}",
        a.rotation,
        a.investigations,
        a.seed,
        root.display()
    );

    // The Zabbix demo source: OKO calls the API and hands the export to `zabbix/1`.
    let read = smysl::tel::readers::zabbix_history::read_trends(
        &fixture("trends.json"),
        &fixture("items.json"),
        "7.0",
    )
    .expect("the trends export");
    let tr = smysl::tel::zabbix_history::record_trends(&read, "zabbix-server:zbx-main")
        .expect("trends as windows");
    let tool = AgentId::new("tool:smysl-tel").unwrap();
    let windows: Vec<UnitCore> = tr.windows.iter().map(|w| w.unit()).collect();
    let mut tw = BTreeMap::new();
    let mut rels = tr.lineage.relations.clone();
    for w in &windows {
        let f = tel::window_fields(w.payload.as_deref().unwrap()).unwrap();
        let (_, op, rung) = tel::rule_tw(&f).expect("a platform window");
        tw.insert(canonical_uid(w), (op, rung));
        let s: Uid = f.series.unwrap().parse().unwrap();
        rels.push(Relation::new(
            RelKind::Extension(tel::OF_SERIES.to_string()),
            canonical_uid(w),
            s,
        ));
    }
    let mut units = vec![tr.instrument.clone()];
    units.extend(tr.lineage.origins.values().cloned());
    units.extend(tr.series.iter().cloned());
    units.extend(tr.metas.iter().cloned());
    units.extend(windows.iter().cloned());
    let by = By {
        agent: tool.clone(),
        op: Op::Imported,
        rung: Rung::Document,
        now: Hlc::new(T0, 0, tool),
        tw,
    };
    ledger.commit(&by, units, rels, Vec::new());
    let evidence: Vec<(Uid, String)> = windows
        .iter()
        // A trend whose `value_max` is out of `dec`'s range has no `max` (SMY-W616): nothing
        // to quote.
        .filter_map(|w| {
            tel::quoted_value(w.payload.as_deref().unwrap(), "max", None)
                .map(|max| (canonical_uid(w), max))
        })
        .collect();
    println!(
        "zabbix/1  {} series, {} cited windows, committed",
        tr.series.len(),
        evidence.len()
    );

    // Two statistics (RFC SMYSL-5): their series and instruments, then the units, by rule TI.
    let st = statistics();
    let tool = AgentId::new("tool:smysl-tel").unwrap();
    let by = |op, rung| By {
        agent: tool.clone(),
        op,
        rung,
        now: Hlc::new(T0, 0, tool.clone()),
        tw: BTreeMap::new(),
    };
    ledger.commit(
        &by(Op::Imported, Rung::Document),
        st.entities.clone(),
        st.lineage.clone(),
        Vec::new(),
    );
    ledger.commit(
        &by(Op::Transformed, Rung::Computed),
        st.units.clone(),
        st.relations.clone(),
        Vec::new(),
    );
    let field = |u: Uid, f: &str, i: Option<usize>| {
        let c = st.units.iter().find(|c| canonical_uid(c) == u).unwrap();
        tel::quoted_value(c.payload.as_deref().unwrap(), f, i).expect("a stored field")
    };
    let entropy = field(st.entropy, "value", None);
    println!(
        "tel info  ent/1 {entropy} bits; mi/1 over -5..5, {} at lag {}, null {} of {}: committed",
        field(st.scan, "value", None),
        field(st.scan, "lag", None),
        field(st.scan, "null_ge", None),
        field(st.scan, "null_b", None)
    );

    let humans = ["alice", "bob", "chen", "dana"];
    let model = AgentId::new("model:ollama/qwen2.5-7b-845dbda0ea48").unwrap();
    let mut r = Rng(a.seed);
    let mut fabricated = 0usize;
    let mut hypotheses = 0usize;
    let mut first_inv = None;
    let started = Instant::now();
    for n in 0..a.investigations {
        let t0 = T0 + n as u64 * 20 * MIN;
        let t = std::cell::Cell::new(t0);
        let tick = || {
            t.set(t.get() + 1_000);
            t.get()
        };
        let human = humans[r.below(humans.len())];
        let (agent, is_model) = if r.below(3) == 0 {
            (model.clone(), true)
        } else {
            (AgentId::new(format!("human:{human}")).unwrap(), false)
        };
        let at = |when: u64, op: Op, rung: Rung| By {
            agent: agent.clone(),
            op,
            rung,
            now: Hlc::new(when, 0, agent.clone()),
            tw: BTreeMap::new(),
        };
        let authored = |when| at(when, Op::Authored, Rung::Document);
        let ran = |when| at(when, Op::Imported, Rung::Computed);

        // Open, with its view.
        let question = format!("incident {n}: why is DB 01 slow?");
        let open = inv::open(&question, tick(), Some(&a.rotation));
        let i = canonical_uid(&open);
        first_inv.get_or_insert(i);
        let view = Record::View(inv::view(i));
        ledger.commit(
            &authored(t.get()),
            vec![open.clone()],
            Vec::new(),
            vec![view],
        );

        // Consult.
        let k = r.between(2, 5);
        let consulted: Vec<(Uid, String)> = (0..k)
            .map(|_| evidence[r.below(evidence.len())].clone())
            .collect();
        let edges = consulted
            .iter()
            .map(|(w, _)| inv::consulted(i, *w))
            .collect();
        ledger.commit(&authored(tick()), Vec::new(), edges, Vec::new());

        // The first investigation reads the statistics too, and quotes them: rightly, and then
        // rounded to two places, which the entropy does not hold.
        if n == 0 {
            let edges = [st.entropy, st.scan]
                .iter()
                .map(|u| inv::consulted(i, *u))
                .collect();
            ledger.commit(&authored(tick()), Vec::new(), edges, Vec::new());
            let rounded = format!("{:.2}", entropy.parse::<f64>().unwrap());
            assert_ne!(rounded, entropy, "the entropy has more than two places");
            for (claim, quotes) in [
                (
                    "the follower carries the driver three minutes late",
                    vec![
                        format!("{}#value={entropy}", st.entropy.canonical()),
                        format!(
                            "{}#null_ge={}",
                            st.scan.canonical(),
                            field(st.scan, "null_ge", None)
                        ),
                        format!(
                            "{}#values[2]={}",
                            st.scan.canonical(),
                            field(st.scan, "values", Some(2))
                        ),
                    ],
                ),
                (
                    "the follower's entropy is low",
                    vec![format!("{}#value={rounded}", st.entropy.canonical())],
                ),
            ] {
                let u = inv::hypothesis(claim, &quotes, None, vec![st.entropy, st.scan]);
                let hu = canonical_uid(&u);
                ledger.commit(
                    &authored(tick()),
                    vec![u],
                    vec![inv::during(hu, i)],
                    Vec::new(),
                );
            }
            fabricated += 1;
        }

        // Probe.
        for p in 0..r.between(1, 2) {
            let out = ledger.output(&format!(
                "$ mysql -h db01 -e 'show processlist' token=s3cr3t-{n}-{p}\n\
                 Id User Host db Command Time State Info\n{} rows",
                r.between(10, 400)
            ));
            let u = inv::probe("mysql show processlist", tick(), Some(&out));
            let d = inv::during(canonical_uid(&u), i);
            ledger.commit(&ran(t.get()), vec![u], vec![d], Vec::new());
        }

        // Hypothesise.
        for h in 0..r.between(1, 3) {
            let (w, max) = &consulted[r.below(consulted.len())];
            hypotheses += 1;
            // `is_multiple_of` is 1.87, after the MSRV.
            let value = if hypotheses.checked_rem(a.fabricate) == Some(0) {
                fabricated += 1;
                // What a model that invents a number writes: plausible, and not what was read.
                format!("{}9", max.trim_end_matches('0'))
            } else {
                max.clone()
            };
            let quote = format!("{}#max={value}", w.canonical());
            let u = inv::hypothesis(
                &format!("incident {n}, hypothesis {h}: load peaked at {value}"),
                &[quote],
                Some(*w),
                vec![*w],
            );
            let hu = canonical_uid(&u);
            ledger.commit(
                &authored(tick()),
                vec![u],
                vec![inv::during(hu, i), inv::explains(hu, *w)],
                Vec::new(),
            );
        }

        // Act, sometimes; a model's unsafe command needs a person to confirm it.
        if r.below(3) == 0 {
            let confirmed = is_model.then_some(human);
            let out = ledger.output("$ systemctl restart mysqld password=hunter2\nok");
            let u = inv::action("systemctl restart mysqld", tick(), confirmed, Some(&out));
            let d = inv::during(canonical_uid(&u), i);
            ledger.commit(&ran(t.get()), vec![u], vec![d], Vec::new());
        }

        // Close.
        let (closed, sup) = inv::close(&open, t0 + 19 * MIN).expect("an investigation");
        ledger.commit(
            &authored(t0 + 19 * MIN),
            vec![closed],
            vec![sup],
            Vec::new(),
        );
    }
    let elapsed = started.elapsed();

    // The timings, early and late.
    let ops = ledger.commit_us.len();
    let tenth = (ops / 10).max(1);
    println!();
    println!(
        "{} ledger operations, {} records, in {:.1} s",
        ops,
        ledger.records,
        elapsed.as_secs_f64()
    );
    println!(
        "check   {} batches against the kept check, {} in full{}",
        ledger.paths[0],
        ledger.paths[1],
        if a.verify {
            "; every one equal to the full check (--verify)"
        } else {
            ""
        }
    );
    for (name, v) in [
        ("commit", &ledger.commit_us),
        ("  stage", &ledger.stage_us),
        ("read", &ledger.read_us),
    ] {
        let (early, late) = (&v[..tenth], &v[v.len() - tenth..]);
        println!(
            "{name:<7} first tenth p50 {:>7} µs p95 {:>7} µs | last tenth p50 {:>7} µs p95 {:>7} µs | max {:>8} µs",
            pct(early, 0.5),
            pct(early, 0.95),
            pct(late, 0.5),
            pct(late, 0.95),
            v.iter().max().unwrap()
        );
    }
    drop(ledger);

    // Reopen, and check what is there.
    let t = Instant::now();
    let lib = Library::open(&root).expect("the ledger reopens");
    let reopen = t.elapsed();
    let store = lib.store();
    let count = |schema: &str| {
        StoreRead::units(store)
            .filter(|(_, u)| u.core.schema.as_str() == schema)
            .count()
    };
    println!(
        "reopened in {:.0} ms: {} units, {} relations; {} investigations, {} hypotheses, {} probes, {} actions",
        reopen.as_secs_f64() * 1000.0,
        StoreRead::units(store).count(),
        store.relations().count(),
        count(tel::INVESTIGATION),
        count(tel::HYPOTHESIS),
        count(tel::PROBE),
        count(tel::ACTION)
    );
    let report = smysl::check(store, smysl::CheckOptions::default());
    let mut codes: BTreeMap<String, usize> = BTreeMap::new();
    for d in &report.diagnostics {
        *codes.entry(d.code.to_string()).or_default() += 1;
    }
    println!("check     {codes:?}");
    let w606 = codes.get("SMY-W606").copied().unwrap_or(0);
    println!("rule TQ   {fabricated} fabricated quote(s) written, {w606} caught (SMY-W606)");
    let mut ok = w606 == fabricated && codes.len() <= 1;
    let rotation_ok = StoreRead::units(store)
        .filter(|(_, u)| u.core.schema.as_str() == tel::INVESTIGATION)
        .all(|(_, u)| {
            smysl::surface::payload_to_object(u.core.payload.as_deref().unwrap())
                .ok()
                .and_then(|o| match o.get("tel:rotation").map(|v| v.value.clone()) {
                    Some(smysl::surface::hjson::HValue::Str(s)) => Some(s == a.rotation),
                    _ => None,
                })
                .unwrap_or(false)
        });
    ok &= rotation_ok;
    println!(
        "rotation  every investigation carries `{}`: {rotation_ok}",
        a.rotation
    );
    let size: u64 = walk(&root);
    println!("on disk   {:.1} MB", size as f64 / 1e6);
    if let Some(i) = first_inv {
        let effective = smysl::effective_status(store, smysl::RetractionPolicy::default());
        let units: BTreeMap<Uid, &UnitCore> = StoreRead::units(store)
            .map(|(u, c)| (*u, &c.core))
            .collect();
        let relations: Vec<&Relation> = store.relations().collect();
        let rep = inv::report(
            i,
            &units,
            &relations,
            |u| effective.get(u).unwrap_or(smysl::Status::Unfounded),
            60 * MIN,
        );
        println!(
            "report    first investigation: {} hypotheses ({} capped by rule TQ), {} probes, \
             {} actions, {} consulted, {} consulted and unused",
            rep.hypotheses.len(),
            rep.hypotheses.iter().filter(|h| !h.2.is_empty()).count(),
            rep.probes.len(),
            rep.actions.len(),
            rep.consulted.len(),
            rep.unused.len()
        );
    }
    println!(
        "verdict: {}",
        if ok {
            "ledger written and checked"
        } else {
            "FAILED"
        }
    );
    std::process::exit(if ok { 0 } else { 1 });
}

/// GE-I1's driver and follower in a substrate of their own, with their instruments and lineage,
/// and two statistics over the two hours before `T0`: the follower's `ent/1`, and its `mi/1`
/// scan against the driver.
struct Statistics {
    entities: Vec<UnitCore>,
    lineage: Vec<Relation>,
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    entropy: Uid,
    scan: Uid,
}

fn statistics() -> Statistics {
    use smysl::series::{MemBackend, Sample, SeriesInfo, Substrate};
    use smysl::tel::info::{run_on, Spec, With};
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/info/series.tsv");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let exporter = tel::instrument("exporter", "prom-target:ge-i1/sim:9100").unwrap();
    let scraper = tel::instrument("scraper", "prom-scrape:ge-i1/sim:9100").unwrap();
    let (origin, inst) = (canonical_uid(&exporter), canonical_uid(&scraper));
    let mut sub = Substrate::new(MemBackend::default());
    let mut entities = vec![exporter, scraper];
    let mut lineage = vec![tel::via(origin, inst)];
    let mut uid = BTreeMap::new();
    for key in ["info_driver", "info_follower"] {
        let rows: Vec<Vec<&str>> = text
            .lines()
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .filter(|f| f[0] == key)
            .collect();
        let samples: Vec<Sample> = rows
            .iter()
            .map(|f| {
                let v = smysl::Decimal::parse_token(f[3]).unwrap();
                Sample::new(f[2].parse().unwrap(), None, v, f[4].parse().unwrap()).unwrap()
            })
            .collect();
        let s = tel::series("prom", key).unwrap();
        let su = canonical_uid(&s);
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: key.into(),
            class: rows[0][1].into(),
        };
        sub.add(&su, &inst, &info, &samples).unwrap();
        lineage.push(tel::measured_by(su, origin));
        entities.push(s);
        uid.insert(key, su);
    }
    let (from, to) = (T0 - 120 * MIN, T0);
    let run = |recipe: &str, with: Option<With>| {
        let spec = Spec::parse(recipe).expect("a recipe");
        let o = run_on(&spec, &sub, &[uid["info_follower"]], None, with, from, to).unwrap();
        o.into_iter().next().unwrap().result.expect("a statistic")
    };
    let ent = run(
        r#"{ stat: ent/1, step: 60000, bins: "width:1", min_n: 100 }"#,
        None,
    );
    let mi = run(
        r#"{ stat: mi/1, step: 60000, bins: "rank:4", lag: "-5..5", null: 50, seed: 7 }"#,
        Some(With {
            series: uid["info_driver"],
            instrument: None,
        }),
    );
    let (entropy, scan) = (canonical_uid(&ent.unit), canonical_uid(&mi.unit));
    let mut relations = ent.relations;
    relations.extend(mi.relations);
    Statistics {
        entities,
        lineage,
        units: vec![ent.unit, mi.unit],
        relations,
        entropy,
        scan,
    }
}

fn walk(p: &Path) -> u64 {
    match std::fs::metadata(p) {
        Ok(m) if m.is_dir() => std::fs::read_dir(p)
            .map(|d| d.flatten().map(|e| walk(&e.path())).sum())
            .unwrap_or(0),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}
