//! Filters through outputs (TX-P6 step 4; SMYSL-2.5 §3.3).
//!
//! The step's exit has two halves:
//!
//! 1. **§3.3 mapping-coverage tests.** §3.3 is a table mapping each `sq` stage to the function
//!    that answers it, and the table is the specification of what this step owes. So the
//!    coverage test walks the table: every row either runs here, or is refused by name with
//!    what it needs — and the test asserts which, row by row, so a row that was quietly
//!    forgotten fails rather than being absent.
//! 2. **Shuffled-append determinism.** A store is a log, and the same records appended in a
//!    different order are the same store. Every stage this step lowers must therefore give the
//!    same answer over a shuffled log, or the stage reads the log rather than the graph.

use std::collections::BTreeSet;

use smysl_core::surface::parse_surface;
use smysl_core::Record;
use smysl_graph::Store;
use smysl_query::{plan, run, Answer, Env, PlanOptions, StoreScope};

const NAME: &str = "F1-incident";

fn store() -> Store {
    let path = format!("../../fixtures/corpus/{NAME}.smy");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    Store::from_records(parse_surface(&src).expect("the fixture parses").records)
}

/// The same records, appended in reverse. A store is a log and this is the same store.
fn reversed() -> Store {
    let path = format!("../../fixtures/corpus/{NAME}.smy");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let mut records: Vec<Record> = parse_surface(&src).expect("parses").records;
    records.reverse();
    Store::from_records(records)
}

fn answer(store: &Store, query: &str) -> Answer {
    answer_with(store, query, &PlanOptions::default())
}

fn answer_with(store: &Store, query: &str, opts: &PlanOptions) -> Answer {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let q = format!("from store:{NAME}\n{query}");
    let parsed = smysl_query::parse(&q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let plan = plan(&parsed.query, &env, opts)
        .unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message));
    run(&plan, &env, opts)
        .unwrap_or_else(|d| panic!("{q}: {} {}", d[0].code, d[0].message))
        .answer
}

fn refusal(store: &Store, query: &str) -> (String, String) {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let q = format!("from store:{NAME}\n{query}");
    let parsed = smysl_query::parse(&q).unwrap_or_else(|d| panic!("{q}: {}", d.message));
    let opts = PlanOptions::default();
    match plan(&parsed.query, &env, &opts) {
        Ok(p) => match run(&p, &env, &opts) {
            Ok(_) => panic!("{q} was not refused"),
            Err(d) => (d[0].code.as_str().to_string(), d[0].message.clone()),
        },
        Err(d) => (d[0].code.as_str().to_string(), d[0].message.clone()),
    }
}

/// Every stage in §3.3's table that this build answers, answered.
///
/// One query per row rather than one per stage class: the table is the unit of the exit, and a
/// row that is listed but never exercised is the failure this test exists to catch.
#[test]
fn every_mapped_stage_this_build_answers_runs() {
    let store = store();

    // --- filters: kernel fields (§3.3 `where kind / schema / status`, `where lang`,
    //     `where commitment`, `where matches`) -------------------------------------------------
    assert!(!answer(&store, "where kind = evidence").uids().is_empty());
    assert!(
        !answer(&store, "where schema = \"smysl.kernel/0.1#claim\"")
            .uids()
            .is_empty()
            || !answer(&store, "where kind = claim").uids().is_empty()
    );
    assert!(!answer(&store, "where status = measured").uids().is_empty());
    assert!(!answer(&store, "where status >= derived").uids().is_empty());
    // `lang` is core key 9 and optional: F1's `@doc` declares `en` but its units carry no
    // per-unit tag, so the honest assertion is that the field is *readable* — the filter runs
    // and admits only units that have it, which here is none.
    assert!(answer(&store, "where lang = en").uids().is_empty());
    assert!(answer(&store, "where not (lang = en)").uids().len() == store.units().count());
    assert!(!answer(&store, "where matches \"pool\"").uids().is_empty());
    // Boolean structure, which §4.5 of the grammar gives and the evaluator has to honour.
    let both = answer(&store, "where kind = evidence and status = measured");
    let either = answer(&store, "where kind = evidence or kind = claim");
    let negated = answer(&store, "where not (kind = evidence)");
    assert!(!both.uids().is_empty() && !either.uids().is_empty() && !negated.uids().is_empty());
    assert!(either.uids().len() > both.uids().len());
    // `in`, over a list.
    let listed = answer(&store, "where status in [measured, cited]");
    assert!(listed.uids().len() >= answer(&store, "where status = measured").uids().len());

    // --- traversals (§3.3 `expand ->k`, `<-k`, `-[k]-`, and the spelled ones) ----------------
    let seed = "where kind = claim";
    let plain = answer(&store, seed).uids().len();
    for stage in [
        "expand ->grounds",
        "expand <-grounds",
        "expand -[grounds]-",
        "expand grounds",
        "expand deps",
        "expand rests on",
        "expand dependents",
        "expand lineage",
    ] {
        let grown = answer(&store, &format!("{seed}\n{stage}")).uids().len();
        assert!(grown >= plain, "{stage} lost units");
    }
    // A traversal adds to the set it was given, which is what `expand` means.
    assert!(
        answer(&store, &format!("{seed}\nexpand ->grounds"))
            .uids()
            .len()
            > plain,
        "the fixture's claims rest on something"
    );
    // A depth bound.
    let deep = answer(&store, &format!("{seed}\nexpand ->grounds depth 1"))
        .uids()
        .len();
    assert!(deep >= plain);

    // --- closures (§3.3 `with rebuttals`, `with grounds`) ------------------------------------
    assert!(
        answer(&store, &format!("{seed}\nwith grounds"))
            .uids()
            .len()
            > plain
    );
    assert!(!answer(&store, &format!("{seed}\nwith rebuttals"))
        .uids()
        .is_empty());

    // --- set operations (§3.3 `union / intersect / except`) ----------------------------------
    let a: BTreeSet<_> = answer(&store, "where kind = evidence")
        .uids()
        .into_iter()
        .collect();
    let b: BTreeSet<_> = answer(&store, "where kind = claim")
        .uids()
        .into_iter()
        .collect();
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();
    let set = |text: &str| -> BTreeSet<smysl_core::Uid> {
        let parsed = smysl_query::parse(text).unwrap_or_else(|d| panic!("{text}: {}", d.message));
        let p = plan(&parsed.query, &env, &opts).expect("plans");
        run(&p, &env, &opts)
            .expect("runs")
            .answer
            .uids()
            .into_iter()
            .collect()
    };
    let left = format!("from store:{NAME}\nwhere kind = evidence");
    let right = format!("from store:{NAME}\nwhere kind = claim");
    assert_eq!(
        set(&format!("{left}\nunion {right}")),
        a.union(&b).copied().collect::<BTreeSet<_>>()
    );
    assert_eq!(
        set(&format!("{left}\nintersect {right}")),
        a.intersection(&b).copied().collect::<BTreeSet<_>>()
    );
    assert_eq!(
        set(&format!("{left}\nexcept {right}")),
        a.difference(&b).copied().collect::<BTreeSet<_>>()
    );

    // --- ordering (§3.3 `order by salience`, `order by <field>`, `group by`, `compare by`) ---
    let by_salience = answer(&store, "order by salience").uids();
    let descending = answer(&store, "order by salience descending").uids();
    assert_eq!(by_salience.len(), descending.len());
    assert_ne!(
        by_salience, descending,
        "a direction that changes nothing is not one"
    );
    let by_status = answer(&store, "order by status").uids();
    assert_eq!(by_status.len(), by_salience.len(), "ordering keeps the set");
    let Answer::Groups(groups) = answer(&store, "group by kind") else {
        panic!("`group by` groups");
    };
    assert!(groups.len() > 1, "the fixture has several kinds");
    assert!(matches!(
        answer(&store, "compare by status"),
        Answer::Groups(_)
    ));

    // --- shaping (§3.3 `limit n`, `pack N tokens`) -------------------------------------------
    assert_eq!(answer(&store, "limit 3").uids().len(), 3);
    assert!(answer(&store, "find \"pool\" ~lex\nlimit 2").uids().len() <= 2);
    assert!(matches!(
        answer(&store, "pack 400 tokens"),
        Answer::Packed(_)
    ));

    // --- outputs (§3.3 `count / units / json / show`) ----------------------------------------
    let Answer::Count(n) = answer(&store, "count") else {
        panic!("`count` counts");
    };
    assert_eq!(n, store.units().count());
    for writer in ["units", "json", "show"] {
        assert_eq!(
            answer(&store, writer).uids().len(),
            n,
            "{writer} is a layout of the same list"
        );
    }
}

/// Every row of §3.3 this build does **not** answer, refused by name.
///
/// The complement of the test above, and the half that keeps the table honest: a stage that
/// quietly did nothing would pass the coverage test by being absent from it.
#[test]
fn every_unmapped_stage_is_refused_with_what_it_needs() {
    let store = store();
    for (stage, expect) in [
        (
            "where near(\"x\", 0.8)",
            "`near(…)` compares against an embedding",
        ),
        (
            "said during 2026-07-02",
            "a time stage reads effective time",
        ),
        ("with holders", "`with holders` and `with speakers`"),
        ("with datings", "`with spans` and `with datings`"),
        ("in reading order", "`in reading order` is span order"),
        ("in time order", "`in time order` reads effective time"),
        ("align by class", "`align by class` needs the class matrix"),
        (
            "pack 400 tokens as view",
            "`as view` appends a `View` record",
        ),
        ("spans", "`spans` reports where each unit sits"),
        ("measure T1", "`measure` dispatches to the measures"),
    ] {
        let (code, message) = refusal(&store, stage);
        assert_eq!(code, "SMY-E461", "{stage}");
        assert!(message.starts_with(expect), "{stage}: {message}");
    }
}

/// An operator that does not apply to the type of the field it names is `SMY-E512`.
///
/// Not `E421` (no schema declares the field) and not `E422` (a key retired without a mapping):
/// `kind` is declared and readable, and what is wrong is the question asked of it. And not a
/// silent answer, which is the alternative — lexicographic order on a name would answer
/// something nobody asked.
#[test]
fn an_operator_that_does_not_fit_its_field_is_refused() {
    let store = store();
    for stage in [
        "where kind > claim",
        "where schema < \"x\"",
        "where lang >= en",
    ] {
        let (code, _) = refusal(&store, stage);
        assert_eq!(code, "SMY-E512", "{stage}");
    }
    // A payload key is refused for its own reason: no declaration gives it a type (a key
    // declared `int` is ordered, `tests/lensed.rs`).
    let (code, message) = refusal(&store, "where holder > \"x\"");
    assert_eq!(code, "SMY-E512");
    assert!(message.contains("no declaration gives"), "{message}");
    // The two ordered fields are not refused.
    let store2 = store;
    assert!(!answer(&store2, "where status > speculative")
        .uids()
        .is_empty());
}

/// The exit clause's second half: a shuffled log is the same store.
///
/// Every stage this step lowers, over records appended in reverse. A stage that read the log
/// rather than the graph — an iteration order leaking into a filter, a traversal keeping the
/// order it reached things in, a group list built from a `Vec` — fails here and passes
/// everything else in this file.
#[test]
fn every_stage_answers_the_same_over_a_shuffled_log() {
    let forward = store();
    let backward = reversed();
    assert_eq!(
        forward.units().count(),
        backward.units().count(),
        "the same records"
    );

    for query in [
        "where kind = evidence",
        "where status >= derived",
        "where matches \"pool\"",
        "where kind = claim and status = derived",
        "where not (kind = evidence)",
        "where status in [measured, cited]",
        "where kind = claim\nexpand ->grounds",
        "where kind = claim\nexpand <-grounds",
        "where kind = claim\nexpand -[grounds]-",
        "where kind = claim\nexpand lineage",
        "where kind = claim\nexpand ->grounds depth 1",
        "where kind = claim\nwith grounds",
        "where kind = claim\nwith rebuttals",
        "order by salience",
        "order by salience descending",
        "order by status",
        "order by status descending",
        "group by kind",
        "compare by status",
        "limit 4",
        "pack 400 tokens",
        "count",
        "units",
        "find \"pool\" ~lex",
        "find \"pool\" ~lex\nlimit 2",
        "find \"pool\" ~lex\nwhere status = measured",
    ] {
        assert_eq!(
            answer(&forward, query),
            answer(&backward, query),
            "{query} depends on append order"
        );
    }

    // A set expression over two shuffled stores, which is the same assertion one level up.
    for op in ["union", "intersect", "except"] {
        let text = format!(
            "from store:{NAME}\nwhere kind = evidence\n{op} from store:{NAME}\nwhere status = measured"
        );
        let run_on = |store: &Store| {
            let scope = StoreScope::new(NAME, store);
            let env = Env::new(store, &scope);
            let opts = PlanOptions::default();
            let parsed = smysl_query::parse(&text).expect("parses");
            let p = plan(&parsed.query, &env, &opts).expect("plans");
            run(&p, &env, &opts).expect("runs").answer
        };
        assert_eq!(run_on(&forward), run_on(&backward), "{op}");
    }
}

/// `SMY-W463`: a traversal that stopped at its depth while the graph went on.
///
/// A warning beside a *successful* answer, which is why `run` returns both. The result is
/// correct for the depth asked; what the caller could not otherwise know is that there was
/// more.
#[test]
fn a_traversal_that_stops_early_says_so() {
    let store = store();
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();

    let q = format!("from store:{NAME}\nwhere kind = claim\nexpand ->grounds depth 0");
    let parsed = smysl_query::parse(&q).expect("parses");
    let p = plan(&parsed.query, &env, &opts).expect("plans");
    let outcome = run(&p, &env, &opts).expect("a depth bound is not an error");
    assert!(
        outcome
            .warnings
            .iter()
            .any(|d| d.code.as_str() == "SMY-W463"),
        "{:?}",
        outcome.warnings
    );

    // Unbounded, there is no frontier left, so nothing is warned about.
    let q = format!("from store:{NAME}\nwhere kind = claim\nexpand ->grounds");
    let parsed = smysl_query::parse(&q).expect("parses");
    let p = plan(&parsed.query, &env, &opts).expect("plans");
    let outcome = run(&p, &env, &opts).expect("runs");
    assert!(outcome.warnings.is_empty(), "{:?}", outcome.warnings);
}

/// A `$parameter` in a `limit` or a `depth` binds between planning and running, like one
/// anywhere else (SMYSL-2.9 §4.10).
#[test]
fn a_parameter_in_a_count_is_bound_at_execution() {
    let store = store();
    let mut opts = PlanOptions::default();
    let unbound = refusal(&store, "limit $n");
    assert_eq!(unbound.0, "SMY-E460");

    opts.params.insert("n".to_string(), "3".to_string());
    assert_eq!(answer_with(&store, "limit $n", &opts).uids().len(), 3);
    opts.params.insert("d".to_string(), "1".to_string());
    assert_eq!(
        answer_with(
            &store,
            "where kind = claim\nexpand ->grounds depth $d",
            &opts
        ),
        answer_with(
            &store,
            "where kind = claim\nexpand ->grounds depth 1",
            &opts
        )
    );
}
