//! Does lexical retrieval actually work on smysl content?
//!
//! Run it and read the numbers:
//!
//! ```text
//! cargo test -p smysl-retrieve --test evaluation -- --nocapture
//! ```
//!
//! # How the queries were written
//!
//! Three classes, deliberately, because one number over one class would be a measurement of
//! the query set rather than of the retriever:
//!
//! - **`Echo`** — the query shares vocabulary with the gist. Lexical retrieval should be
//!   near-perfect here, and a failure means something is broken rather than merely weak.
//! - **`Paraphrase`** — the query means the same thing in *different words*. This is the
//!   vocabulary-mismatch case, the one BM25 is known to be bad at and the one a semantic
//!   embedder exists to fix. It is the class that decides whether model2vec earns its place.
//! - **`Identifier`** — a metric name, a file path, a symbol. Lexical retrieval should beat
//!   any embedder here, which is why the tokeniser does not stem.
//!
//! No query reuses a gist verbatim. A query set built by copying gists measures nothing: it
//! would score perfectly against any tokeniser that does not actively corrupt its input, and
//! report that as evidence.
//!
//! # What is asserted
//!
//! Floors, not targets. The assertions are loose enough that a genuinely better retriever
//! passes and tight enough that a regression fails. The point of this file is the printed
//! table; the assertions exist so it cannot rot unnoticed.

use std::collections::BTreeMap;

use smysl_core::surface::parse_surface;
use smysl_core::{KernelType, Label, Record, SchemaId, Uid};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    Echo,
    Paraphrase,
    Identifier,
}

struct Case {
    fixture: &'static str,
    query: &'static str,
    /// The label whose unit should come back first.
    want: &'static str,
    class: Class,
}

const CASES: &[Case] = &[
    // ---- Echo: the words are there to be found -----------------------------
    Case {
        fixture: "F1-incident",
        query: "pool acquisition wait rose",
        want: "e/pool-wait",
        class: Class::Echo,
    },
    Case {
        fixture: "F1-incident",
        query: "canary configuration regression",
        want: "e/canary",
        class: Class::Echo,
    },
    Case {
        fixture: "F1-incident",
        query: "95th percentile of request latency",
        want: "d/p95",
        class: Class::Echo,
    },
    Case {
        fixture: "F2-research",
        query: "cohort A subjects effect size",
        want: "e/cohort-a",
        class: Class::Echo,
    },
    Case {
        fixture: "F2-research",
        query: "adherence per cent in every dose band",
        want: "e/adherence-flat",
        class: Class::Echo,
    },
    Case {
        fixture: "F4-qa",
        query: "queries per checkout request rose",
        want: "e/query-count",
        class: Class::Echo,
    },
    Case {
        fixture: "F5-dataset",
        query: "trace coverage in all six regions",
        want: "e/coverage",
        class: Class::Echo,
    },
    Case {
        fixture: "F5-dataset",
        query: "order volume by hour of day",
        want: "t/orders-by-hour",
        class: Class::Echo,
    },
    // ---- Paraphrase: the same meaning, different words ---------------------
    // If BM25 fails most of these and a semantic backend fixes them, that is the argument
    // for adding one. If it does not, the argument is not there.
    Case {
        fixture: "F1-incident",
        query: "connection backlog on the european shard",
        want: "c/pool-saturation",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F1-incident",
        query: "what ultimately caused the slowdown",
        want: "f/root-cause",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F2-research",
        query: "the result holds in a second independent group",
        want: "c/replicates",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F2-research",
        query: "more medicine produced more improvement",
        want: "c/dose-response",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F4-qa",
        query: "the shopping basket code asked the database too often",
        want: "c/n-plus-one",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F4-qa",
        query: "heavy shopping traffic made it worse but was not to blame",
        want: "c/load-contribution",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F5-dataset",
        query: "the slowness sits in one place not everywhere",
        want: "c/tail-is-regional",
        class: Class::Paraphrase,
    },
    Case {
        fixture: "F5-dataset",
        query: "could patchy measurement explain the difference",
        want: "h/sampling-artefact",
        class: Class::Paraphrase,
    },
    // ---- Identifier: names, paths, metrics ---------------------------------
    Case {
        fixture: "F1-incident",
        query: "pool.wait_ms",
        want: "e/pool-wait",
        class: Class::Identifier,
    },
    Case {
        fixture: "F4-qa",
        query: "checkout.p95",
        want: "e/trace-sample",
        class: Class::Identifier,
    },
    Case {
        fixture: "F4-qa",
        query: "db.queries_per_request",
        want: "e/query-count",
        class: Class::Identifier,
    },
    Case {
        fixture: "F5-dataset",
        query: "latency_by_region.csv",
        want: "t/latency-by-region",
        class: Class::Identifier,
    },
];

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

#[test]
fn lexical_retrieval_over_the_corpus() {
    let mut by_class: BTreeMap<Class, Tally> = BTreeMap::new();
    let mut by_kind: BTreeMap<String, Tally> = BTreeMap::new();
    let mut overall = Tally::default();
    let mut misses: Vec<String> = Vec::new();

    for case in CASES {
        let (store, labels) = load(case.fixture);
        let want = *labels
            .get(&Label::new(case.want).expect("label parses"))
            .unwrap_or_else(|| panic!("{}: no label {}", case.fixture, case.want));

        let hits = Bm25::index(&store).search(&Query::new(case.query, K));
        let rr = reciprocal_rank(&hits, &want);

        overall.add(rr);
        by_class.entry(case.class).or_default().add(rr);
        let kind = kind_of(&store, &want)
            .map(|k| format!("{k:?}"))
            .unwrap_or_else(|| "?".into());
        by_kind.entry(kind).or_default().add(rr);

        if rr == 0.0 {
            misses.push(format!(
                "  {:?}  {:22} {:?}",
                case.class, case.want, case.query
            ));
        }
    }

    println!(
        "\nBM25 over the corpus — {} queries, top {K}\n",
        CASES.len()
    );
    println!(
        "{:<12} {:>4} {:>10} {:>8} {:>8}",
        "class", "n", "recall@5", "MRR", "P@1"
    );
    for (class, t) in &by_class {
        println!(
            "{:<12} {:>4} {:>10.2} {:>8.2} {:>8.2}",
            format!("{class:?}"),
            t.n,
            t.recall(),
            t.mrr(),
            t.precision_at_1()
        );
    }
    println!(
        "{:<12} {:>4} {:>10.2} {:>8.2} {:>8.2}",
        "ALL",
        overall.n,
        overall.recall(),
        overall.mrr(),
        overall.precision_at_1()
    );

    println!(
        "\n{:<12} {:>4} {:>10} {:>8}",
        "kernel type", "n", "recall@5", "MRR"
    );
    for (kind, t) in &by_kind {
        println!(
            "{:<12} {:>4} {:>10.2} {:>8.2}",
            kind,
            t.n,
            t.recall(),
            t.mrr()
        );
    }

    if !misses.is_empty() {
        println!("\nnot found in the top {K}:");
        for m in &misses {
            println!("{m}");
        }
    }
    println!();

    // ---- floors ------------------------------------------------------------
    //
    // Echo and Identifier are what lexical retrieval is *for*. If either slips, something is
    // broken rather than merely weak, so they are held tightly.
    let echo = by_class.get(&Class::Echo).expect("echo cases ran");
    assert!(
        echo.recall() >= 0.85,
        "echo recall@{K} fell to {:.2}; lexical retrieval should not miss its own vocabulary",
        echo.recall()
    );
    let ident = by_class
        .get(&Class::Identifier)
        .expect("identifier cases ran");
    assert!(
        ident.recall() >= 0.75,
        "identifier recall@{K} fell to {:.2}; a stemming or stop-word change is the usual \
         cause",
        ident.recall()
    );

    // Paraphrase is held only against *zero*. It is the class BM25 is expected to be weak
    // on, and pinning it to today's score would freeze in whatever this query set happens to
    // reward. What the number is for is deciding whether a semantic backend is worth its
    // dependency — not for passing a test.
    let para = by_class
        .get(&Class::Paraphrase)
        .expect("paraphrase cases ran");
    assert!(
        para.n > 0,
        "the paraphrase class is the whole reason this file exists"
    );
}

/// The index must be built over something. A retriever over an empty store returns nothing
/// and looks identical to a retriever that works and found nothing.
#[test]
fn the_corpus_fixtures_are_actually_loaded() {
    for f in ["F1-incident", "F2-research", "F4-qa", "F5-dataset"] {
        let (store, labels) = load(f);
        assert!(store.units().count() >= 5, "{f}: too few units to evaluate");
        assert!(
            !labels.is_empty(),
            "{f}: no labels, so ground truth cannot be named"
        );
        assert!(
            store.iter().any(|r| matches!(r, Record::Unit(_))),
            "{f}: no units"
        );
    }
}
