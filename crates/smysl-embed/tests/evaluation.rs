//! Does a semantic backend actually beat the lexical one, and where?
//!
//! ```text
//! make eval-semantic SMYSL_EMBED_MODEL=/path/to/potion-base-8M
//! ```
//!
//! The same twenty queries as `smysl-retrieve`'s evaluation, read from the same file, so the
//! two tables can be put side by side. That is the only reason to run this at all: 0.5.0
//! measured lexical retrieval at 0.12 precision-at-one on paraphrase and 0.67 recall on
//! `claim`, and the question is whether an embedder moves those numbers enough to justify a
//! model file, a C++ compiler and a second engine.
//!
//! Three retrievers are scored, not two. `Hybrid` is the one that would actually ship, and it
//! can be worse than both of its halves if the routing is wrong — dispatching a query to the
//! engine that answers it badly is a failure mode neither half has on its own.
//!
//! Skips without `SMYSL_EMBED_MODEL`, like the other tests that need something this
//! repository does not carry.

use std::collections::BTreeMap;

use smysl_core::surface::parse_surface;
use smysl_core::{KernelType, Label, SchemaId, Uid};
use smysl_embed::{Hybrid, Model, Semantic};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Echo,
    Paraphrase,
    Identifier,
}

struct Case {
    fixture: String,
    query: String,
    /// The label whose unit should come back first.
    want: String,
    class: Class,
}

/// The query set, read from `fixtures/retrieval/queries.tsv`.
///
/// One source, shared with every other retriever's evaluation. A query set copied into a
/// second evaluation drifts, and two scores measured on different questions say nothing
/// about each other — which would defeat the only reason to run a second evaluation.
fn cases() -> Vec<Case> {
    let path = "../../fixtures/retrieval/queries.tsv";
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut out = Vec::new();
    for (n, line) in src.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            f.len(),
            4,
            "{path}:{}: expected four tab-separated fields",
            n + 1
        );
        out.push(Case {
            fixture: f[0].to_string(),
            class: match f[1] {
                "Echo" => Class::Echo,
                "Paraphrase" => Class::Paraphrase,
                "Identifier" => Class::Identifier,
                other => panic!("{path}:{}: unknown class `{other}`", n + 1),
            },
            want: f[2].to_string(),
            query: f[3].to_string(),
        });
    }
    assert!(!out.is_empty(), "{path} yielded no cases");
    out
}

const K: usize = 5;

fn load(fixture: &str) -> (Store, BTreeMap<Label, Uid>) {
    let path = format!("../../fixtures/corpus/{fixture}.smy");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let out = parse_surface(&src).expect("corpus fixture parses");
    (Store::from_records(out.records.clone()), out.labels)
}

fn kind_of(store: &Store, uid: &Uid) -> Option<KernelType> {
    match &store.get(uid)?.core.schema {
        SchemaId::Kernel(k) => Some(*k),
        _ => None,
    }
}

/// Reciprocal rank of `want`, or 0.0 if it is not in the top `K`.
fn reciprocal_rank(hits: &[smysl_retrieve::Hit], want: &Uid) -> f64 {
    hits.iter()
        .position(|h| h.uid == *want)
        .map(|i| 1.0 / (i as f64 + 1.0))
        .unwrap_or(0.0)
}

#[derive(Default, Clone)]
struct Tally {
    n: usize,
    hits_at_k: usize,
    rr: f64,
    top1: usize,
}

impl Tally {
    fn add(&mut self, rr: f64) {
        self.n += 1;
        self.rr += rr;
        if rr > 0.0 {
            self.hits_at_k += 1;
        }
        if rr == 1.0 {
            self.top1 += 1;
        }
    }
    fn recall(&self) -> f64 {
        self.hits_at_k as f64 / self.n.max(1) as f64
    }
    fn mrr(&self) -> f64 {
        self.rr / self.n.max(1) as f64
    }
    fn precision_at_1(&self) -> f64 {
        self.top1 as f64 / self.n.max(1) as f64
    }
}

/// One engine's scores: by query class, by kernel type, and overall.
struct Scored {
    by_class: BTreeMap<Class, Tally>,
    by_kind: BTreeMap<String, Tally>,
    overall: Tally,
}

#[test]
fn semantic_against_lexical_over_the_corpus() {
    let Ok(model_path) = std::env::var("SMYSL_EMBED_MODEL") else {
        eprintln!("skipped: set SMYSL_EMBED_MODEL to a model directory");
        return;
    };

    let all = cases();
    let mut rows: BTreeMap<&str, Scored> = BTreeMap::new();

    for engine in ["lexical", "semantic", "hybrid"] {
        let mut by_class: BTreeMap<Class, Tally> = BTreeMap::new();
        let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
        let mut overall = Tally::default();

        for case in &all {
            let (store, labels) = load(&case.fixture);
            let want = *labels
                .get(&Label::new(&case.want).expect("label parses"))
                .unwrap_or_else(|| panic!("{}: no label {}", case.fixture, case.want));

            let model = Model::from_dir(&model_path).expect("the model loads");
            let q = Query::new(&case.query, K);
            let hits = match engine {
                "lexical" => Bm25::index(&store).search(&q),
                "semantic" => Semantic::index(&store, model).search(&q),
                _ => Hybrid::new(Bm25::index(&store), Semantic::index(&store, model)).search(&q),
            };

            let rr = reciprocal_rank(&hits, &want);
            overall.add(rr);
            by_class.entry(case.class).or_default().add(rr);
            let kind = kind_of(&store, &want)
                .map(|k| format!("{k:?}"))
                .unwrap_or_else(|| "?".into());
            by_kind.entry(kind).or_default().add(rr);
        }
        rows.insert(
            engine,
            Scored {
                by_class,
                by_kind,
                overall,
            },
        );
    }

    println!("\n{} queries, top {K}\n", all.len());
    println!(
        "{:<12} {:>10} {:>8} {:>8} {:>8}",
        "class", "engine", "recall@5", "MRR", "P@1"
    );
    for class in [Class::Echo, Class::Paraphrase, Class::Identifier] {
        for engine in ["lexical", "semantic", "hybrid"] {
            let t = &rows[engine].by_class[&class];
            println!(
                "{:<12} {:>10} {:>8.2} {:>8.2} {:>8.2}",
                format!("{class:?}"),
                engine,
                t.recall(),
                t.mrr(),
                t.precision_at_1()
            );
        }
    }
    println!();
    println!(
        "{:<12} {:>10} {:>8} {:>8}",
        "kernel type", "engine", "recall@5", "MRR"
    );
    let kinds: Vec<String> = rows["lexical"].by_kind.keys().cloned().collect();
    for kind in kinds {
        for engine in ["lexical", "semantic", "hybrid"] {
            if let Some(t) = rows[engine].by_kind.get(&kind) {
                println!(
                    "{:<12} {:>10} {:>8.2} {:>8.2}",
                    kind,
                    engine,
                    t.recall(),
                    t.mrr()
                );
            }
        }
    }
    println!();
    for engine in ["lexical", "semantic", "hybrid"] {
        let t = &rows[engine].overall;
        println!(
            "{:<12} {:>10} {:>8.2} {:>8.2} {:>8.2}",
            "ALL",
            engine,
            t.recall(),
            t.mrr(),
            t.precision_at_1()
        );
    }
    println!();

    // The assertion is a property, not a target. Not "semantic wins" — that is what the
    // table is for, and pinning a number would freeze in whatever this query set rewards.
    //
    // What must hold is that routing never loses to the engines it routes *between*. That is
    // the whole promise of dispatch: send each query to whichever engine is better at it, and
    // the result cannot be worse than either. The first version of this crate failed exactly
    // that — 0.78 MRR against pure semantic's 0.84 — while passing a weaker assertion that
    // only compared it to lexical. This is the assertion that would have caught it.
    let lex = rows["lexical"].overall.mrr();
    let sem = rows["semantic"].overall.mrr();
    let hyb = rows["hybrid"].overall.mrr();
    assert!(
        hyb >= lex - 1e-9 && hyb >= sem - 1e-9,
        "the hybrid ({hyb:.2} MRR) is worse than an engine it routes between \
         (lexical {lex:.2}, semantic {sem:.2}); routing is costing more than it earns"
    );
}
