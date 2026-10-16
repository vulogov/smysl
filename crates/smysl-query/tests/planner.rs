//! Planner goldens (TX-P6 step 2; SMYSL-2.5 §3.2).
//!
//! The step's exit is *"plan snapshots for `from store:` queries over `fixtures/corpus`"*, and
//! a snapshot is the right shape for it: a plan is a record of what four phases decided, so
//! what a test should pin is the whole record rather than a field at a time. The snapshot is
//! the `--explain` text, because that is what §3.2 says the phases exist to produce and
//! because a golden a person can read is a golden a person will notice is wrong.
//!
//! Every fixture in `fixtures/corpus` is planned, each against a store built from that one
//! file, so a snapshot's unit counts are a property of the fixture rather than of the order
//! `read_dir` happened to return.
//!
//! `SMYSL_BLESS=1` rewrites the expectations, as it does for the grammar goldens.

use std::path::{Path, PathBuf};

use smysl_graph::Store;
use smysl_query::{explain, plan, Env, PlanOptions, StoreScope};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus")
}

fn plans_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sq/plans")
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

fn compare(path: &Path, got: &str) {
    if blessing() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(path, got).expect("expectation is writable");
        return;
    }
    let want = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; run with SMYSL_BLESS=1 to write it",
            path.display()
        )
    });
    assert_eq!(want, got, "{} changed", path.display());
}

/// Every `.smy` fixture, by name, with a store built from it alone.
fn corpora() -> Vec<(String, Store)> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(corpus_dir()).expect("fixtures/corpus") {
        let path = entry.expect("entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("smy") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("utf-8 name")
            .to_string();
        let text = std::fs::read_to_string(&path).expect("fixture is readable");
        // F6 is "supposed to fail" its *checks* (SMYSL-2.7 §2); it still parses, and a
        // planner has no opinion about a store's health, so it is planned like the rest.
        let parsed = smysl_core::surface::parse_surface(&text).expect("fixture parses");
        out.push((name, Store::from_records(parsed.records)));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    assert!(!out.is_empty(), "no corpus fixtures");
    out
}

fn planned(store: &Store, name: &str, query: &str) -> String {
    let scope = StoreScope::new(name, store);
    let env = Env::new(store, &scope);
    let parsed = smysl_query::parse(query).expect("the query parses");
    let plan = plan(&parsed.query, &env, &PlanOptions::default())
        .unwrap_or_else(|d| panic!("{name}: {} {}", d[0].code, d[0].message));
    explain(&plan)
}

/// The exit clause: `from store:<name>` over every corpus fixture.
#[test]
fn from_store_plans_over_the_corpus() {
    for (name, store) in corpora() {
        let text = planned(&store, &name, &format!("from store:{name}"));
        compare(&plans_dir().join(format!("{name}.explain")), &text);
    }
}

/// Phase 4's catalog branch, which `fixtures/corpus` cannot reach: none of F1 to F13 carries
/// a manifest, so every snapshot above partitions as "uncatalogued". This fixture is one
/// manifest with two parts, the second declaring `lang: ru`, and three units — one sourcing
/// each part and one sourcing no part at all — so the three outcomes phase 4 can produce are
/// all in one snapshot: two catalog partitions keyed on `(manifest, lang)` and the
/// uncatalogued remainder.
///
/// It lives here rather than in `fixtures/corpus` because that tree is F1 to F13 with an
/// `.expected` beside each, iterated by four other crates' tests; a fourteenth file would
/// change what those tests measure.
#[test]
fn a_catalogued_store_partitions_by_manifest_and_language() {
    let path = plans_dir().join("catalog.smy");
    let text = std::fs::read_to_string(&path).expect("the catalog fixture");
    let parsed = smysl_core::surface::parse_surface(&text).expect("it parses");
    let store = Store::from_records(parsed.records);
    let got = planned(&store, "catalog", "from store:catalog");
    assert!(
        got.contains("[ru]") && got.contains("[en]") && got.contains("(no manifest)"),
        "all three partition outcomes should appear:\n{got}"
    );
    compare(&plans_dir().join("catalog.explain"), &got);
}

/// A query with no `from` resolves to the whole scope at plan time (SMYSL-2.9 OQ-79), so it
/// must plan to the same candidate set as naming the store does. Snapshotted for one fixture
/// and asserted for all: the equality is the claim, the snapshot is so a reader can see it.
#[test]
fn an_absent_from_is_the_whole_scope() {
    for (name, store) in corpora() {
        let scope = StoreScope::new(name.clone(), &store);
        let env = Env::new(&store, &scope);
        let named = smysl_query::parse(&format!("from store:{name}")).expect("parses");
        let a = plan(&named.query, &env, &PlanOptions::default()).expect("named plans");
        // There is no query that is *only* an absent `from`: a pipeline must carry at least
        // one stage (SMYSL-2.9 §4.2) and every other stage is phase 5 or later, so no query
        // this build plans can exercise the `None` arm end to end. The arm calls
        // `Scope::everything`, so that is what the equality is asserted against, and the
        // end-to-end case arrives with the first source, in step 3.
        let b = smysl_query::Scope::everything(&scope);
        assert_eq!(a.candidates, b.uids, "{name}");
    }
}

/// `store:` naming a store that is not open is `SMY-E511`, not an empty result.
#[test]
fn an_unknown_store_is_refused() {
    let (name, store) = corpora().remove(0);
    let scope = StoreScope::new(name, &store);
    let env = Env::new(&store, &scope);
    let q = smysl_query::parse("from store:not-open").expect("parses");
    let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_str(), "SMY-E511");
}

/// The five scope items the library layer owns are refused by name, with the code whose
/// meaning is "not available in this build" (SMYSL-2.5 §8).
#[test]
fn the_library_scopes_need_smysl_text() {
    let (name, store) = corpora().remove(0);
    let scope = StoreScope::new(name, &store);
    let env = Env::new(&store, &scope);
    for text in [
        "from texts",
        "from library",
        "from collection:kjv",
        "from channel:ops",
        "from channels",
        "from import:batch-1",
    ] {
        let q = smysl_query::parse(text).expect("parses");
        let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
        assert_eq!(diags[0].code.as_str(), "SMY-E461", "{text}");
    }
}

/// Phases 5 to 9 are refused by name, with the phase that will lower them.
#[test]
fn a_stage_past_phase_four_is_refused_by_name() {
    let (name, store) = corpora().remove(0);
    let scope = StoreScope::new(name.clone(), &store);
    let env = Env::new(&store, &scope);
    for (text, expect) in [
        ("from store:NAME\nfind \"x\" ~lex", "a source"),
        (
            "from store:NAME\nwhere status = measured",
            "a `where` filter",
        ),
        ("from store:NAME\nsaid during 2026-09-01", "a time stage"),
        ("from store:NAME\nexpand ->grounds", "an `expand` traversal"),
        ("from store:NAME\nwith rebuttals", "a `with` closure"),
        ("from store:NAME\nlimit 10", "a shaping stage"),
        ("from store:NAME\nin reading order", "an ordering stage"),
        ("from store:NAME\ncount", "an output stage"),
    ] {
        let q = smysl_query::parse(&text.replace("NAME", &name)).expect("parses");
        let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
        assert_eq!(diags[0].code.as_str(), "SMY-E461", "{text}");
        assert!(
            diags[0].message.starts_with(expect),
            "{text}: {}",
            diags[0].message
        );
    }
}

/// A traversal naming a relation kind nothing declares is `SMY-E462`; a kernel kind is not.
#[test]
fn an_undeclared_relation_kind_is_refused() {
    let (name, store) = corpora().remove(0);
    let scope = StoreScope::new(name.clone(), &store);
    let env = Env::new(&store, &scope);

    let q = smysl_query::parse(&format!("from store:{name}\nexpand ->notakind")).expect("parses");
    let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
    let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
    assert!(codes.contains(&"SMY-E462"), "{codes:?}");

    // `causes` is a kernel kind, so the only complaint left is that traversals are phase 7.
    let q = smysl_query::parse(&format!("from store:{name}\nexpand ->causes")).expect("parses");
    let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
    let codes: Vec<&str> = diags.iter().map(|d| d.code.as_str()).collect();
    assert!(!codes.contains(&"SMY-E462"), "{codes:?}");
}

/// A `using` pin for a declaration the store does not hold is `SMY-E466`.
#[test]
fn an_unpinnable_schema_is_refused() {
    let (name, store) = corpora().remove(0);
    let scope = StoreScope::new(name.clone(), &store);
    let env = Env::new(&store, &scope);
    let q =
        smysl_query::parse(&format!("using x.nothing/v1@3\nfrom store:{name}")).expect("parses");
    let diags = plan(&q.query, &env, &PlanOptions::default()).expect_err("refused");
    assert_eq!(diags[0].code.as_str(), "SMY-E466");
}
