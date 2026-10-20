//! GE-T6's pure arm (SMYSL-2.6 §5.4, TX-P8 M7): do T2 and T3 tell the Bible's genres apart?
//!
//! The corpus is the five Bibles GE-T1 names, 66 books each, which this repository does not hold;
//! `SMYSL_GE_T6=<dir>` names a directory of `<edition>/*.usfm`, as `SMYSL_GE_T1` does for GE-T1,
//! and without it the test says so and passes. What comes inside is the **measurement**:
//! `fixtures/measure/ge-t6.tsv`, rewritten under `SMYSL_BLESS=1` and compared otherwise.
//!
//! ```text
//! SMYSL_GE_T6=<dir> cargo test --release -p smysl-measure --test ge_t6 -- --ignored --nocapture
//! ```
//!
//! # The protocol, as built
//!
//! - **Labels** are `fixtures/measure/genres.tsv`, §5.4's list by USFM code; the four books it
//!   does not place are excluded and listed.
//! - **Equal-length samples**: each book contributes m = 20 windows of 20,000 characters (T2) or
//!   50,000 (T3, whose symbols are characters by default), at offsets drawn by the crate's
//!   generator keyed by edition, book and measure, with the seed. A book shorter than one window
//!   is excluded from that measure and listed. A book's value is the median of its 20.
//! - **Statistic**: Kruskal–Wallis H across genres on the books' values, per language, ties
//!   corrected; ε² = H/(n − 1); p from 10,000 seeded shuffles of the labels, `(1 + #{H* ≥ H}) /
//!   10,001`. Genre medians with 95% intervals from 1,000 bootstrap resamples of the genre's
//!   books, nearest-rank.
//! - **Kill**: the between-genre mean square, `Σ n_g (m_g − m)² / (k − 1)`, against the
//!   within-genre mean square, `ΣΣ (x − m_g)² / (n − k)`, plus the window-sampling variance —
//!   each book's value recomputed under seeds 0 to 9, its sample variance, averaged over books.
//!   A language where the first is not larger counts against the measure, and a measure counted
//!   against in 3 of 5 languages is dropped from defaults.
//!
//! The windows are measured by the measures themselves, through `run`, so what is tested is
//! what `measure` computes. The statistics are `support/genre.rs`, which A8's arm
//! (`tests/ge_t6_a8.rs`) shares.

#[path = "support/genre.rs"]
mod genre;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_graph::Store;
use smysl_measure::num::from_q20;
use smysl_measure::rng::Rng;
use smysl_measure::text::{self, Corpus, Doc};
use smysl_measure::{run, Level, Num, Outcome, Recipe, Resolved};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

use genre::{analyse, genre_names, genres, mean_variance, median, repo, verdict};

const WINDOWS: usize = 20;
const SEEDS: u64 = 10;

/// The two measures of the pure arm: id, window in characters, the value read.
const MEASURES: [(&str, usize, &str); 2] = [("T2", 20_000, "bits/char"), ("T3", 50_000, "H")];

fn lang_of(edition: &str) -> &'static str {
    match edition {
        "deu1912" => "de",
        "eng-kjv" => "en",
        "fraLSG" => "fr",
        "russyn" => "ru",
        "spaRV1909" => "es",
        other => panic!("no language for edition `{other}`"),
    }
}

/// One edition's books, read through a library as a measure reads them: `(code, characters)`.
fn books(edition_dir: &Path, edition: &str) -> Vec<(String, Vec<char>)> {
    let root = std::env::temp_dir().join(format!("smysl-ge-t6-{edition}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    let mut files: Vec<PathBuf> = std::fs::read_dir(edition_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "usfm"))
        .collect();
    files.sort();
    for f in &files {
        let stem = f.file_stem().unwrap().to_string_lossy().to_lowercase();
        let alias = format!("bible/{stem}");
        let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
            .with_lang(LangTag::new(lang_of(edition)).unwrap())
            .with_carry(Carry::Text);
        spec.versification = Some("kjv".to_string());
        library
            .add(
                &ReadInput::new(&std::fs::read(f).unwrap()),
                &spec,
                &Caps::DEFAULT,
            )
            .unwrap_or_else(|e| panic!("{edition} {alias}: {e}"));
    }
    let texts = text::texts("from texts", &library).unwrap();
    let c = Corpus::load(&library, &texts, Level::Part, None).unwrap();
    let out = c
        .docs
        .iter()
        .map(|d| {
            let code = d.alias.rsplit_once('-').unwrap().1.to_uppercase();
            (code, d.text().chars().collect())
        })
        .collect();
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// One window's value under the measure.
fn measure(id: &str, key: &str, window: String) -> f64 {
    let c = Corpus::from_docs(vec![Doc::new("window", vec![window])]);
    let mut recipe = Recipe::new("", 0, "ge-t6");
    recipe.min_n = Some(2);
    let store = Store::new();
    let scope = Resolved::of_uids("ge-t6", BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope).with_text(&c);
    let r = run(&[id], &recipe, &input).remove(0);
    match &r.outcome {
        Outcome::Value(v) => match v.values[key] {
            Num::Fixed(i) => from_q20(i),
            _ => unreachable!(),
        },
        other => panic!("{id} did not run on a window: {other:?}"),
    }
}

/// A book's value under one seed: the median of its 20 windows.
fn book_value(
    id: &str,
    key: &str,
    w: usize,
    edition: &str,
    code: &str,
    chars: &[char],
    seed: u64,
) -> f64 {
    let domain = format!("smysl/ge-t6/1\0{edition}\0{code}\0{id}");
    let mut rng = Rng::new(&smysl_core::hash_bytes(domain.as_bytes()), seed);
    let mut vals: Vec<f64> = (0..WINDOWS)
        .map(|_| {
            let o = rng.index(chars.len() - w + 1);
            measure(id, key, chars[o..o + w].iter().collect())
        })
        .collect();
    median(&mut vals)
}

/// Run `tasks` across the machine's cores; the results come back in task order, so the
/// parallelism cannot reach a value.
fn parallel<T: Sync, R: Send>(tasks: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let out: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(tasks.len()));
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= tasks.len() {
                    break;
                }
                let r = f(&tasks[i]);
                out.lock().unwrap().push((i, r));
            });
        }
    });
    let mut out = out.into_inner().unwrap();
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

#[test]
#[ignore = "needs SMYSL_GE_T6=<dir> of <edition>/*.usfm; run in release"]
fn ge_t6_pure_arm() {
    let Some(dir) = std::env::var_os("SMYSL_GE_T6").map(PathBuf::from) else {
        eprintln!("  no corpus: set SMYSL_GE_T6=<dir> of <edition>/*.usfm (GE-T1's five Bibles)");
        return;
    };
    let labels = genres();
    let genre_names = genre_names(&labels);
    let mut editions: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| dir.join(n).is_dir())
        .collect();
    editions.sort();

    let mut out = String::from(
        "# GE-T6's pure arm (SMYSL-2.6 §5.4): T2 and T3 across the Bible's genres, per edition.\n\
         # Written by crates/smysl-measure/tests/ge_t6.rs under SMYSL_BLESS=1 with SMYSL_GE_T6\n\
         # naming GE-T1's five Bibles. Values to 4 places; H is Kruskal-Wallis, ties corrected.\n\
         # edition\tmeasure\tkey\tvalue\n",
    );
    let mut against: BTreeMap<&str, usize> = BTreeMap::new();
    for edition in &editions {
        let all = books(&dir.join(edition), edition);
        for (id, w, key) in MEASURES {
            let mut excluded = Vec::new();
            let mut kept: Vec<(&str, usize, &[char])> = Vec::new();
            for (code, chars) in &all {
                let g = labels.get(code).map(String::as_str).unwrap_or("-");
                if g == "-" {
                    excluded.push(format!("{code}(unlabelled)"));
                } else if chars.len() < w {
                    excluded.push(format!("{code}({})", chars.len()));
                } else {
                    let gi = genre_names.iter().position(|x| x == g).unwrap();
                    kept.push((code.as_str(), gi, chars.as_slice()));
                }
            }
            let tasks: Vec<(usize, u64)> = (0..kept.len())
                .flat_map(|b| (0..SEEDS).map(move |s| (b, s)))
                .collect();
            let vals = parallel(&tasks, |&(b, s)| {
                book_value(id, key, w, edition, kept[b].0, kept[b].2, s)
            });
            // by_seed[b][s]
            let by_seed: Vec<&[f64]> = vals.chunks(SEEDS as usize).collect();
            let x: Vec<f64> = by_seed.iter().map(|v| v[0]).collect();
            let g: Vec<usize> = kept.iter().map(|k| k.1).collect();
            let a = analyse(&x, &g, genre_names.len(), mean_variance(&by_seed));
            if a.against {
                *against.entry(id).or_default() += 1;
            }
            out.push_str(&a.rows(edition, id, &excluded, "var:window", &genre_names));
            eprintln!(
                "  {edition} {id}: {} books, H {:.2}, p {:.4}, against {}",
                a.n, a.h, a.p, a.against
            );
        }
    }
    for (id, _, _) in MEASURES {
        let k = against.get(id).copied().unwrap_or(0);
        out.push_str(&verdict(id, k, editions.len()));
    }
    print!("{out}");
    let path = repo("fixtures/measure/ge-t6.tsv");
    if std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &out).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, out, "{} changed", path.display());
}
