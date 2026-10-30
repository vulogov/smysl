//! **GE-T5, cross-lingual arm** (SMYSL-2.4 owns GE-T5; SMYSL-2.5 §6 TX-P10 step 4).
//!
//! ```text
//! SMYSL_EMBED_MODEL_MULTILINGUAL=<potion-multilingual-128M> \
//!     cargo test --release --features semantic --test ge_t5_cross -- --nocapture
//! SMYSL_EMBED_CANDLE_MODEL=<multilingual-e5-small> \
//!     cargo test --release --features semantic-candle --test ge_t5_cross -- --nocapture
//! ```
//!
//! The within-language arm (`crates/smysl-text/tests/ge_t5.rs`) asks whether an engine pairs a
//! verse with its restatement in the same language. This arm asks it across languages, where a
//! term overlap has nothing to work with and an embedding is the only engine there is: is the
//! verse in one language paired with the same verse in another?
//!
//! **The gold is of record by construction.** `fixtures/stability/corpus/` is 59 verses in each
//! of the five tier-1 languages — Gen.4, Ex.20 and Ps.14 from five public-domain editions — on
//! one line each, in one order. The same line in two files is the same verse, so the pair is
//! `same` because the editions say so and nothing judged it; Russian numbers the psalm 13, the
//! Synodal numbering of the same poem, and the harness checks that every other reference agrees
//! before it trusts the lines. That is also the arm's limit: a verse and its neighbour often
//! share a subject (Ex.20's commandments, the psalm's parallelism), so some `not` pairs are near
//! misses no reading would call wrong, and the precision below is a floor rather than a ceiling.
//!
//! Two figures per model, reported against **no bar** — §5.5's restated clause, an engine
//! scored on the coders' own scale against their agreement with each other, needs a sheet of
//! coders, which this repository does not hold (it is owed, as the within-language arm's is):
//!
//! - **top-1**, per ordered pair of languages: each verse searched among the other language's
//!   59, right when its own translation comes first. Chance is 1/59.
//! - **precision at recall 0.7**, draft 1's statistic and the within-language arm's: every
//!   cross-language pair scored by the mean of its two directions, a threshold swept down the
//!   scores, and the precision read where recall first reaches 0.7 of the 590 gold pairs.
//!
//! The one assertion is a smoke bar set before the first run: top-1 over all 20 ordered pairs at
//! least ten times chance (0.17). It says the vectors cross a language boundary at all; the
//! figures are what the run is for.
//!
//! Both arms are model-dependent: nothing here runs without a model directory, and a figure from
//! one machine is not claimed for another.

#![cfg(any(feature = "semantic", feature = "semantic-candle"))]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl::query::SemanticSource;
use smysl::{canonical_uid, KernelType, Record, Status, Store, Uid, UnitCoreBuilder};

const LANGS: [&str; 5] = ["de", "en", "es", "fr", "ru"];
const RECALL_AT: f64 = 0.7;
const SMOKE_TOP1: f64 = 0.17;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/stability/corpus")
}

/// `verses[l][i]`: line `i` of language `l`, its reference checked against English's.
fn verses() -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut refs: Option<Vec<String>> = None;
    for l in LANGS {
        let text = std::fs::read_to_string(corpus().join(format!("{l}.txt"))).expect("corpus");
        let mut these = Vec::new();
        let mut lines = Vec::new();
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let (r, v) = line.split_once(' ').expect("`<ref> <text>`");
            // Synodal Ps.13 is the Masoretic Ps.14: the same poem, numbered differently.
            these.push(r.replace("Ps.13.", "Ps.14."));
            lines.push(v.to_string());
        }
        match &refs {
            None => refs = Some(these),
            Some(want) => assert_eq!(&these, want, "{l}: the lines are not the same verses"),
        }
        out.push(lines);
    }
    assert!(out.iter().all(|v| v.len() == 59));
    out
}

struct Corpus {
    store: Store,
    /// `uid[l][i]`.
    uid: Vec<Vec<Uid>>,
}

fn build() -> Corpus {
    let verses = verses();
    let mut records = Vec::new();
    let mut uid = Vec::new();
    for lang in &verses {
        let mut row = Vec::new();
        for v in lang {
            let core = UnitCoreBuilder::new(KernelType::Claim, v.as_str(), Status::Speculative)
                .build()
                .expect("a verse is a gist");
            row.push(canonical_uid(&core));
            records.push(Record::Unit(core));
        }
        uid.push(row);
    }
    Corpus {
        store: Store::from_records(records),
        uid,
    }
}

/// Both figures, for one model, printed.
fn measure(name: &str, port: &dyn SemanticSource, c: &Corpus) -> f64 {
    let verses = verses();
    let n = verses[0].len();
    let index: BTreeMap<Uid, (usize, usize)> = c
        .uid
        .iter()
        .enumerate()
        .flat_map(|(l, row)| row.iter().enumerate().map(move |(i, u)| (*u, (l, i))))
        .collect();

    // dir[(l, i, m, j)]: the score of verse j of language m when verse i of language l asks.
    let mut dir: BTreeMap<(usize, usize, usize, usize), f32> = BTreeMap::new();
    let mut right = [[0usize; 5]; 5];
    let t = std::time::Instant::now();
    for (l, lang) in verses.iter().enumerate() {
        let others: BTreeSet<Uid> = (0..5)
            .filter(|m| *m != l)
            .flat_map(|m| c.uid[m].iter().copied())
            .collect();
        for (i, v) in lang.iter().enumerate() {
            let hits = port.search(v, &others, others.len());
            let mut best: [Option<(f32, usize)>; 5] = [None; 5];
            for h in hits {
                let (m, j) = index[&h.uid];
                dir.insert((l, i, m, j), h.score);
                if best[m].is_none_or(|(s, _)| h.score > s) {
                    best[m] = Some((h.score, j));
                }
            }
            for (m, b) in best.iter().enumerate() {
                if m != l && b.is_some_and(|(_, j)| j == i) {
                    right[l][m] += 1;
                }
            }
        }
    }
    let took = t.elapsed();

    println!("\n{name}: top-1, query language down, target across (of {n})");
    println!("        {}", LANGS.join("    "));
    for (l, row) in right.iter().enumerate() {
        let cells: Vec<String> = (0..5)
            .map(|m| {
                if m == l {
                    "  — ".to_string()
                } else {
                    format!("{:>4}", row[m])
                }
            })
            .collect();
        println!("  {}  {}", LANGS[l], cells.join("  "));
    }
    let top1: usize = right.iter().flatten().sum();
    let top1 = top1 as f64 / (20 * n) as f64;

    // Unordered cross-language pairs, scored by the mean of both directions; a direction the
    // source returned nothing for (cosine at or below zero) scores zero.
    let mut pairs: Vec<(f32, bool)> = Vec::new();
    for l in 0..5 {
        for m in (l + 1)..5 {
            for i in 0..n {
                for j in 0..n {
                    let a = dir.get(&(l, i, m, j)).copied().unwrap_or(0.0);
                    let b = dir.get(&(m, j, l, i)).copied().unwrap_or(0.0);
                    pairs.push(((a + b) / 2.0, i == j));
                }
            }
        }
    }
    pairs.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(b.1.cmp(&a.1)));
    let gold = pairs.iter().filter(|p| p.1).count();
    let (mut tp, mut seen) = (0usize, 0usize);
    let mut at = None;
    for (score, same) in &pairs {
        seen += 1;
        if *same {
            tp += 1;
        }
        if tp as f64 / gold as f64 >= RECALL_AT {
            at = Some((tp as f64 / seen as f64, *score));
            break;
        }
    }
    let (precision, threshold) = at.expect("recall reaches 0.7 somewhere down the list");
    println!(
        "{name}: top-1 {top1:.4} over 20 ordered pairs (chance {:.4}); precision {precision:.4} at \
         recall {RECALL_AT} ({gold} gold pairs of {}, threshold {threshold:.4}); {} searches in {:.1} s",
        1.0 / n as f64,
        pairs.len(),
        5 * n,
        took.as_secs_f64()
    );
    top1
}

fn path(var: &str) -> Option<String> {
    let p = std::env::var(var).ok()?;
    assert!(p != "required", "{var}=required needs a path as well");
    Some(p)
}

#[cfg(feature = "semantic")]
#[test]
fn s0_pairs_verses_across_languages() {
    let Some(dir) = path("SMYSL_EMBED_MODEL_MULTILINGUAL") else {
        eprintln!("skipped: set SMYSL_EMBED_MODEL_MULTILINGUAL to potion-multilingual-128M");
        return;
    };
    let c = build();
    let model = smysl::EmbedModel::from_dir(&dir).expect("the model loads");
    let index = smysl::Semantic::index(&c.store, model);
    let hash = index.model_hash();
    let port = smysl::query::RetrieverSource::new(index, hash);
    let top1 = measure("S0 potion-multilingual-128M", &port, &c);
    assert!(
        top1 >= SMOKE_TOP1,
        "top-1 {top1:.4} under the smoke bar {SMOKE_TOP1}"
    );
}

#[cfg(feature = "semantic-candle")]
#[test]
fn s1_pairs_verses_across_languages() {
    let Some(dir) = path("SMYSL_EMBED_CANDLE_MODEL") else {
        eprintln!("skipped: set SMYSL_EMBED_CANDLE_MODEL to multilingual-e5-small");
        return;
    };
    let c = build();
    let pooling = smysl::Pooling::from_dir(&dir).unwrap_or(smysl::Pooling::Mean);
    let encoder = smysl::CandleEncoder::from_dir(&dir, pooling).expect("the encoder loads");
    let hash = encoder.model_hash();
    let index = smysl::CandleSemantic::index(&c.store, encoder).expect("indexes");
    let port = smysl::query::RetrieverSource::new(index, hash);
    let top1 = measure("S1 multilingual-e5-small", &port, &c);
    assert!(
        top1 >= SMOKE_TOP1,
        "top-1 {top1:.4} under the smoke bar {SMOKE_TOP1}"
    );
}

/// The rule `~hybrid` routes by is restated in `smysl-query` (which cannot reach
/// `smysl-embed`); the two must give one answer, or `~hybrid` is not the measured `Hybrid`.
#[cfg(feature = "semantic")]
#[test]
fn hybrid_routing_is_the_measured_rule() {
    for q in [
        "pool.wait_ms",
        "checkout.p95",
        "latency_by_region.csv",
        "a/b",
        "x:y",
        "well-known",
        "connection pool",
        "pool",
        "",
        "  pool.wait_ms  ",
        "pool. wait",
        "наводнение",
    ] {
        assert_eq!(
            smysl::query::routes_to_lexical(q),
            smysl_embed::looks_like_identifier(q),
            "{q:?}"
        );
    }
}
