//! GE-T3 (SMYSL-2.6 §5.5), the instrument: is a cross-lingual A-measure a fact about the text
//! or about the language it was extracted in?
//!
//! The gate's material is 50 verses × 5 translations × 5 extraction runs, and per verse the
//! units extracted (A1's count, before it is scaled by words) and the classes they form. For
//! each verse and each figure the table of languages × runs is decomposed two ways, as a
//! two-way analysis of variance without replication decomposes it — the language mean square
//! `R · Σ_l (m_l − m)² / (L − 1)` and the run mean square `L · Σ_r (m_r − m)² / (R − 1)` — and
//! each is summed over the verses. **Language variance above run variance** says extraction
//! differs by language more than it differs between two runs of one language: the bias is a
//! finding, and every cross-lingual A* figure carries it (draft 3 §23). The gate reports which,
//! and is decided by having been measured, not by the answer.
//!
//! # Nested, not crossed
//!
//! The decomposition above crosses run with language, as if run 3 of German and run 3 of
//! Russian were one level of something. They are not: runs are independent within a language,
//! so the run variance GE-T3 asks about is the **within-language** mean square,
//! `Σ_l Σ_r (x_lr − m_l)² / (L(R − 1))`, and that is what the verdict reads. The crossed run
//! mean square is printed beside it and is smaller on the measured table (13.50 against 21.76).
//!
//! # What was measured
//!
//! GE-T2's corpus (`fixtures/stability/corpus/`: 59 verses, the same references in all five
//! tier-1 languages, Russian's Ps.13 the others' Ps.14) through hosted `deepseek-chat` at
//! temperature 0, two passes per window, five runs per language, each run its own library,
//! `ingest --text corpus --window-level line`; then `same-as propose --engine lexical --yes` on
//! each, and classes under `strict`. The material is 59 rather than 50 verses because the corpus
//! is 59 and choosing 50 of them would be a choice. The table is `fixtures/measure/ge-t3.tsv`,
//! written from the libraries by `ge_t3_table_from_runs`:
//!
//! ```text
//! SMYSL_GE_T3_RUNS=<dir>   <lang>/<run>/store.smysl, rebuilds the table (--ignored)
//! SMYSL_GE_T3=<file.tsv>   `verse<TAB>lang<TAB>run<TAB>units<TAB>classes`, decomposes another
//! ```
//!
//! **Language variance is above run variance for both figures** — units 74.90 against 21.76,
//! classes 24.34 against 9.20, summed over the 59 verses — so the bias is a finding and every
//! cross-lingual A* figure carries it. One run failed as extraction: Russian run 5 wrote English
//! gists for a Cyrillic passage, `source` policy refused them (`SMY-W436`), and two units
//! survived, neither with a span. It is in the table as the run it was; with run 5 dropped from
//! every language the verdict is the same (73.43 against 16.90, 25.38 against 6.87).
//! `ge_t3_measured` pins the committed table's figures on every run of the suite.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn sq(x: f64) -> f64 {
    x * x
}

/// One verse's cells: `(lang, run) -> value`.
type Table = BTreeMap<(String, u32), f64>;

/// `(language mean square, run mean square)` of one full table; `None` when a language or a
/// run is missing a cell, or there are fewer than two of either.
fn mean_squares(t: &Table) -> Option<(f64, f64)> {
    let langs: BTreeSet<&String> = t.keys().map(|(l, _)| l).collect();
    let runs: BTreeSet<u32> = t.keys().map(|(_, r)| *r).collect();
    let (nl, nr) = (langs.len(), runs.len());
    if nl < 2 || nr < 2 || t.len() != nl * nr {
        return None;
    }
    let m = t.values().sum::<f64>() / t.len() as f64;
    let mean_of = |f: &dyn Fn(&(String, u32)) -> bool| {
        let v: Vec<f64> = t.iter().filter(|(k, _)| f(k)).map(|(_, v)| *v).collect();
        v.iter().sum::<f64>() / v.len() as f64
    };
    let ss_l: f64 = langs
        .iter()
        .map(|l| sq(mean_of(&|k| &k.0 == *l) - m))
        .sum::<f64>()
        * nr as f64;
    let ss_r: f64 = runs
        .iter()
        .map(|r| sq(mean_of(&|k| k.1 == *r) - m))
        .sum::<f64>()
        * nl as f64;
    Some((ss_l / (nl - 1) as f64, ss_r / (nr - 1) as f64))
}

/// The run variance GE-T3 is about, nested: runs are independent **within** a language — run 3
/// of German and run 3 of Russian are not one level of anything — so the comparison is the
/// language mean square against the **within-language** one, `Σ_l Σ_r (x_lr − m_l)² / (L(R − 1))`.
/// `None` as [`mean_squares`] is.
fn within_language(t: &Table) -> Option<f64> {
    let langs: BTreeSet<&String> = t.keys().map(|(l, _)| l).collect();
    let runs: BTreeSet<u32> = t.keys().map(|(_, r)| *r).collect();
    let (nl, nr) = (langs.len(), runs.len());
    if nl < 2 || nr < 2 || t.len() != nl * nr {
        return None;
    }
    let ss: f64 = langs
        .iter()
        .map(|l| {
            let xs: Vec<f64> = t
                .iter()
                .filter(|(k, _)| &k.0 == *l)
                .map(|(_, v)| *v)
                .collect();
            let m = xs.iter().sum::<f64>() / xs.len() as f64;
            xs.iter().map(|x| sq(x - m)).sum::<f64>()
        })
        .sum();
    Some(ss / (nl * (nr - 1)) as f64)
}

/// Summed over verses: the within-language mean square.
fn nested(tables: &BTreeMap<String, Table>) -> f64 {
    tables.values().filter_map(within_language).sum()
}

/// Summed over verses: `(language, run, verses decomposed)`.
fn decompose(tables: &BTreeMap<String, Table>) -> (f64, f64, usize) {
    let mut out = (0.0, 0.0, 0);
    for t in tables.values() {
        if let Some((l, r)) = mean_squares(t) {
            out.0 += l;
            out.1 += r;
            out.2 += 1;
        }
    }
    out
}

/// The two figures' tables from a sheet.
fn read(path: &PathBuf) -> [BTreeMap<String, Table>; 2] {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out: [BTreeMap<String, Table>; 2] = Default::default();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() || line.starts_with("verse\t") {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            f.len(),
            5,
            "a row is `verse lang run units classes`: {line}"
        );
        let run: u32 = f[2].parse().unwrap_or_else(|_| panic!("run `{}`", f[2]));
        for (k, x) in [f[3], f[4]].iter().enumerate() {
            let v: f64 = x.parse().unwrap_or_else(|_| panic!("value `{x}`"));
            out[k]
                .entry(f[0].to_string())
                .or_default()
                .insert((f[1].to_string(), run), v);
        }
    }
    out
}

/// A planted table: `verses` verses, five languages and five runs, the value
/// `base + lang_step · l + run_step · r`.
fn planted(verses: usize, lang_step: f64, run_step: f64) -> BTreeMap<String, Table> {
    let mut out = BTreeMap::new();
    for v in 0..verses {
        let mut t = Table::new();
        for l in 0..5u32 {
            for r in 0..5u32 {
                t.insert(
                    (format!("l{l}"), r),
                    10.0 + v as f64 + lang_step * l as f64 + run_step * r as f64,
                );
            }
        }
        out.insert(format!("v{v}"), t);
    }
    out
}

#[test]
fn ge_t3() {
    if let Some(path) = std::env::var_os("SMYSL_GE_T3").map(PathBuf::from) {
        for (name, tables) in ["units", "classes"].iter().zip(read(&path)) {
            let (l, r, n) = decompose(&tables);
            let w = nested(&tables);
            println!(
                "GE-T3 {name}: {n} verse(s); language mean square {l:.4}, within-language run \
                 {w:.4} (crossed run {r:.4}): {}",
                if l > w {
                    "language variance is above run variance, and cross-lingual A* carries the bias"
                } else {
                    "run variance is at or above language variance"
                }
            );
            assert!(
                n > 0,
                "GE-T3: no verse has a full table of languages and runs"
            );
        }
        return;
    }

    // Languages two units apart and runs identical: all of the variance is language. A table
    // is five by five, so the language mean square is 5 · Σ(2l − 4)² / 4 = 50 per verse.
    let (l, r, n) = decompose(&planted(50, 2.0, 0.0));
    assert_eq!(n, 50);
    assert!((l - 50.0 * 50.0).abs() < 1e-9 && r.abs() < 1e-9, "{l} {r}");
    // The other way round, and both: the larger step is the larger variance.
    let (l, r, _) = decompose(&planted(50, 0.0, 1.0));
    assert!(l.abs() < 1e-9 && (r - 50.0 * 12.5).abs() < 1e-9, "{l} {r}");
    // Nested, the same runs are Σ(r − 2)² / 4 = 2.5 per language, and so per verse; a language
    // step adds nothing within a language.
    assert!((nested(&planted(50, 0.0, 1.0)) - 50.0 * 2.5).abs() < 1e-9);
    assert!(nested(&planted(50, 2.0, 0.0)).abs() < 1e-9);
    let (l, r, _) = decompose(&planted(50, 1.0, 3.0));
    assert!(r > l);
    // A verse missing a cell is not decomposed.
    let mut short = planted(2, 1.0, 1.0);
    short.get_mut("v0").unwrap().remove(&("l0".to_string(), 0));
    assert_eq!(decompose(&short).2, 1);

    println!(
        "GE-T3 is measured: fixtures/measure/ge-t3.tsv, 59 verses x 5 languages x 5 runs; \
         SMYSL_GE_T3=<file.tsv> decomposes another table."
    );
}

// ---- the measurement -------------------------------------------------------------------------

fn repo(rel: &str) -> PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The committed table: GE-T2's corpus, five languages, five runs against hosted DeepSeek.
const TABLE: &str = "fixtures/measure/ge-t3.tsv";

/// The line a unit was drawn from: the first line of its locator.
fn line_of(reference: &str) -> Option<u64> {
    use smysl_text::locator::{self, Locator};
    let mut l = locator::parse(reference.split_once('#')?.1).ok()?;
    loop {
        match l {
            Locator::Line(n) => return Some(n),
            Locator::Range(a, _) => l = *a,
            _ => return None,
        }
    }
}

/// One run's cells: for each line of the corpus, the units drawn from it and the classes they
/// form under A2's default policy. A line nothing was drawn from is a cell of 0 and 0, not a
/// missing one: the run read it and extracted nothing.
fn cells(store: &smysl_graph::Store, lines: u64) -> (BTreeMap<u64, (u64, u64)>, u64) {
    use smysl_core::Uid;
    use smysl_measure::{run, Input, Outcome, Recipe, Resolved};
    let mut by: BTreeMap<u64, BTreeSet<Uid>> = (1..=lines).map(|l| (l, BTreeSet::new())).collect();
    let mut unplaced = 0;
    for (uid, unit) in store.units() {
        match unit
            .core
            .source
            .as_ref()
            .and_then(|s| line_of(&s.reference))
        {
            Some(l) if by.contains_key(&l) => {
                by.get_mut(&l).unwrap().insert(*uid);
            }
            _ => unplaced += 1,
        }
    }
    // `strict`, the normative policy, over the store's same-as edges: `same-as propose --engine
    // lexical --yes` on each run, the engine this build proposes within a language with.
    let mut recipe = Recipe::new("", 0, "ge-t3");
    recipe.policy = Some(smysl_measure::recipe::ClassPolicy::Strict);
    let out = by
        .into_iter()
        .map(|(l, uids)| {
            if uids.is_empty() {
                return (l, (0, 0));
            }
            let n = uids.len() as u64;
            let scope = Resolved::of_uids("ge-t3", uids);
            let input = Input::new(store, &scope);
            let classes = match run(&["A2"], &recipe, &input).remove(0).outcome {
                Outcome::Value(v) => v.n["classes"],
                other => panic!("A2 did not run on line {l}: {other:?}"),
            };
            (l, (n, classes))
        })
        .collect();
    (out, unplaced)
}

/// GE-T3's table from extraction runs: `SMYSL_GE_T3_RUNS=<dir>` of `<lang>/<run>/store.smysl`,
/// each a library holding GE-T2's `<lang>.txt` as `corpus`, ingested once with `ingest --text
/// corpus --window-level line`. Lines are the verse, by place: every file is the same 59
/// references in the same order, Russian's Ps.13 being the others' Ps.14.
#[test]
#[ignore = "needs SMYSL_GE_T3_RUNS=<dir> of <lang>/<run>/store.smysl"]
fn ge_t3_table_from_runs() {
    let Some(dir) = std::env::var_os("SMYSL_GE_T3_RUNS").map(PathBuf::from) else {
        eprintln!("  no runs: set SMYSL_GE_T3_RUNS=<dir> of <lang>/<run>/store.smysl");
        return;
    };
    let list = |d: &std::path::Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| !n.starts_with('.') && d.join(n).is_dir())
            .collect();
        v.sort();
        v
    };
    let mut out = String::from(
        "# GE-T3's table (SMYSL-2.6 §5.5): per verse of GE-T2's corpus, per language and run, the\n\
         # units extracted (A1's count) and the classes they form (A2's, default policy).\n\
         # Written by crates/smysl-measure/tests/ge_t3.rs under SMYSL_BLESS=1 from\n\
         # SMYSL_GE_T3_RUNS. Runs: hosted deepseek-chat, temperature 0, two passes per window.\n\
         verse\tlang\trun\tunits\tclasses\n",
    );
    for lang in list(&dir) {
        let text = std::fs::read_to_string(repo(&format!("fixtures/stability/corpus/{lang}.txt")))
            .unwrap();
        let refs: Vec<&str> = text
            .lines()
            .map(|l| l.split_whitespace().next().unwrap_or(""))
            .collect();
        let en = std::fs::read_to_string(repo("fixtures/stability/corpus/en.txt")).unwrap();
        let verse: Vec<&str> = en
            .lines()
            .map(|l| l.split_whitespace().next().unwrap_or(""))
            .collect();
        assert_eq!(refs.len(), verse.len(), "{lang} is not the corpus's shape");
        for (r, run) in list(&dir.join(&lang)).iter().enumerate() {
            let lib =
                smysl_text::library::Library::open(&dir.join(&lang).join(run).join("store.smysl"))
                    .unwrap_or_else(|e| panic!("{lang}/{run}: {e}"));
            let (cells, unplaced) = cells(lib.store(), refs.len() as u64);
            eprintln!(
                "  {lang} {run}: {} units on {} lines, {unplaced} unplaced",
                cells.values().map(|c| c.0).sum::<u64>(),
                cells.values().filter(|c| c.0 > 0).count()
            );
            for (l, (u, c)) in cells {
                out.push_str(&format!(
                    "{}\t{lang}\t{}\t{u}\t{c}\n",
                    verse[l as usize - 1],
                    r + 1
                ));
            }
        }
    }
    let path = repo(TABLE);
    if std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &out).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, out, "{} changed", path.display());
}

/// GE-T3 as measured: the committed table, decomposed on every run of the suite. Language
/// variance is above within-language run variance for both figures, so the bias is a finding
/// and cross-lingual A* carries it (draft 3 §23).
#[test]
fn ge_t3_measured() {
    let [units, classes] = read(&repo(TABLE));
    for (name, tables, (lang, within)) in [
        ("units", units, (74.9, 21.76)),
        ("classes", classes, (24.34, 9.2)),
    ] {
        let (l, _, n) = decompose(&tables);
        let w = nested(&tables);
        assert_eq!(n, 59, "{name}: every verse has a full table");
        assert!(
            (l - lang).abs() < 1e-9 && (w - within).abs() < 1e-9,
            "{name}: {l} {w}"
        );
        assert!(
            l > w,
            "{name}: language variance is no longer above run variance"
        );
    }
}
