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
//! # What runs here, and what is owed
//!
//! The material costs 25 extraction runs, a model's each, so it is **owed** until somebody runs
//! them (`SMYSL_INGEST_LIVE=1`, as GE-T2 does). The harness takes the table they produce:
//!
//! ```text
//! SMYSL_GE_T3=<file.tsv>   `verse<TAB>lang<TAB>run<TAB>units<TAB>classes`, one row a cell
//! ```
//!
//! Without it, the scripted arm plants two tables — one whose languages differ and whose runs
//! do not, one the other way round — and checks the decomposition tells them apart exactly.

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
            println!(
                "GE-T3 {name}: {n} verse(s); language mean square {l:.4}, run {r:.4}: {}",
                if l > r {
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
    let (l, r, _) = decompose(&planted(50, 1.0, 3.0));
    assert!(r > l);
    // A verse missing a cell is not decomposed.
    let mut short = planted(2, 1.0, 1.0);
    short.get_mut("v0").unwrap().remove(&("l0".to_string(), 0));
    assert_eq!(decompose(&short).2, 1);

    println!(
        "GE-T3 is OWED: 25 extraction runs (50 verses x 5 translations x 5 runs) have not been \
         made. SMYSL_GE_T3=<file.tsv> reports them."
    );
}
