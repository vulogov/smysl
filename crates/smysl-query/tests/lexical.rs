//! The lexical source (TX-P6 step 3; SMYSL-2.5 §3.3, §3.4).
//!
//! The step's exit has two halves and they check different things:
//!
//! 1. *"single-partition results equal today's `find` ranking on
//!    `fixtures/retrieval/queries.tsv`"* — that routing a query through the grammar, the
//!    planner, a `Subset` view and RRF does not change the answer. Every one of those four is
//!    a place a ranking could quietly shift.
//! 2. *"RRF determinism test"* — that fusion is a function of its inputs and not of the order
//!    they arrived in.
//!
//! The query set is the shared one, read from the same file `smysl-retrieve`'s and
//! `smysl-embed`'s evaluations read, because a set copied into a third evaluation drifts.

use std::collections::BTreeSet;

use smysl_core::surface::parse_surface;
use smysl_graph::Store;
use smysl_query::{plan, run, Answer, Env, IdfScope, PlanOptions, StoreScope};
use smysl_retrieve::{Bm25, Query as RQuery, Retriever};

/// `(fixture, query)` from the shared set; the labels the evaluation grades against are not
/// this test's business, because this test compares two rankings rather than judging one.
fn cases() -> Vec<(String, String)> {
    let path = "../../fixtures/retrieval/queries.tsv";
    let src = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut out = Vec::new();
    for line in src.lines() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(f.len(), 4, "{path}: expected four tab-separated fields");
        out.push((f[0].to_string(), f[3].to_string()));
    }
    assert!(!out.is_empty(), "{path} yielded no cases");
    out
}

fn store_of(fixture: &str) -> Store {
    let path = format!("../../fixtures/corpus/{fixture}.smy");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Store::from_records(parse_surface(&src).expect("corpus fixture parses").records)
}

/// A `find` through the whole stack: grammar, planner, executor, fusion.
fn through_sq(store: &Store, name: &str, text: &str, opts: &PlanOptions) -> Answer {
    let scope = StoreScope::new(name, store);
    let env = Env::new(store, &scope);
    // The text goes back through the grammar as a quoted string, so the test exercises the
    // lexer's string rules on every query in the set as well.
    let q = format!("from store:{name}\nfind {text:?} ~lex");
    let parsed = smysl_query::parse(&q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let plan = plan(&parsed.query, &env, opts)
        .unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message));
    run(&plan, &env, opts)
        .unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message))
        .answer
}

/// The exit clause's first half.
///
/// A corpus fixture carries no manifest, so its scope partitions into one — which is the
/// single-partition case the clause names, and the case where RRF is order-preserving: with
/// one list, `1/(k + rank)` is strictly decreasing in rank, so fusing cannot reorder it. Any
/// difference found here is therefore a real difference in what was scored, not an artefact
/// of fusing.
#[test]
fn a_single_partition_ranks_as_find_does() {
    let opts = PlanOptions::default();
    let mut compared = 0;
    for (fixture, text) in cases() {
        let store = store_of(&fixture);

        let direct = Bm25::index(&store).search(&RQuery::new(&text, opts.limit));
        let want: Vec<_> = direct.iter().map(|h| h.uid).collect();
        let got = through_sq(&store, &fixture, &text, &opts).uids();

        assert_eq!(want, got, "{fixture}: {text:?}");
        compared += 1;
    }
    assert!(compared >= 20, "only {compared} queries compared");
}

/// The same, with the per-term contributions: `--why` is only worth printing if the numbers
/// behind it are the ones that produced the ranking.
#[test]
fn the_contributions_survive_the_journey() {
    let opts = PlanOptions::default();
    for (fixture, text) in cases() {
        let store = store_of(&fixture);
        let direct = Bm25::index(&store).search(&RQuery::new(&text, opts.limit));
        let Answer::Ranked(fused) = through_sq(&store, &fixture, &text, &opts) else {
            panic!("a `find` should rank");
        };
        for (hit, f) in direct.iter().zip(&fused) {
            assert_eq!(f.placements.len(), 1, "one partition");
            assert_eq!(hit.terms, f.placements[0].terms, "{fixture}: {text:?}");
            assert_eq!(hit.score, f.placements[0].score, "{fixture}: {text:?}");
        }
    }
}

/// The exit clause's second half.
///
/// Determinism, at the reading that can actually fail: the fused order must not depend on the
/// order the partitions were handed over. §3.4 says partitions are summed in sorted
/// partition-key order, and this is the assertion behind that clause — a fusion that iterated
/// a hash map would pass every single-partition test in this file and fail here.
#[test]
fn fusion_does_not_depend_on_the_order_the_partitions_arrive_in() {
    use smysl_query::{fuse::rrf, PartitionKey, Ranked};
    use smysl_retrieve::Hit;

    let uid = |n: u8| smysl_core::Uid::from_bytes([n; 32]);
    let part = |lang: &str| PartitionKey::Catalog {
        manifest: "m".to_string(),
        lang: lang.to_string(),
    };
    let lists = vec![
        Ranked {
            key: part("en"),
            hits: vec![
                Hit::new(uid(1), 3.0),
                Hit::new(uid(2), 2.0),
                Hit::new(uid(3), 1.0),
            ],
        },
        Ranked {
            key: part("ru"),
            hits: vec![Hit::new(uid(3), 9.0), Hit::new(uid(1), 0.5)],
        },
        Ranked {
            key: PartitionKey::Uncatalogued,
            hits: vec![Hit::new(uid(2), 7.0)],
        },
    ];

    let want = rrf(&lists);
    for rotation in 1..lists.len() {
        let mut shuffled = lists.clone();
        shuffled.rotate_left(rotation);
        assert_eq!(want, rrf(&shuffled), "rotated by {rotation}");
    }
    let mut reversed = lists.clone();
    reversed.reverse();
    assert_eq!(want, rrf(&reversed));

    // And the arithmetic itself, by hand, so the test fails if `k` or the 1-based rank moves:
    // uid 3 is rank 3 in `en` and rank 1 in `ru`.
    let three = want.iter().find(|f| f.uid == uid(3)).expect("uid 3 ranked");
    assert_eq!(three.score, 1.0 / 63.0 + 1.0 / 61.0);
    assert_eq!(
        three.placements.iter().map(|p| p.rank).collect::<Vec<_>>(),
        vec![3, 1],
        "sorted partition-key order: Uncatalogued sorts before Catalog, en before ru"
    );
}

/// Two units that fuse to the same score come back in uid order, which is the only tie-break
/// that is a function of the data.
#[test]
fn a_tie_is_broken_by_uid() {
    use smysl_query::{fuse::rrf, PartitionKey, Ranked};
    use smysl_retrieve::Hit;

    let hi = smysl_core::Uid::from_bytes([9; 32]);
    let lo = smysl_core::Uid::from_bytes([1; 32]);
    let fused = rrf(&[
        Ranked {
            key: PartitionKey::Uncatalogued,
            hits: vec![Hit::new(hi, 1.0)],
        },
        Ranked {
            key: PartitionKey::Catalog {
                manifest: "m".to_string(),
                lang: "en".to_string(),
            },
            hits: vec![Hit::new(lo, 1.0)],
        },
    ]);
    assert_eq!(fused[0].score, fused[1].score, "both are rank 1 once");
    assert_eq!(fused[0].uid, lo);
}

/// IDF scope changes what the statistics are counted over, not which units may be returned.
///
/// The claim §3.4 rests on, asserted where it is observable: with several partitions, scoring
/// under `partition` and under `collection` must return the same *units* from each partition
/// — only their scores, and so possibly their order, differ.
#[test]
fn the_idf_scope_moves_the_statistics_and_not_the_candidates() {
    let text = std::fs::read_to_string("../../fixtures/sq/plans/catalog.smy").expect("fixture");
    let store = Store::from_records(parse_surface(&text).expect("it parses").records);

    let mut by_partition = PlanOptions::default();
    by_partition.idf_scope = IdfScope::Partition;
    let collection = PlanOptions::default();
    assert_eq!(collection.idf_scope, IdfScope::Collection, "the default");

    let a = through_sq(&store, "catalog", "pool", &by_partition);
    let b = through_sq(&store, "catalog", "pool", &collection);

    let set = |answer: &Answer| -> BTreeSet<_> { answer.uids().into_iter().collect() };
    assert_eq!(set(&a), set(&b), "the same units, scored differently");
}

/// What `--explain` and `--why` say about a query that actually ranked something.
///
/// The step-2 plan goldens cannot show phase 5 or the fusion, because none of them has a
/// source — so this is the snapshot that does, and it is a snapshot for the same reason those
/// are: a plan is a record of what the phases decided, and a golden a person can read is a
/// golden a person will notice is wrong.
#[test]
fn the_explain_and_why_text_for_a_ranked_query() {
    let store = store_of("F1-incident");
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();

    let parsed = smysl_query::parse("from store:F1-incident\nfind \"connection pool\" ~lex")
        .expect("it parses");
    let plan = plan(&parsed.query, &env, &opts).expect("it plans");
    let Answer::Ranked(fused) = run(&plan, &env, &opts).expect("it runs").answer else {
        panic!("a `find` should rank");
    };

    let got = format!(
        "{}\n{}",
        smysl_query::explain(&plan),
        smysl_query::why(&fused)
    );
    let path = std::path::Path::new("../../fixtures/sq/plans/find-lexical.why");
    if std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1") {
        std::fs::write(path, &got).expect("writable");
        return;
    }
    let want = std::fs::read_to_string(path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, got);
}

/// The sources phase 5 cannot lower are refused by name, each saying what it is waiting for.
///
/// `~sem` and `~hybrid` are not "a later phase" and the message does not say they are: they
/// need a port bound to a model, which is a tier this crate is deliberately not in.
#[test]
fn the_sources_this_build_cannot_lower_are_refused_by_name() {
    let store = store_of("F1-incident");
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();

    for (text, expect) in [
        ("find \"x\" ~sem", "`~sem` consults an embedding model"),
        ("find \"x\" ~hybrid", "`~hybrid` routes between"),
        ("find \"x\" lang:any", "`lang:any` needs a language bridge"),
        ("find \"x\" lang:ru", "`lang:<code>` needs the catalog"),
        (
            "about entity(\"Naomi\")",
            "`about entity(…)` needs entity resolution",
        ),
        ("text(kjv#Ruth.1.1)", "`text(…)` needs the library layer"),
        ("view v/daily", "`view` needs a saved view"),
        ("question \"why\"", "`question` is the interpretive source"),
    ] {
        let q = format!("from store:F1-incident\n{text}");
        let parsed = smysl_query::parse(&q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
        let diags = plan(&parsed.query, &env, &opts).expect_err("refused");
        assert_eq!(diags[0].code.as_str(), "SMY-E461", "{text}");
        assert!(
            diags[0].message.starts_with(expect),
            "{text}: {}",
            diags[0].message
        );
    }
}

/// A bare `find` is lexical (SMYSL-2.9 OQ-83), and plans identically to the one that says so.
#[test]
fn an_absent_engine_is_lexical() {
    let store = store_of("F1-incident");
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();

    let bare = smysl_query::parse("from store:F1-incident\nfind \"pool\"").expect("parses");
    let said = smysl_query::parse("from store:F1-incident\nfind \"pool\" ~lex").expect("parses");
    let a = plan(&bare.query, &env, &opts).expect("plans");
    let b = plan(&said.query, &env, &opts).expect("plans");

    assert_eq!(
        run(&a, &env, &opts).expect("runs").answer,
        run(&b, &env, &opts).expect("runs").answer,
        "a bare `find` and `~lex` must answer the same question"
    );
    // The plans differ in exactly one place: whether the query wrote the engine down. A saved
    // query has to give back what was written, so the two are not the same plan.
    let smysl_query::Source::Lexical(bare) = a.source().expect("a source") else {
        panic!("lexical")
    };
    let smysl_query::Source::Lexical(said) = b.source().expect("a source") else {
        panic!("lexical")
    };
    assert!(!bare.explicit && said.explicit);
}

/// An unbound `$parameter` is `SMY-E460` at execution, not at planning (SMYSL-2.9 §4.10).
///
/// Which is the whole point of the split: the same plan is reusable across bindings, so the
/// plan cannot be the thing that refuses.
#[test]
fn an_unbound_parameter_is_refused_when_it_is_run() {
    let store = store_of("F1-incident");
    let scope = StoreScope::new("F1-incident", &store);
    let env = Env::new(&store, &scope);
    let mut opts = PlanOptions::default();

    let parsed = smysl_query::parse("from store:F1-incident\nfind $q ~lex").expect("parses");
    let plan = plan(&parsed.query, &env, &opts).expect("planning does not need a binding");

    let diags = run(&plan, &env, &opts).expect_err("running does");
    assert_eq!(diags[0].code.as_str(), "SMY-E460");

    opts.params
        .insert("q".to_string(), "connection pool".to_string());
    let bound = run(&plan, &env, &opts).expect("bound, it runs").answer;
    let direct = through_sq(&store, "F1-incident", "connection pool", &opts);
    assert_eq!(bound, direct, "the same plan, bound, is the literal query");
}
