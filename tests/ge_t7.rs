//! GE-T7's stand-in (SMYSL-2.8 §5.4; TX-P13c step 2): a library of a thousand works in five
//! languages, one shard each beside the catalog, about 10⁶ units, and every threshold GE-T7
//! names measured over it — the build, a measure's growth, a scoped query, `check`'s peak on the
//! redb backend, `derive_thread` over 10⁵ units, contention detection over the whole union
//! (which decides OQ-16), and a single-record append to a 10⁵-unit shard.
//!
//! ```text
//! cargo test --release --all-features --test ge_t7 -- --ignored --nocapture
//! SMYSL_GE_T7_PRIMARY=1 cargo test --release --all-features --test ge_t7 -- --ignored --nocapture
//! SMYSL_GE_T7_WORKS=50 SMYSL_GE_T7_BIG=5000 SMYSL_GE_T7_UNITS=50000 \
//!     cargo test --release --all-features --test ge_t7 -- --ignored --nocapture
//! ```
//!
//! **A stand-in, not GE-T7.** GE-T7's corpus is a thousand public-domain works through
//! SMYSL-2.4's whole pipeline; the repository carries ten (Ruth and Jude in five languages,
//! `fixtures/library/bibles/slice`). Each work here is one language's 110 verses, rotated by
//! its number and headed by it, so a thousand works are a thousand distinct texts that the
//! readers, the segmenter and the indexes all really read — about 12 MB of text, where a
//! thousand books would be some hundreds. The units are built, not extracted (OQ-69's
//! proposal: GE-T7's build excludes model extraction), at §5.4's proportions plus every shape
//! a contention detector looks for, so detection has work to do at every size. One work holds
//! 10⁵ units and the rest share what is left: the shard GE-T7's append threshold names.
//!
//! A measurement, not a gate, for the reason `scale_ladder.rs` gives. What it does assert is
//! that the answers are right: the union's contentions are the materialised store's, and are
//! exactly the ones the corpus was built to hold.

#![cfg(all(feature = "cli", feature = "text", feature = "store-redb"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Instant;

use smysl_core::{
    canonical_uid, AgentId, Carry, Commit, Commitment, DetectionKind, Hlc, KernelType, Label,
    LabelBinding, LangTag, Record, RelKind, Relation, Role, SourceKind, SourceRef, Status, Step,
    Thread, ThreadId, ThreadSchema, Tid, Uid, UnitCoreBuilder,
};
use smysl_graph::merge::contention::{detect, DetectionContext};
use smysl_graph::{AnyStore, Store, StoreRead};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;

const LANGS: [(&str, &str); 5] = [
    ("en", "eng-kjv"),
    ("de", "deu1912"),
    ("fr", "fraLSG"),
    ("es", "spaRV1909"),
    ("ru", "russyn"),
];
/// Works per shelf: `collection:<lang>/sNN` is twenty works.
const SHELF: usize = 20;

fn env(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .map(|v| v.parse().expect("a whole number"))
        .unwrap_or(default)
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// One language's verses, Ruth then Jude, without their markers.
fn verses(edition: &str) -> Vec<String> {
    let mut out = Vec::new();
    for book in ["08-RUT", "65-JUD"] {
        let raw = std::fs::read_to_string(repo(&format!(
            "fixtures/library/bibles/slice/{edition}/{book}.usfm"
        )))
        .unwrap();
        for line in raw.lines() {
            if let Some(rest) = line.strip_prefix("\\v ") {
                let text = rest.split_once(' ').map_or("", |(_, t)| t).trim();
                if !text.is_empty() {
                    out.push(text.to_string());
                }
            }
        }
    }
    out
}

/// The twenty commonest words of five letters or more: what a reader of that language finds.
fn words(verses: &[String]) -> Vec<String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for v in verses {
        for w in v.split(|c: char| !c.is_alphabetic()) {
            if w.chars().count() >= 5 {
                *count.entry(w.to_lowercase()).or_default() += 1;
            }
        }
    }
    let mut by: Vec<(usize, String)> = count.into_iter().map(|(w, n)| (n, w)).collect();
    by.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    by.into_iter().take(20).map(|(_, w)| w).collect()
}

struct Work {
    alias: String,
    lang: &'static str,
    /// Its verses as the text was written: line 1 is the heading, line `k + 2` verse `k`.
    lines: Vec<String>,
    /// Each line's first ten words, within 80 bytes: a gist, which a verse is too long to be
    /// (`E022`).
    heads: Vec<String>,
    units: usize,
}

fn works(n: usize, big: usize, total: usize) -> Vec<Work> {
    let texts: Vec<Vec<String>> = LANGS.iter().map(|(_, e)| verses(e)).collect();
    // A multiple of a hundred, so every work has each place in a hundred that `records` uses.
    let rest = total.saturating_sub(big) / (n - 1).max(1) / 100 * 100;
    (0..n)
        .map(|w| {
            let (l, k) = (w % LANGS.len(), w / LANGS.len());
            let v = &texts[l];
            let rot = (k * 7) % v.len();
            let lines: Vec<String> = v[rot..].iter().chain(&v[..rot]).cloned().collect();
            let heads = lines
                .iter()
                .map(|l| {
                    // Thirty tokens at four bytes each, less the suffix a gist gets.
                    let mut head = String::new();
                    for word in l.split_whitespace().take(10) {
                        if head.len() + word.len() + 1 > 80 {
                            break;
                        }
                        if !head.is_empty() {
                            head.push(' ');
                        }
                        head.push_str(word);
                    }
                    head
                })
                .collect();
            Work {
                heads,
                alias: format!("{}/s{:02}/w{:03}", LANGS[l].0, k / SHELF, k),
                lang: LANGS[l].0,
                lines,
                units: if w == 0 { big } else { rest },
            }
        })
        .collect()
}

/// What the corpus was built to hold, per detector: what detection must find exactly.
#[derive(Debug, Default, PartialEq, Eq)]
struct Expected {
    forks: usize,
    rebuttals: usize,
    labels: usize,
    commitments: usize,
}

fn agent(name: &str) -> AgentId {
    AgentId::new(name).unwrap()
}

/// Work `w`'s records: its units, their edges, and every shape a detector looks for, once per
/// thousand units. Some of each reach into the next work, so a shard alone does not hold them.
fn records(w: usize, works: &[Work], tids: &[Tid], expected: &mut Expected) -> Vec<Record> {
    let work = &works[w];
    let next = (w + 1) % works.len();
    let source = |w: usize, j: usize| {
        let line = 2 + j % works[w].lines.len();
        SourceRef::new(SourceKind::Doc, format!("{}#L{line}", tids[w].canonical()))
    };
    let cited = |w: usize, j: usize, gist: String| {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited)
            .source(source(w, j))
            .build()
            .unwrap()
    };
    let mut out = Vec::new();
    let mut uids: Vec<Uid> = Vec::with_capacity(work.units);
    let mut derived: Vec<Uid> = Vec::new();
    for j in 0..work.units {
        let gist = format!("{} [{w}.{j}]", work.heads[j % work.heads.len()]);
        let core = if j % 10 == 9 {
            let g = [uids[j - 1], uids[j - 3], uids[j - 7]];
            UnitCoreBuilder::new(KernelType::Claim, gist, Status::Derived)
                .grounds(g)
                .source(source(w, j))
                .build()
                .unwrap()
        } else {
            cited(w, j, gist)
        };
        let uid = canonical_uid(&core);
        if j % 10 == 9 {
            if let Some(&to) = derived.get(j % derived.len().max(1)) {
                out.push(Record::Relation(Relation::new(RelKind::Backs, uid, to)));
            }
            derived.push(uid);
        }
        uids.push(uid);
        out.push(Record::Unit(core));
    }
    let ext = |k: &str| RelKind::Extension(k.into());
    for (j, &u) in uids.iter().enumerate() {
        // §5.4's text shapes, for the measures: a same-as across languages in every hundred,
        // a warrant in every fifty, a contradiction in every thousand.
        if j % 100 == 5 {
            // The next work's unit at the same place in its hundred: cited, as every unit not
            // ending a ten is, and there, as every work's length is a multiple of a hundred.
            let k = j % works[next].units;
            let line = &works[next].heads[k % works[next].heads.len()];
            let other = canonical_uid(&cited(next, k, format!("{line} [{next}.{k}]")));
            out.push(Record::Relation(Relation::new(
                ext(smysl_graph::proposition::SAME_AS),
                u,
                other,
            )));
        }
        if j % 50 == 1 {
            out.push(Record::Relation(Relation::new(
                RelKind::Warrant,
                uids[j - 1],
                u,
            )));
        }
        if j % 1000 == 2 {
            out.push(Record::Relation(Relation::new(
                ext(smysl_core::schemas::text::CONTRADICTS),
                u,
                uids[j - 2],
            )));
        }
        if j % 200 == 3 {
            // A quotation of the next work: an edge whose target is in another shard.
            out.push(Record::Relation(Relation::new(
                ext("x.text/quotes"),
                u,
                canonical_uid(&cited(
                    next,
                    0,
                    format!("{} [{next}.0]", works[next].heads[0]),
                )),
            )));
        }
        let at = j % 1000;
        if at == 11 && j + 20 < uids.len() {
            // (a) A fork: two revisions of one unit, the second sourced to the next work, so
            // its unit and its `supersedes` live in the next work's shard.
            let a = cited(w, j, format!("revision a of [{w}.{j}]"));
            let b = cited(next, j, format!("revision b of [{w}.{j}]"));
            let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
            out.push(Record::Unit(a));
            out.push(Record::Unit(b));
            out.push(Record::Relation(Relation::new(RelKind::Supersedes, ua, u)));
            out.push(Record::Relation(Relation::new(RelKind::Supersedes, ub, u)));
            expected.forks += 1;
            // Not a fork: a chain of two revisions of the unit after it.
            let target = uids[j + 12];
            let c = cited(w, j, format!("revision c of [{w}.{}]", j + 12));
            let d = cited(w, j, format!("revision d of [{w}.{}]", j + 12));
            let (uc, ud) = (canonical_uid(&c), canonical_uid(&d));
            out.push(Record::Unit(c));
            out.push(Record::Unit(d));
            out.push(Record::Relation(Relation::new(
                RelKind::Supersedes,
                uc,
                target,
            )));
            out.push(Record::Relation(Relation::new(
                RelKind::Supersedes,
                ud,
                target,
            )));
            out.push(Record::Relation(Relation::new(RelKind::Supersedes, ud, uc)));
        }
        if at == 37 {
            // (b) A rebuttal a thread presents beside its target, and one no thread does.
            let r = cited(w, j, format!("a rebuttal of [{w}.{j}]"));
            let ur = canonical_uid(&r);
            out.push(Record::Unit(r));
            out.push(Record::Relation(Relation::new(RelKind::Rebuts, ur, u)));
            out.push(Record::Relation(Relation::new(
                RelKind::Rebuts,
                uids[j - 1],
                uids[j - 2],
            )));
            let owner = agent("human:reader");
            out.push(Record::Thread(
                Thread::new(
                    ThreadId::new(format!("t/w{w}-{j}")).unwrap(),
                    ThreadSchema::Brief,
                    owner.clone(),
                    "both sides",
                    Hlc::zero(owner),
                )
                .with_steps([Step::new(Role::BottomLine, u), Step::new(Role::Risk, ur)]),
            ));
            expected.rebuttals += 1;
        }
        if at == 51 && j + 1 < uids.len() {
            // (c) One label, two units.
            let label = Label::new(format!("c/w{w}-{j}")).unwrap();
            out.push(Record::LabelBinding(LabelBinding::new(label.clone(), u)));
            out.push(Record::LabelBinding(LabelBinding::new(label, uids[j + 1])));
            expected.labels += 1;
        }
        if at == 67 && j + 1 < uids.len() {
            // (d) Two agents disagree about how settled a unit is; one agent revising its own
            // word on the next is not a disagreement.
            let c = |unit, level, who: &str, ms| {
                Record::Commit(Commit::new(
                    unit,
                    level,
                    agent(who),
                    Hlc::new(ms, 0, agent(who)),
                ))
            };
            out.push(c(u, Commitment::Drafted, "human:one", 1));
            out.push(c(u, Commitment::Canonical, "human:two", 2));
            out.push(c(uids[j + 1], Commitment::Floated, "human:one", 1));
            out.push(c(uids[j + 1], Commitment::Committed, "human:one", 2));
            expected.commitments += 1;
        }
    }
    out
}

fn counts(found: &[smysl_core::Contention]) -> Expected {
    let mut e = Expected::default();
    for c in found {
        match c.detected.kind {
            DetectionKind::SupersessionFork => e.forks += 1,
            DetectionKind::LiveRebuttal => e.rebuttals += 1,
            DetectionKind::LabelCollision => e.labels += 1,
            DetectionKind::CommitmentFork => e.commitments += 1,
            _ => {}
        }
    }
    e
}

/// The shards of `root` named by `keep`, each read as the CLI's `--shard` reads it.
fn union(root: &Path, keep: impl Fn(&str) -> bool, redb: bool) -> AnyStore {
    let all = smysl_text::shards::list(root).unwrap();
    if redb {
        // The library's one unit store, opened for the shards kept (TX-P13c step 3).
        let only: Vec<String> = all
            .iter()
            .filter(|(n, _)| keep(n))
            .map(|(n, _)| n.clone())
            .collect();
        let subset = only.len() < all.len();
        return AnyStore::open_redb_shards(
            &root.join(smysl_text::shards::UNIT_STORE),
            &all,
            subset.then_some(only.as_slice()),
            None,
            smysl_graph::redb::RedbOptions::default(),
        )
        .unwrap()
        .0;
    }
    let mut stores = Vec::new();
    for (name, _) in all {
        if !keep(&name) {
            continue;
        }
        // The log, or a primary shard's records from the unit store.
        let bytes = smysl_text::shards::read_bytes(root, &name).unwrap();
        if bytes.is_empty() {
            continue;
        }
        stores.push((
            name,
            AnyStore::from(Store::from_records(
                smysl_core::from_cbor_seq(&bytes).unwrap().0,
            )),
        ));
    }
    AnyStore::shards(stores, None)
}

/// Peak resident set and wall time of `smysl <args>`, in a child of its own.
fn peak(args: &[&str]) -> (Option<u64>, f64, String) {
    let mac = cfg!(target_os = "macos");
    let t = Instant::now();
    let out = std::process::Command::new("/usr/bin/time")
        .arg(if mac { "-l" } else { "-v" })
        .arg(env!("CARGO_BIN_EXE_smysl"))
        .args(args)
        .output()
        .unwrap();
    let took = ms(t);
    let err = String::from_utf8_lossy(&out.stderr).to_string();
    let rss = err.lines().find_map(|l| {
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
    });
    let tail = String::from_utf8_lossy(&out.stdout)
        .lines()
        .chain(err.lines())
        .filter(|l| l.contains("SMY-E") || l.contains("error"))
        .take(3)
        .collect::<Vec<_>>()
        .join(" | ");
    (rss, took, tail)
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "met"
    } else {
        "NOT MET"
    }
}

fn p(sorted: &[f64], q: f64) -> f64 {
    sorted[((sorted.len() as f64 * q).ceil() as usize).clamp(1, sorted.len()) - 1]
}

#[test]
#[ignore = "a measurement over a million units, not a gate"]
fn ge_t7_stand_in() {
    let n = env("SMYSL_GE_T7_WORKS", 1000);
    let big = env("SMYSL_GE_T7_BIG", 100_000);
    let total = env("SMYSL_GE_T7_UNITS", 1_000_000);
    let keep = std::env::var("SMYSL_GE_T7_DIR").ok();
    let dir = keep.clone().map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("smysl-ge-t7-{}", std::process::id()))
    });
    let root = dir.join("lib");
    let works = works(n, big, total);
    // `SMYSL_GE_T7_ONLY=check,append`: those stages alone (the build always, if there is none).
    let only = std::env::var("SMYSL_GE_T7_ONLY").ok();
    let want = |stage: &str| {
        only.as_deref()
            .is_none_or(|v| v.split(',').any(|s| s == stage))
    };
    println!("GE-T7 stand-in: {n} works, the first of {big} units, {total} in all");

    // --- the build: readers and segmentation, the records, the indexes ---------------------
    let mut expected = Expected::default();
    let built = Instant::now();
    if root.exists() {
        println!("  reusing {}", root.display());
    } else {
        std::fs::create_dir_all(&dir).unwrap();
        let mut lib = Library::create(&root).unwrap();
        let t = Instant::now();
        let mut tids = Vec::with_capacity(n);
        for (w, work) in works.iter().enumerate() {
            let text = format!("Work {w}.\n{}\n", work.lines.join("\n"));
            let spec = AddSpec::new("txt/1", &work.alias, "public-domain")
                .with_lang(LangTag::new(work.lang).unwrap())
                .with_carry(Carry::Text);
            let added = lib
                .add(&Input::new(text.as_bytes()), &spec, &Caps::DEFAULT)
                .unwrap();
            tids.push(added.parts[0]);
        }
        println!(
            "  {:<44} {:>10.1} s",
            "build: read and segment",
            t.elapsed().as_secs_f64()
        );
        let t = Instant::now();
        let mut records_n = 0;
        for w in 0..n {
            let r = records(w, &works, &tids, &mut expected);
            records_n += r.len();
            lib.append_records(&r).unwrap();
        }
        println!(
            "  {:<44} {:>10.1} s  ({records_n} records, {n} appends)",
            "build: append the records",
            t.elapsed().as_secs_f64()
        );
        let t = Instant::now();
        lib.write_index().unwrap();
        println!(
            "  {:<44} {:>10.1} s",
            "build: the indexes",
            t.elapsed().as_secs_f64()
        );
        let secs = built.elapsed().as_secs_f64();
        println!(
            "  {:<44} {:>10.1} s  < 3600 s: {}",
            "library build",
            secs,
            verdict(secs < 3600.0)
        );
    }
    if expected == Expected::default() {
        // A kept library: what it holds, counted from the corpus that wrote it.
        let tids = vec![smysl_text::ids::tid(&smysl_text::norm::Normalised::of("")); n];
        for w in 0..n {
            records(w, &works, &tids, &mut expected);
        }
    }

    // `SMYSL_GE_T7_PRIMARY=1` (TX-P13d's exit): every shard made primary before anything is
    // measured, so the run is over the unit store holding the records rather than the logs.
    let primary = std::env::var_os("SMYSL_GE_T7_PRIMARY").is_some();
    if primary {
        let mut lib = Library::open(&root).unwrap();
        let names = lib.shards();
        let t = Instant::now();
        let digests = lib.migrate_primary_all(&names).unwrap();
        println!(
            "  {:<44} {:>10.1} s  ({} shards)",
            "migrate: every shard made primary",
            t.elapsed().as_secs_f64(),
            digests.len()
        );
        assert_eq!(lib.primary_shards().len(), names.len());
    }

    let t = Instant::now();
    let lib = Library::open(&root).unwrap();
    let store = lib.store();
    println!(
        "  {:<44} {:>10.1} ms  ({} units, {} shards)",
        "Library::open",
        ms(t),
        store.unit_count(),
        lib.shards().len()
    );

    // --- contention detection (OQ-16) -------------------------------------------------------
    if want("detect") {
        let ctx = DetectionContext::default();
        let t = Instant::now();
        let whole = detect(store, &ctx);
        let on_store = ms(t);
        assert_eq!(counts(&whole), expected, "the store's contentions");
        let t = Instant::now();
        let mem = union(&root, |_| true, false);
        let mem_open = ms(t);
        let u = mem.as_union().unwrap();
        let t = Instant::now();
        let first = u.detected().to_vec();
        let on_union = ms(t);
        let t = Instant::now();
        let again = u.detected().len();
        let kept = ms(t);
        assert_eq!(first, whole, "a union detects what its store does");
        assert_eq!(again, whole.len());
        let t = Instant::now();
        let redb = union(&root, |_| true, true);
        let redb_open = ms(t);
        let t = Instant::now();
        let on_redb = detect(&redb, &ctx);
        let on_redb_ms = ms(t);
        assert_eq!(on_redb, whole, "the redb union detects what the store does");
        println!(
            "  contentions: {} forks, {} rebuttals, {} labels, {} commitments",
            expected.forks, expected.rebuttals, expected.labels, expected.commitments
        );
        println!(
            "  {:<44} {:>10.1} ms",
            "detect: the materialised store", on_store
        );
        println!(
            "  {:<44} {:>10.1} ms  (union built in {:.1} s)",
            "detect: the union, memory shards",
            on_union,
            mem_open / 1000.0
        );
        println!(
            "  {:<44} {:>10.3} ms",
            "detect: the union again (kept)", kept
        );
        println!(
            "  {:<44} {:>10.1} ms  (opened in {:.1} s)",
            "detect: the library's unit store (redb)",
            on_redb_ms,
            redb_open / 1000.0
        );
        let worst = on_store.max(on_union).max(on_redb_ms);
        println!(
            "  {:<44} {:>10.1} ms  < 1000 ms: {}",
            "contention detection, worst",
            worst,
            verdict(worst < 1000.0)
        );
        drop((mem, redb));
    }

    // --- a measure over 10⁵, 3·10⁵ and 10⁶ units --------------------------------------------
    if want("measure") {
        let mut order: Vec<Uid> = Vec::new();
        for name in lib.shards() {
            order.extend(lib.uid_shards().uids_of(&name));
        }
        let mut seen = BTreeSet::new();
        order.retain(|u| store.contains_uid(u) && seen.insert(*u));
        let sizes: Vec<usize> = [100_000, 300_000, 1_000_000]
            .into_iter()
            .map(|s| s.min(order.len()))
            .collect();
        let mut points = Vec::new();
        for &size in &sizes {
            let scope =
                smysl_measure::Resolved::of_uids("ge-t7", order[..size].iter().copied().collect());
            let input = smysl_measure::Input::new(store, &scope).with_library(&lib);
            let mut recipe = smysl_measure::Recipe::new("", 0, "ge-t7");
            recipe.policy = Some(smysl_measure::ClassPolicy::Strict);
            recipe.profile = Some("default".into());
            // Five languages in one scope: their words counted in one vocabulary, as `E489` asks.
            recipe.analyzer = Some("smysl/an/1:surface".into());
            let t = Instant::now();
            let results = smysl_measure::run(&["A1", "A2", "A4", "A5", "A7"], &recipe, &input);
            let secs = t.elapsed().as_secs_f64();
            for r in &results {
                assert!(
                    matches!(r.outcome, smysl_measure::Outcome::Value(_)),
                    "{} at {size}: {:?}",
                    r.recipe.id,
                    r.outcome
                );
            }
            println!(
                "  {:<44} {:>10.2} s  ({size} units)",
                "measure: A1 A2 A4 A5 A7", secs
            );
            points.push((size as f64, secs));
        }
        // Linear through the origin, by least squares; every point within 2× of the line.
        let slope = points.iter().map(|(x, y)| x * y).sum::<f64>()
            / points.iter().map(|(x, _)| x * x).sum::<f64>();
        let off = points
            .iter()
            .map(|(x, y)| (y / (slope * x)).max(slope * x / y))
            .fold(0.0f64, f64::max);
        println!(
            "  {:<44} {:>10.2} ×  ≤ 2: {}",
            "measure: furthest from linear",
            off,
            verdict(off <= 2.0)
        );
    }

    // --- a scoped query --------------------------------------------------------------------
    if want("query") {
        let scope = smysl_text::scope::LibraryScope::new("library", &lib);
        let qenv = smysl_query::Env::new(store, &scope)
            .with_times(&scope)
            .with_entities(&scope);
        let opts = smysl_query::PlanOptions::default();
        let vocab: Vec<Vec<String>> = LANGS.iter().map(|(_, e)| words(&verses(e))).collect();
        let mut all_times = Vec::new();
        // A shelf without the large work, where there is more than one shelf.
        let shelf = format!(
            "collection:\"en/s{:02}\"",
            usize::from(n / LANGS.len() > SHELF)
        );
        for (item, lang) in [
            (shelf.as_str(), Some(0)),
            ("collection:de", Some(1)),
            ("library", None),
        ] {
            let mut times = Vec::new();
            for k in 0..20 {
                let word = match lang {
                    Some(l) => vocab[l][k].clone(),
                    None => vocab[k % LANGS.len()][k / LANGS.len()].clone(),
                };
                let q = format!("from {item}\nfind \"{word}\"\nlimit 10");
                let t = Instant::now();
                let parsed = smysl_query::parse(&q).unwrap();
                let plan = smysl_query::plan(&parsed.query, &qenv, &opts)
                    .unwrap_or_else(|d| panic!("{q}: {}", d[0].message));
                let run = smysl_query::run(&plan, &qenv, &opts)
                    .unwrap_or_else(|d| panic!("{q}: {}", d[0].message));
                std::hint::black_box(run);
                times.push(ms(t));
                // Twenty samples, unless three already took a minute: past the threshold that far,
                // more samples would not change the verdict and would cost the run an hour.
                if times.len() >= 3 && times.iter().sum::<f64>() > 60_000.0 {
                    break;
                }
            }
            times.sort_by(|a, b| a.partial_cmp(b).unwrap());
            println!(
                "  {:<44} {:>10.1} ms p50, {:.1} ms p95 of {}",
                format!("query: from {item} | find | limit 10"),
                p(&times, 0.5),
                p(&times, 0.95),
                times.len()
            );
            all_times.extend(times);
        }
        all_times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = p(&all_times, 0.95);
        println!(
            "  {:<44} {:>10.1} ms  < 1000 ms: {}",
            "scoped query p95, all scopes",
            p95,
            verdict(p95 < 1000.0)
        );
        drop(qenv);
        drop(scope);
    }

    // --- derive_thread over 10⁵ units --------------------------------------------------------
    if want("thread") {
        let opts = smysl_thread::DeriveOptions::new(
            ThreadId::new("t/ge-t7").unwrap(),
            AgentId::new("tool:ge-t7").unwrap(),
        );
        let first_alias = works[0].alias.clone();
        let big_shard = union(&root, |name| name == first_alias, false);
        let t = Instant::now();
        std::hint::black_box(smysl_thread::derive_thread(
            &big_shard,
            ThreadSchema::Analysis,
            &opts,
        ));
        let one = ms(t);
        println!(
            "  {:<44} {:>10.1} ms  ({} units)",
            "derive_thread: the 10⁵-unit shard",
            one,
            big_shard.unit_count()
        );
        drop(big_shard);
        let small: BTreeSet<String> = works[1..]
            .iter()
            .scan(0usize, |sum, w| {
                *sum += w.units;
                (*sum <= big).then(|| w.alias.clone())
            })
            .collect();
        let many = union(&root, |name| small.contains(name), false);
        let t = Instant::now();
        std::hint::black_box(smysl_thread::derive_thread(
            &many,
            ThreadSchema::Analysis,
            &opts,
        ));
        let across = ms(t);
        println!(
            "  {:<44} {:>10.1} ms  ({} units in {} shards)",
            "derive_thread: a union of small shards",
            across,
            many.unit_count(),
            small.len()
        );
        drop(many);
        println!(
            "  {:<44} {:>10.1} ms  < 2000 ms: {}",
            "derive_thread over 10⁵ units, worst",
            one.max(across),
            verdict(one.max(across) < 2000.0)
        );
        drop(lib);
    }

    // --- check: the peak on each backend ----------------------------------------------------
    if want("check") {
        let at = root.to_str().unwrap();
        let (mem_rss, mem_ms, mem_err) = peak(&["check", "-s", at]);
        let (redb_rss, redb_ms, redb_err) = peak(&["--backend", "redb", "check", "-s", at]);
        let mb = |b: Option<u64>| b.map_or("?".into(), |b| format!("{:.0} MB", b as f64 / 1e6));
        println!(
            "  {:<44} {:>10}  in {:.1} s {mem_err}",
            "check: peak RSS, memory",
            mb(mem_rss),
            mem_ms / 1000.0
        );
        println!(
            "  {:<44} {:>10}  in {:.1} s {redb_err}",
            "check: peak RSS, redb",
            mb(redb_rss),
            redb_ms / 1000.0
        );
        println!(
            "  {:<44} {:>10}  ≤ 1 GB: {}",
            "check over the union on redb",
            mb(redb_rss),
            verdict(redb_rss.is_some_and(|b| b <= 1_000_000_000))
        );
    }

    // --- a single-record append to the 10⁵-unit shard ---------------------------------------
    if want("append") {
        let mut lib = Library::open(&root).unwrap();
        let tid = lib
            .store()
            .manifests()
            .find(|(_, m)| m.alias == works[0].alias)
            .map(|(_, m)| m.parts[0].tid)
            .unwrap();
        let fresh = |tag: &str| {
            let n = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            UnitCoreBuilder::new(KernelType::Claim, format!("{tag} at {n}"), Status::Cited)
                .source(SourceRef::new(
                    SourceKind::Doc,
                    format!("{}#L2", tid.canonical()),
                ))
                .build()
                .unwrap()
        };
        let mut lib_ms = Vec::new();
        for _ in 0..5 {
            let r = [Record::Unit(fresh("appended through the library"))];
            let t = Instant::now();
            lib.append_records(&r).unwrap();
            lib_ms.push(ms(t));
        }
        lib_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        drop(lib);
        // A one-log writer on the shard's log; not for a primary shard, whose library append
        // above is already the store's.
        let mut redb_ms = vec![f64::INFINITY; 5];
        if !primary {
            let log = root.join(smysl_text::shards::log_path(&works[0].alias));
            let (mut shard, _) = smysl_graph::redb::RedbShard::open_rw(
                &log,
                smysl_graph::redb::RedbOptions::default(),
            )
            .unwrap();
            redb_ms.clear();
            for _ in 0..5 {
                let r = [Record::Unit(fresh("appended through redb"))];
                let t = Instant::now();
                assert_eq!(shard.append(&r).unwrap().added, 1);
                redb_ms.push(ms(t));
            }
            redb_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
            drop(shard);
        }
        println!(
            "  {:<44} {:>10.2} ms  (median of 5)",
            "append: through the library", lib_ms[2]
        );
        println!(
            "  {:<44} {:>10.2} ms  (median of 5)",
            "append: through the shard's redb store", redb_ms[2]
        );
        let best = lib_ms[2].min(redb_ms[2]);
        println!(
            "  {:<44} {:>10.2} ms  < 5 ms: {}",
            "single-record append to the 10⁵-unit shard",
            best,
            verdict(best < 5.0)
        );
    }

    if keep.is_none() {
        let _ = std::fs::remove_dir_all(&dir);
    }
}
