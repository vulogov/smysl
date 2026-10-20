//! GE-T6's A8 arm (SMYSL-2.6 §5.4, TX-P9 N7): does A8 tell the Bible's genres apart better than
//! extraction tells two runs of one book apart?
//!
//! A8 is not pure in the sense T2 and T3 are: its input is the units a model extracted, so the
//! "run-to-run variance" the kill criterion reads is **the variance across extraction runs**,
//! GE-T2's runs, where the pure arm reads window sampling. Everything else is the pure arm's,
//! from `support/genre.rs`: the labels, Kruskal–Wallis H with ε² and a permutation p, the
//! between-genre mean square against the within-genre one plus the noise, 3 of 5 languages to
//! drop a measure from defaults.
//!
//! # The protocol, as built
//!
//! - **Material**: `SMYSL_GE_T6_A8=<dir>` holds `<edition>/<run>`, one store per extraction run
//!   of one edition — each GE-T1 book added to a library as `bible/<nn>-<code>`, as the pure arm
//!   adds them, and ingested with `ingest --text`. Runs are the files in `<edition>/`, in name
//!   order; §5.4 asks for GE-T2's five.
//! - **A book** is the units of a run drawn from that book's parts: the text a unit was read
//!   under when its source names a manifest, else the manifest listing its part. A unit drawn
//!   from no part is in no book.
//! - **A book's value** is A8's `area` over its units under the default recipe, one per run, and
//!   the median of its runs; the **noise** is each book's sample variance over its runs,
//!   averaged over books. A book A8 refuses in any run, or which a run did not extract at all,
//!   is excluded and listed with the reason, as the pure arm lists a book shorter than a window.
//! - **No length control.** A8's budgets are fractions of the book's own `full_tokens`, so its
//!   curve is in the book's units already; equal-length windows would cut the grounds and
//!   rebuttals a pack has to carry together, which is the thing A8 measures.
//!
//! The scripted arm runs everywhere: books found by the text a unit was read from, A8 per book
//! as `run` computes it, and the kill criterion reading extraction variance on planted values.
//! The measurement is `--ignored`, and **owed** until the runs exist: 5 editions × 5 runs of
//! 66 books is about 780,000 verses through a model, `SMYSL_INGEST_LIVE=1` as GE-T2's.
//!
//! ```text
//! SMYSL_GE_T6_A8=<dir> cargo test --release -p smysl-measure --test ge_t6_a8 -- --ignored --nocapture
//! ```

#[path = "support/genre.rs"]
mod genre;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Tid, Uid};
use smysl_core::types::{Carry, SourceKind, SourceRef};
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_measure::num::from_q20;
use smysl_measure::{run, Input, Num, Outcome, Recipe, Resolved};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

use genre::{analyse, genre_names, genres, mean_variance, median, repo, verdict};

const SCOPE: &str = "ge-t6/a8";

/// The book a text's alias names: `bible/08-rut` is `RUT`.
fn code_of(alias: &str) -> String {
    let tail = alias.rsplit('/').next().unwrap_or(alias);
    tail.rsplit_once('-')
        .map_or(tail, |(_, c)| c)
        .to_uppercase()
}

/// A run's units by book.
fn books(store: &Store) -> BTreeMap<String, BTreeSet<Uid>> {
    let mut by_tid: BTreeMap<Tid, String> = BTreeMap::new();
    for (mid, m) in store.manifests() {
        if store.is_superseded(mid) {
            continue;
        }
        for p in &m.parts {
            // A part two texts share is read under the first alias, as A3 reads it.
            by_tid
                .entry(p.tid)
                .and_modify(|a| {
                    if m.alias < *a {
                        a.clone_from(&m.alias);
                    }
                })
                .or_insert_with(|| m.alias.clone());
        }
    }
    let mut out: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for (uid, unit) in store.units() {
        let Some(source) = unit.core.source.as_ref() else {
            continue;
        };
        let Some(tid) = Store::part_of_reference(&source.reference) else {
            continue;
        };
        let alias = match source.manifest().and_then(|m| store.manifest(m)) {
            Some(m) if m.parts.iter().any(|p| p.tid == tid) => Some(m.alias.clone()),
            _ => by_tid.get(&tid).cloned(),
        };
        if let Some(a) = alias {
            out.entry(code_of(&a)).or_default().insert(*uid);
        }
    }
    out
}

/// A8's area over one book's units, or the code it refused with.
fn area(store: &Store, uids: &BTreeSet<Uid>) -> Result<f64, String> {
    let recipe = Recipe::new("", 0, SCOPE);
    let scope = Resolved::of_uids(SCOPE, uids.clone());
    let input = Input::new(store, &scope);
    match run(&["A8"], &recipe, &input).remove(0).outcome {
        Outcome::Value(v) => match v.values.get("area") {
            Some(Num::Fixed(x)) => Ok(from_q20(*x)),
            other => Err(format!("area {other:?}")),
        },
        Outcome::NotRun { code, .. } => {
            Err(code.map_or_else(|| "not run".to_string(), |c| c.to_string()))
        }
        other => Err(format!("{other:?}")),
    }
}

/// One edition's books, kept and excluded: `(code, genre, area per run)` and the reasons.
type Kept = Vec<(String, usize, Vec<f64>)>;

fn edition(
    runs: &[Store],
    labels: &BTreeMap<String, String>,
    names: &[String],
) -> (Kept, Vec<String>) {
    let per_run: Vec<BTreeMap<String, BTreeSet<Uid>>> = runs.iter().map(books).collect();
    let codes: BTreeSet<&String> = per_run.iter().flat_map(|b| b.keys()).collect();
    let (mut kept, mut excluded) = (Vec::new(), Vec::new());
    'book: for code in codes {
        let g = labels.get(code).map(String::as_str).unwrap_or("-");
        if g == "-" {
            excluded.push(format!("{code}(unlabelled)"));
            continue;
        }
        let mut areas = Vec::with_capacity(runs.len());
        for (r, (store, books)) in runs.iter().zip(&per_run).enumerate() {
            let Some(uids) = books.get(code) else {
                excluded.push(format!("{code}(none in run {r})"));
                continue 'book;
            };
            match area(store, uids) {
                Ok(a) => areas.push(a),
                Err(why) => {
                    excluded.push(format!("{code}({why} in run {r})"));
                    continue 'book;
                }
            }
        }
        let gi = names.iter().position(|x| x == g).unwrap();
        kept.push((code.clone(), gi, areas));
    }
    (kept, excluded)
}

// ---- the scripted arm ----------------------------------------------------------------------

struct Built {
    root: PathBuf,
    library: Library,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Ruth and Jude from the committed KJV slice, as `bible/08-rut` and `bible/65-jud`.
fn library() -> Built {
    let root = std::env::temp_dir().join(format!("smysl-ge-t6-a8-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for file in ["08-RUT", "65-JUD"] {
        let raw = std::fs::read(repo(&format!(
            "fixtures/library/bibles/slice/eng-kjv/{file}.usfm"
        )))
        .unwrap();
        let alias = format!("bible/{}", file.to_lowercase());
        let spec = AddSpec::new("usfm/1", &alias, "public-domain")
            .with_lang(LangTag::new("en").unwrap())
            .with_carry(Carry::Text);
        library
            .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
    }
    Built { root, library }
}

/// One planted "run": `per_part` claims from every part of every text, a run's statuses its
/// own, a ground for every second claim, and one unit drawn from no part.
fn planted_run(
    lib: &Library,
    run: u64,
    per_part: usize,
) -> (Store, BTreeMap<String, BTreeSet<Uid>>) {
    // `derived` and `inferred` need grounds to be authored at all, so the ladder skips them.
    const LADDER: [Status; 3] = [Status::Speculative, Status::Cited, Status::Measured];
    let mut records = Vec::new();
    let mut by: BTreeMap<String, BTreeSet<Uid>> = BTreeMap::new();
    for (_, m) in lib.store().manifests() {
        let mut previous: Option<Uid> = None;
        for (k, p) in m.parts.iter().enumerate() {
            for i in 0..per_part {
                let status = LADDER[(k + i + run as usize) % LADDER.len()];
                let mut b = UnitCoreBuilder::new(
                    KernelType::Claim,
                    format!("{} part {k} claim {i} run {run}", m.alias),
                    status,
                )
                .source(SourceRef::new(SourceKind::Doc, p.tid.canonical()));
                if let (Some(g), true) = (previous, i % 2 == 1) {
                    b = b.grounds(vec![g]);
                }
                let core = b.build().unwrap();
                let uid = canonical_uid(&core);
                previous = Some(uid);
                by.entry(code_of(&m.alias)).or_default().insert(uid);
                records.push(Record::Unit(core));
            }
        }
        records.push(Record::Manifest(m.clone()));
    }
    let loose = UnitCoreBuilder::new(KernelType::Claim, "from no part", Status::Speculative)
        .build()
        .unwrap();
    records.push(Record::Unit(loose));
    (Store::from_records(records), by)
}

/// A unit is in the book of the text it was drawn from, a unit drawn from no part is in none,
/// and a book's value is A8 over exactly its units — what `measure` computes over that scope.
#[test]
fn a_book_is_the_units_drawn_from_its_text() {
    let built = library();
    let labels = genres();
    let names = genre_names(&labels);
    let runs: Vec<(Store, BTreeMap<String, BTreeSet<Uid>>)> =
        (0..3).map(|r| planted_run(&built.library, r, 2)).collect();
    for (store, planted) in &runs {
        let found = books(store);
        assert_eq!(&found, planted);
        assert_eq!(
            found.keys().map(String::as_str).collect::<Vec<_>>(),
            ["JUD", "RUT"]
        );
        let all: usize = found.values().map(BTreeSet::len).sum();
        assert_eq!(
            all + 1,
            store.units().count(),
            "the loose unit is in no book"
        );
    }

    let stores: Vec<Store> = runs.into_iter().map(|(s, _)| s).collect();
    let (kept, excluded) = edition(&stores, &labels, &names);
    assert!(excluded.is_empty(), "{excluded:?}");
    assert_eq!(kept.len(), 2);
    for (code, gi, areas) in &kept {
        assert_eq!(areas.len(), 3);
        let want = labels[code].as_str();
        assert_eq!(names[*gi], want, "{code}");
        for (r, a) in areas.iter().enumerate() {
            let direct = area(&stores[r], &books(&stores[r])[code]).unwrap();
            assert_eq!(a.to_bits(), direct.to_bits(), "{code} run {r}");
            // D is D_sal + ½·D_stat, each at most 1, so the area is at most 1½.
            assert!((0.0..=1.5).contains(a), "{code} run {r}: {a}");
        }
    }
    assert_eq!(
        kept.iter().map(|k| names[k.1].as_str()).collect::<Vec<_>>(),
        ["epistle", "narrative"]
    );
    // The runs differ in their statuses and so in their losses: the noise is extraction's.
    let by: Vec<&[f64]> = kept.iter().map(|k| k.2.as_slice()).collect();
    assert!(mean_variance(&by) > 0.0);

    // A run that did not extract Jude excludes Jude, and says which run.
    let mut short = stores;
    let (run, mut by) = planted_run(&built.library, 9, 2);
    let jud = by.remove("JUD").unwrap();
    let ruth_only = Store::from_records(
        run.iter()
            .filter(|r| !matches!(r, Record::Unit(u) if jud.contains(&canonical_uid(u))))
            .cloned()
            .collect(),
    );
    short.push(ruth_only);
    let (kept, excluded) = edition(&short, &labels, &names);
    assert_eq!(kept.len(), 1);
    assert_eq!(excluded, ["JUD(none in run 3)"]);
}

/// Planted values: twenty books in five genres a step apart, five runs each. With runs that
/// barely differ the genres stand out and A8 is kept; with runs that differ by more than the
/// genres do the same medians count against it — the criterion reads extraction's variance,
/// not only the books'.
#[test]
fn the_kill_reads_extraction_variance() {
    let labels = genres();
    let names = genre_names(&labels);
    let k = names.len();
    let planted = |step: f64, jitter: f64| -> (Vec<f64>, Vec<usize>, f64) {
        let mut runs: Vec<Vec<f64>> = Vec::new();
        let mut g = Vec::new();
        for gi in 0..k {
            for b in 0..4 {
                let base = 0.2 + step * gi as f64 + 0.001 * b as f64;
                runs.push((0..5).map(|r| base + jitter * (r as f64 - 2.0)).collect());
                g.push(gi);
            }
        }
        let x: Vec<f64> = runs.iter().map(|v| median(&mut v.clone())).collect();
        let by: Vec<&[f64]> = runs.iter().map(Vec::as_slice).collect();
        (x, g, mean_variance(&by))
    };

    let (x, g, noise) = planted(0.05, 0.001);
    let a = analyse(&x, &g, k, noise);
    assert_eq!((a.n, a.genres), (20, 5));
    assert!(!a.against, "{a:?}");
    assert!(a.p < 0.001, "{a:?}");
    // The run variance of a book is jitter² · Σ(r − 2)² / 4 = 2.5 · jitter².
    assert!((noise - 2.5e-6).abs() < 1e-15, "{noise}");

    // The same books' medians, and runs ten times further apart than the genres.
    let (x2, g2, noise) = planted(0.05, 0.5);
    assert_eq!(x, x2);
    let b = analyse(&x2, &g2, k, noise);
    assert_eq!((b.h, b.p), (a.h, a.p), "the ranks are the medians'");
    assert!(b.against, "{b:?}");
    assert_eq!(
        verdict("A8", 3, 5),
        "all\tA8\tverdict\tdropped from defaults (3 of 5 languages against)\n"
    );

    println!(
        "GE-T6's A8 arm is OWED: 5 editions x 5 extraction runs of 66 books have not been made. \
         SMYSL_GE_T6_A8=<dir> of <edition>/<run> stores reports them (--ignored)."
    );
}

// ---- the measurement -----------------------------------------------------------------------

#[test]
#[ignore = "needs SMYSL_GE_T6_A8=<dir> of <edition>/<run> stores; run in release"]
fn ge_t6_a8_arm() {
    let Some(dir) = std::env::var_os("SMYSL_GE_T6_A8").map(PathBuf::from) else {
        eprintln!("  no runs: set SMYSL_GE_T6_A8=<dir> of <edition>/<run> extracted stores");
        return;
    };
    let labels = genres();
    let names = genre_names(&labels);
    let list = |d: &Path, dirs: bool| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.') && d.join(n).is_dir() == dirs)
            .collect();
        v.sort();
        v
    };
    let editions = list(&dir, true);

    let mut out = String::from(
        "# GE-T6's A8 arm (SMYSL-2.6 §5.4): A8's area across the Bible's genres, per edition.\n\
         # Written by crates/smysl-measure/tests/ge_t6_a8.rs under SMYSL_BLESS=1 with\n\
         # SMYSL_GE_T6_A8 naming the extraction runs. The noise is the variance across runs.\n\
         # edition\tmeasure\tkey\tvalue\n",
    );
    let mut against = 0;
    for ed in &editions {
        let run_names = list(&dir.join(ed), false);
        assert!(
            run_names.len() >= 2,
            "{ed}: {} run(s); the noise is a variance across runs and needs two",
            run_names.len()
        );
        let runs: Vec<Store> = run_names
            .iter()
            .map(|r| Store::open(dir.join(ed).join(r)).unwrap_or_else(|e| panic!("{ed}/{r}: {e}")))
            .collect();
        let (kept, excluded) = edition(&runs, &labels, &names);
        let x: Vec<f64> = kept.iter().map(|k| median(&mut k.2.clone())).collect();
        let g: Vec<usize> = kept.iter().map(|k| k.1).collect();
        let by: Vec<&[f64]> = kept.iter().map(|k| k.2.as_slice()).collect();
        let a = analyse(&x, &g, names.len(), mean_variance(&by));
        if a.against {
            against += 1;
        }
        out.push_str(&format!("{ed}\tA8\truns\t{}\n", runs.len()));
        out.push_str(&a.rows(ed, "A8", &excluded, "var:run", &names));
        eprintln!(
            "  {ed} A8: {} books over {} runs, H {:.2}, p {:.4}, against {}",
            a.n,
            runs.len(),
            a.h,
            a.p,
            a.against
        );
    }
    out.push_str(&verdict("A8", against, editions.len()));
    print!("{out}");
    let path = repo("fixtures/measure/ge-t6-a8.tsv");
    if std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &out).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, out, "{} changed", path.display());
}
