//! The semantic stages (TX-P10 step 4; SMYSL-2.5 §3.2–§3.5, §5.7).
//!
//! Over a stub [`SemanticSource`] that scores by word overlap, so every stage that consults a
//! model is exercised in a pure test and in milliseconds. The stub's numbers mean nothing; what
//! is asserted is what the planner and executor do with a model's numbers — the purity label,
//! `SMY-E416`, the route a `~hybrid` takes, per-partition fusion against the one ranking of the
//! embedding bridge, `near(…)`'s threshold, and `SMY-W467` on a rerun under another model. The
//! real models are the facade's tests.
//!
//! `SMYSL_BLESS=1` rewrites `fixtures/sq/plans/semantic.explain`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use smysl_core::ids::Uid;
use smysl_core::surface::parse_surface;
use smysl_core::types::Record;
use smysl_graph::Store;
use smysl_query::{
    bind, explain, load, plan, rerun, run, to_records, Answer, Env, PlanOptions, Purity,
    SemanticSource, StoreScope,
};
use smysl_retrieve::Hit;

/// Jaccard over lowercased words, standing in for a cosine.
struct Overlap {
    gists: BTreeMap<Uid, BTreeSet<String>>,
    hash: [u8; 32],
}

fn words(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

impl Overlap {
    fn over(store: &Store, hash: u8) -> Overlap {
        Overlap {
            gists: store
                .units()
                .map(|(u, unit)| (*u, words(&unit.core.gist)))
                .collect(),
            hash: [hash; 32],
        }
    }
}

impl SemanticSource for Overlap {
    fn model_hash(&self) -> [u8; 32] {
        self.hash
    }

    fn search(&self, query: &str, within: &BTreeSet<Uid>, limit: usize) -> Vec<Hit> {
        let q = words(query);
        let mut hits: Vec<Hit> = within
            .iter()
            .filter_map(|u| {
                let g = self.gists.get(u)?;
                let both = q.intersection(g).count() as f32;
                let either = q.union(g).count() as f32;
                let score = if either == 0.0 { 0.0 } else { both / either };
                (score > 0.0).then(|| Hit::new(*u, score))
            })
            .collect();
        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap()
                .then_with(|| a.uid.cmp(&b.uid))
        });
        hits.truncate(limit);
        hits
    }
}

fn store_of(path: &str) -> Store {
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Store::from_records(parse_surface(&src).expect("fixture parses").records)
}

fn f1() -> Store {
    store_of("../../fixtures/corpus/F1-incident.smy")
}

fn catalog() -> Store {
    store_of("../../fixtures/sq/plans/catalog.smy")
}

fn planned(store: &Store, name: &str, port: &dyn SemanticSource, text: &str) -> smysl_query::Plan {
    let scope = StoreScope::new(name, store);
    let env = Env::new(store, &scope).with_semantic(port);
    let q = smysl_query::parse(text).unwrap_or_else(|d| panic!("{text}: {}", d.message));
    plan(&q.query, &env, &PlanOptions::default())
        .unwrap_or_else(|d| panic!("{text}: {} {}", d[0].code, d[0].message))
}

fn answer(
    store: &Store,
    name: &str,
    port: &dyn SemanticSource,
    text: &str,
    opts: &PlanOptions,
) -> Answer {
    let scope = StoreScope::new(name, store);
    let env = Env::new(store, &scope).with_semantic(port);
    let q = smysl_query::parse(text).unwrap_or_else(|d| panic!("{text}: {}", d.message));
    let p = plan(&q.query, &env, opts).unwrap_or_else(|d| panic!("{text}: {}", d[0].message));
    run(&p, &env, opts).expect("runs").answer
}

/// Every plan that consults the model, with the reason it gives.
const SEMANTIC: &[(&str, &str)] = &[
    ("find \"connection pool saturation\" ~sem", "~sem"),
    (
        "find \"connection pool saturation\" ~hybrid",
        "~hybrid routed semantic",
    ),
    ("find $q ~hybrid", "~hybrid routed on a parameter"),
    ("find \"pool\" ~sem lang:any", "embedding bridge"),
    (
        "find \"pool\" | where near(\"connection pool\", 0.2)",
        "near",
    ),
    ("where near(\"canary\", 0.1)", "near"),
    (
        "find \"pool\" ~sem\nunion\nfrom store:F1-incident\nfind \"canary\" ~lex",
        "~sem",
    ),
];

/// §3.2 and the step's exit: every semantic plan is labelled model-dependent, says why, and is
/// refused by `--seed-check` with `SMY-E416` before it runs. The plans are snapshotted, so the
/// explain line a reader sees for each is pinned.
#[test]
fn every_semantic_plan_is_model_dependent_and_refused_by_seed_check() {
    let store = f1();
    let port = Overlap::over(&store, 7);
    let mut golden = String::new();
    for (text, why) in SEMANTIC {
        let q = format!("from store:F1-incident\n{text}");
        let p = planned(&store, "F1-incident", &port, &q);
        match &p.purity {
            Purity::ModelDependent(reasons) => {
                assert!(reasons.contains(why), "{text}: {reasons:?}")
            }
            other => panic!("{text} planned {other:?}"),
        }
        let refused = p.seed_check().expect_err("seed-check refuses it");
        assert_eq!(refused.code.as_str(), "SMY-E416", "{text}");
        assert_eq!(
            p.models,
            vec!["07".repeat(32)],
            "{text}: the model is recorded"
        );
        golden.push_str(&format!("# {text}\n{}\n", explain(&p)));
    }

    // And the other side: a pure plan with a port bound is still pure, records no model, and
    // passes.
    for text in [
        "find \"pool\"",
        "find \"pool.wait_ms\" ~hybrid",
        "where kind = claim",
    ] {
        let q = format!("from store:F1-incident\n{text}");
        let p = planned(&store, "F1-incident", &port, &q);
        assert_eq!(p.purity, Purity::Pure, "{text}");
        assert!(p.models.is_empty() && p.seed_check().is_ok(), "{text}");
    }

    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sq/plans/semantic.explain");
    if std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(&path, &golden).expect("bless");
    } else {
        let want = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
        assert_eq!(want, golden, "{} changed", path.display());
    }
}

/// `~sem` ranks by the model's scores, highest first.
#[test]
fn sem_ranks_by_the_source() {
    let store = f1();
    let port = Overlap::over(&store, 1);
    let text = "from store:F1-incident\nfind \"connection pool saturation\" ~sem";
    let Answer::Ranked(hits) = answer(&store, "F1-incident", &port, text, &PlanOptions::default())
    else {
        panic!("a ranking")
    };
    assert!(!hits.is_empty());
    let top = store.get(&hits[0].uid).unwrap();
    assert!(
        top.core.gist.to_lowercase().contains("pool"),
        "{}",
        top.core.gist
    );
}

/// Without `lang:any` a semantic source ranks per partition and RRF fuses, as a lexical one
/// does (§3.4); with it, one ranking over every partition, the cosine as the score, and each
/// hit placed in its own partition (§3.5).
#[test]
fn the_bridge_is_one_ranking_where_fusion_is_one_a_partition() {
    let store = catalog();
    let port = Overlap::over(&store, 2);
    let opts = PlanOptions::default();
    let q = "from store:catalog\nfind \"unit source part manifest\" ~sem";
    let Answer::Ranked(fused) = answer(&store, "catalog", &port, q, &opts) else {
        panic!("a ranking")
    };
    // Three partitions, one unit each: every unit first in its own, so three equal RRF scores.
    assert_eq!(fused.len(), 3);
    assert!(fused.iter().all(|f| f.placements[0].rank == 1));
    assert!(fused.windows(2).all(|w| w[0].score == w[1].score));

    let q = "from store:catalog\nfind \"unit source part manifest\" ~sem lang:any";
    let Answer::Ranked(bridged) = answer(&store, "catalog", &port, q, &opts) else {
        panic!("a ranking")
    };
    assert_eq!(bridged.len(), 3);
    let ranks: Vec<usize> = bridged.iter().map(|f| f.placements[0].rank).collect();
    assert_eq!(ranks, vec![1, 2, 3], "one list, so one rank each");
    assert!(bridged.windows(2).all(|w| w[0].score >= w[1].score));
    assert!(bridged
        .iter()
        .all(|f| (f.score - f64::from(f.placements[0].score)).abs() < 1e-9));
    let keys: BTreeSet<_> = bridged
        .iter()
        .map(|f| f.placements[0].key.clone())
        .collect();
    assert_eq!(keys.len(), 3, "each in its own partition");
}

/// `~hybrid` on a parameter routes on the bound text: a name goes to BM25 and gives what
/// `~lex` gives; a sentence goes to the model.
#[test]
fn a_hybrid_parameter_routes_on_its_binding() {
    let store = f1();
    let port = Overlap::over(&store, 3);
    let mut opts = PlanOptions::default();
    let q = "from store:F1-incident\nfind $q ~hybrid";

    opts.params.insert("q".into(), "pool.wait_ms".into());
    let routed = answer(&store, "F1-incident", &port, q, &opts);
    let lexical = answer(
        &store,
        "F1-incident",
        &port,
        "from store:F1-incident\nfind \"pool.wait_ms\" ~lex",
        &opts,
    );
    assert_eq!(routed, lexical);

    opts.params
        .insert("q".into(), "connection pool saturation".into());
    let routed = answer(&store, "F1-incident", &port, q, &opts);
    let sem = answer(
        &store,
        "F1-incident",
        &port,
        "from store:F1-incident\nfind \"connection pool saturation\" ~sem",
        &opts,
    );
    assert_eq!(routed, sem);
}

/// `near(…)` keeps the units whose score reaches the threshold, and only those; a threshold
/// outside (0, 1] is refused.
#[test]
fn near_holds_at_and_above_its_threshold() {
    let store = f1();
    let port = Overlap::over(&store, 4);
    let opts = PlanOptions::default();
    let all: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
    let scores: BTreeMap<Uid, f32> = port
        .search("connection pool", &all, all.len())
        .into_iter()
        .map(|h| (h.uid, h.score))
        .collect();
    for t in ["0.05", "0.1", "0.2", "0.5"] {
        let q = format!("from store:F1-incident\nwhere near(\"connection pool\", {t})");
        let got: BTreeSet<Uid> = answer(&store, "F1-incident", &port, &q, &opts)
            .uids()
            .into_iter()
            .collect();
        let t: f32 = t.parse().unwrap();
        let want: BTreeSet<Uid> = scores
            .iter()
            .filter(|(_, s)| **s >= t)
            .map(|(u, _)| *u)
            .collect();
        assert_eq!(got, want, "threshold {t}");
    }

    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope).with_semantic(&port);
    for t in ["0.0", "1.5"] {
        let q = format!("from store:F1-incident\nwhere near(\"x\", {t})");
        let parsed = smysl_query::parse(&q).expect("parses");
        let d = plan(&parsed.query, &env, &opts).expect_err("refused");
        assert_eq!(d[0].code.as_str(), "SMY-E460", "{t}");
    }
}

/// §3.6: a saved semantic query records its model, and a rerun under another raises `SMY-W467`;
/// under the same one it does not.
#[test]
fn a_rerun_under_another_model_is_w467() {
    let mut store = f1();
    let text = "from store:F1-incident\nfind \"connection pool saturation\" ~sem";
    let opts = PlanOptions::default();
    let saved_uid = {
        let port = Overlap::over(&store, 5);
        let scope = StoreScope::new("F1-incident", &store);
        let env = Env::new(&store, &scope).with_semantic(&port);
        let parsed = smysl_query::parse(text).unwrap();
        let bound = bind(&parsed.query, &opts.params).unwrap();
        let p = plan(&bound, &env, &opts).unwrap();
        let r = run(&p, &env, &opts).unwrap();
        let (unit, edges) = to_records(&bound, &p, &r, &opts).unwrap();
        let uid = smysl_core::canonical_uid(&unit);
        let mut records = vec![Record::Unit(unit)];
        records.extend(edges.into_iter().map(Record::Relation));
        store.append(&records).unwrap();
        uid
    };
    let saved = load(&store, &saved_uid).expect("loads");
    assert_eq!(saved.models, vec!["05".repeat(32)]);

    for (hash, warned) in [(5u8, false), (6u8, true)] {
        let port = Overlap::over(&store, hash);
        let scope = StoreScope::new("F1-incident", &store);
        let env = Env::new(&store, &scope).with_semantic(&port);
        let again = rerun(&saved, &env, &opts, false).expect("reruns");
        let w467 = again
            .run
            .warnings
            .iter()
            .any(|d| d.code.as_str() == "SMY-W467");
        assert_eq!(w467, warned, "model {hash}");
    }
}
