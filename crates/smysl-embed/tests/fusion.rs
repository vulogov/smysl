//! Should `~hybrid` fuse lexical and semantic results rather than route? SMYSL-2.5 OQ-46.
//!
//! ```text
//! SMYSL_EMBED_MODEL=/path/to/potion-base-8M \
//! SMYSL_EMBED_MODEL_MULTILINGUAL=/path/to/potion-multilingual-128M \
//!     cargo test -p smysl-embed --release --test fusion -- --nocapture
//! ```
//!
//! `Hybrid` routes because a merge was measured worse: 0.78 MRR against pure semantic's 0.84
//! over `fixtures/retrieval/queries.tsv` with `potion-base-8M`. That merge averaged ranks, and
//! the question OQ-46 leaves open is narrower — whether **RRF**, the fusion `sq` already uses
//! across partitions, does better, and whether a multilingual corpus changes the answer. So
//! four engines are scored over three corpora, with every model the environment names:
//!
//! - **lexical** (`Bm25`), **semantic** (`Semantic`), **routed** (`Hybrid`, what ships), and
//!   **fused**: `Σ 1 / (k + rank)` over the two engines' full rankings, `k = 60`, ties by uid.
//! - **queries**: the twenty shared queries (echo, paraphrase, identifier), English.
//! - **ruth**: the forty restatements of `fixtures/proposition/ge-t5/paraphrase.tsv`, each
//!   searched among the KJV's 85 verses of Ruth and the ten planted near misses; the verse it
//!   restates is the answer. Within one language, worded apart — the case a merge is for.
//! - **verses**: `fixtures/stability/corpus/`, 59 verses in five languages; each verse searched
//!   among the other four languages' 236, its four translations the answer. Across languages,
//!   where a term overlap has only names and numbers to work with.
//!
//! The fused row is also swept over `k`, because OQ-45 asks whether `k` should be a parameter
//! and this is the one fusion where a unit is ranked twice, so `k` can move an order at all.
//!
//! Model-dependent, as `evaluation.rs` is: nothing runs without a model directory, and a figure
//! from one machine is not claimed for another. No assertion beyond the corpora loading; the
//! tables are what the run is for, and they are copied into SMYSL-2.5 §3.4.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::surface::parse_surface;
use smysl_core::{canonical_uid, KernelType, Label, Record, Status, Uid, UnitCoreBuilder};
use smysl_embed::{Hybrid, Model, Semantic};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Hit, Query, Retriever};

const K: f64 = 60.0;
const SWEEP: [f64; 6] = [1.0, 10.0, 30.0, 60.0, 100.0, 1000.0];
/// How much of a fused list counts as "the order" when a sweep asks whether `k` moved it.
const TOP: usize = 10;
const ENGINES: [&str; 4] = ["lexical", "semantic", "routed", "fused"];

/// One search: the ranked uids each engine returned, and the answers.
struct Searched {
    lexical: Vec<Uid>,
    semantic: Vec<Uid>,
    routed: Vec<Uid>,
    gold: BTreeSet<Uid>,
}

fn rrf(a: &[Uid], b: &[Uid], k: f64) -> Vec<Uid> {
    let mut score: BTreeMap<Uid, f64> = BTreeMap::new();
    for list in [a, b] {
        for (i, u) in list.iter().enumerate() {
            *score.entry(*u).or_default() += 1.0 / (k + i as f64 + 1.0);
        }
    }
    let mut out: Vec<(Uid, f64)> = score.into_iter().collect();
    out.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap().then(x.0.cmp(&y.0)));
    out.into_iter().map(|(u, _)| u).collect()
}

#[derive(Default, Clone, Copy)]
struct Tally {
    n: usize,
    rr: f64,
    top1: usize,
    /// Share of the answers found in the top `|gold|`, summed.
    recall: f64,
}

impl Tally {
    fn add(&mut self, ranked: &[Uid], gold: &BTreeSet<Uid>) {
        self.n += 1;
        if let Some(i) = ranked.iter().position(|u| gold.contains(u)) {
            self.rr += 1.0 / (i as f64 + 1.0);
            if i == 0 {
                self.top1 += 1;
            }
        }
        let found = ranked
            .iter()
            .take(gold.len())
            .filter(|u| gold.contains(u))
            .count();
        self.recall += found as f64 / gold.len() as f64;
    }
    fn row(&self) -> String {
        let n = self.n.max(1) as f64;
        format!(
            "{:>6.3} {:>6.3} {:>6.3}",
            self.rr / n,
            self.top1 as f64 / n,
            self.recall / n
        )
    }
}

fn hits(h: Vec<Hit>) -> Vec<Uid> {
    h.into_iter().map(|h| h.uid).collect()
}

fn search(bm25: &Bm25, sem: &Semantic, text: &str, within: &BTreeSet<Uid>) -> Searched {
    let q = Query::new(text, within.len()).within(within.iter().copied());
    let lexical = hits(bm25.search(&q));
    let semantic = hits(sem.search(&q));
    let routed = if Hybrid::<Bm25>::routes_to_lexical(&q) {
        lexical.clone()
    } else {
        semantic.clone()
    };
    Searched {
        lexical,
        semantic,
        routed,
        gold: BTreeSet::new(),
    }
}

fn load(dir: &str) -> Model {
    Model::from_dir(dir).unwrap_or_else(|e| panic!("{dir}: the model would not load: {e}"))
}

fn unit(gist: &str) -> (Uid, Record) {
    let core = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .expect("a gist");
    (canonical_uid(&core), Record::Unit(core))
}

/// One corpus fixture, indexed once for every query that names it.
struct Fixture {
    bm25: Bm25,
    sem: Semantic,
    labels: BTreeMap<Label, Uid>,
    all: BTreeSet<Uid>,
}

/// The twenty shared queries, each over its own fixture.
fn queries(model: &str) -> Vec<Searched> {
    let src = std::fs::read_to_string("../../fixtures/retrieval/queries.tsv").expect("queries");
    let mut out = Vec::new();
    let mut indexed: BTreeMap<String, Fixture> = BTreeMap::new();
    for line in src.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        let Fixture {
            bm25,
            sem,
            labels,
            all,
        } = indexed.entry(f[0].to_string()).or_insert_with(|| {
            let path = format!("../../fixtures/corpus/{}.smy", f[0]);
            let parsed = parse_surface(&std::fs::read_to_string(&path).expect("fixture"))
                .expect("the fixture parses");
            let store = Store::from_records(parsed.records.clone());
            let all = store.units().map(|(u, _)| *u).collect();
            Fixture {
                bm25: Bm25::index(&store),
                sem: Semantic::index(&store, load(model)),
                labels: parsed.labels,
                all,
            }
        });
        let want = labels[&Label::new(f[2]).expect("a label")];
        let mut s = search(bm25, sem, f[3], all);
        s.gold.insert(want);
        out.push(s);
    }
    out
}

/// KJV Ruth's verses by `Ruth.c.v`, read from the committed USFM slice.
fn kjv_ruth() -> BTreeMap<String, String> {
    let src = std::fs::read_to_string("../../fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm")
        .expect("the slice");
    let mut out = BTreeMap::new();
    let mut chapter = 0;
    for line in src.lines() {
        if let Some(c) = line.strip_prefix("\\c ") {
            chapter = c.trim().parse().expect("a chapter number");
        } else if let Some(v) = line.strip_prefix("\\v ") {
            let (n, text) = v.split_once(' ').expect("`\\v n text`");
            let text = text.replace(['[', ']'], "").replace("¶ ", "");
            out.insert(format!("Ruth.{chapter}.{n}"), text.trim().to_string());
        }
    }
    assert_eq!(out.len(), 85, "Ruth has 85 verses");
    out
}

fn ruth(model: &str) -> Vec<Searched> {
    let verses = kjv_ruth();
    let src = std::fs::read_to_string("../../fixtures/proposition/ge-t5/paraphrase.tsv")
        .expect("the paraphrase set");
    let rows: Vec<Vec<String>> = src
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#') && !l.starts_with("id\t"))
        .map(|l| l.split('\t').map(str::to_string).collect())
        .collect();
    let mut records = Vec::new();
    let mut at: BTreeMap<String, Uid> = BTreeMap::new();
    for (loc, text) in &verses {
        let (u, r) = unit(text);
        at.insert(loc.clone(), u);
        records.push(r);
    }
    let mut corpus: BTreeSet<Uid> = at.values().copied().collect();
    for r in rows.iter().filter(|r| r[3] == "not") {
        let (u, rec) = unit(&r[4]);
        corpus.insert(u);
        records.push(rec);
    }
    let store = Store::from_records(records);
    let (bm25, sem) = (Bm25::index(&store), Semantic::index(&store, load(model)));
    let mut out = Vec::new();
    for r in rows.iter().filter(|r| r[3] == "same") {
        let mut s = search(&bm25, &sem, &r[4], &corpus);
        s.gold.insert(at[&r[2]]);
        out.push(s);
    }
    assert_eq!(out.len(), 40);
    out
}

fn verses(model: &str) -> Vec<Searched> {
    const LANGS: [&str; 5] = ["de", "en", "es", "fr", "ru"];
    let mut text: Vec<Vec<String>> = Vec::new();
    for l in LANGS {
        let src = std::fs::read_to_string(format!("../../fixtures/stability/corpus/{l}.txt"))
            .expect("the corpus");
        text.push(
            src.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.split_once(' ').expect("`<ref> <text>`").1.to_string())
                .collect(),
        );
    }
    let mut records = Vec::new();
    let mut uid: Vec<Vec<Uid>> = Vec::new();
    for lang in &text {
        let mut row = Vec::new();
        for v in lang {
            let (u, r) = unit(v);
            row.push(u);
            records.push(r);
        }
        uid.push(row);
    }
    let store = Store::from_records(records);
    // One index per engine for the whole corpus: 295 searches would otherwise load the model
    // 295 times.
    let bm25 = Bm25::index(&store);
    let sem = Semantic::index(&store, load(model));
    let mut out = Vec::new();
    for (l, lang) in text.iter().enumerate() {
        let others: BTreeSet<Uid> = (0..5)
            .filter(|m| *m != l)
            .flat_map(|m| uid[m].iter().copied())
            .collect();
        for (i, v) in lang.iter().enumerate() {
            let mut s = search(&bm25, &sem, v, &others);
            s.gold = (0..5).filter(|m| *m != l).map(|m| uid[m][i]).collect();
            out.push(s);
        }
    }
    out
}

fn report(corpus: &str, model: &str, searched: &[Searched]) {
    let mut t: BTreeMap<&str, Tally> = BTreeMap::new();
    for s in searched {
        t.entry("lexical").or_default().add(&s.lexical, &s.gold);
        t.entry("semantic").or_default().add(&s.semantic, &s.gold);
        t.entry("routed").or_default().add(&s.routed, &s.gold);
        t.entry("fused")
            .or_default()
            .add(&rrf(&s.lexical, &s.semantic, K), &s.gold);
    }
    let name = std::path::Path::new(model)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    println!(
        "\n{corpus} · {name} · {} searches\n  {:<9} {:>6} {:>6} {:>6}",
        searched.len(),
        "engine",
        "MRR",
        "P@1",
        "R@|g|"
    );
    for e in ENGINES {
        println!("  {e:<9} {}", t[e].row());
    }
    for k in SWEEP {
        let mut tk = Tally::default();
        let mut moved = 0usize;
        for s in searched {
            let fused = rrf(&s.lexical, &s.semantic, k);
            tk.add(&fused, &s.gold);
            if fused[..TOP.min(fused.len())]
                != rrf(&s.lexical, &s.semantic, K)[..TOP.min(fused.len())]
            {
                moved += 1;
            }
        }
        println!(
            "  k={k:<7} {}  {moved} top-{TOP} differ from k=60's",
            tk.row()
        );
    }
}

fn models() -> Vec<String> {
    ["SMYSL_EMBED_MODEL", "SMYSL_EMBED_MODEL_MULTILINGUAL"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .collect()
}

#[test]
fn routing_against_rrf_fusion() {
    let models = models();
    if models.is_empty() {
        eprintln!("skipped: set SMYSL_EMBED_MODEL and/or SMYSL_EMBED_MODEL_MULTILINGUAL");
        return;
    }
    for m in &models {
        report("queries", m, &queries(m));
        report("ruth", m, &ruth(m));
        report("verses", m, &verses(m));
    }
}
