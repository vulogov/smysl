//! Saved queries and reruns (TX-P8 step 1; SMYSL-2.5 §3.6, §5.4).
//!
//! The step's exit is §5.4's first and last bullets. The first — *save, append data, rerun
//! pinned: `schema_effect` absent, `data_effect` equals the appended set* — is a story about a
//! store that changes between two runs of one query, told here over `F1-incident` with two
//! claims appended that the query will find. The last — *saving a query does not change
//! `salience` of any result* — is OQ-49's guard, asserted on the scores themselves and not on
//! the edge kind's label, because the label is what draft 3 got wrong.

use std::collections::BTreeMap;

use smysl_core::surface::parse_surface;
use smysl_core::types::Record;
use smysl_core::{canonical_uid, KernelType, Status, UnitCoreBuilder};
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_query::saved::{self, LoadError};
use smysl_query::{bind, load, plan, rerun, run, to_records, Env, PlanOptions, StoreScope};

const NAME: &str = "F1-incident";
const QUERY: &str = "from store:F1-incident\nfind \"connection pool\"";

fn records_of(fixture: &str) -> Vec<Record> {
    let path = format!("../../fixtures/corpus/{fixture}.smy");
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    parse_surface(&src).expect("corpus fixture parses").records
}

fn store_of(fixture: &str) -> Store {
    Store::from_records(records_of(fixture))
}

/// Plan and run `text` over `store`, and the records a save of it would write.
fn saved_records(store: &Store, text: &str) -> (Vec<smysl_core::ids::Uid>, Vec<Record>) {
    let scope = StoreScope::new(NAME, store);
    let env = Env::new(store, &scope);
    let opts = PlanOptions::default();
    let parsed = smysl_query::parse(text).expect("parses");
    let bound = bind(&parsed.query, &opts.params).expect("nothing to bind");
    let plan = plan(&bound, &env, &opts).expect("plans");
    let run = run(&plan, &env, &opts).expect("runs");
    let results = run.answer.uids();
    let (unit, edges) = to_records(&bound, &plan, &run, &opts).expect("a saved query");
    let mut records = vec![Record::Unit(unit)];
    records.extend(edges.into_iter().map(Record::Relation));
    (results, records)
}

fn a_claim(gist: &str) -> Record {
    Record::Unit(
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .build()
            .expect("a claim"),
    )
}

/// §5.4, first bullet.
#[test]
fn save_append_data_and_rerun_pinned_reports_exactly_the_appended_set() {
    let mut store = store_of(NAME);
    let (results, records) = saved_records(&store, QUERY);
    assert!(results.len() >= 3, "the query finds something to save");
    let saved_uid = match &records[0] {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    };
    store.append(&records).expect("the save appends");

    // The data: two claims the query will rank, appended after the save.
    let added = [
        a_claim("The connection pool was exhausted by a retry storm from the batch job."),
        a_claim("Raising the connection pool ceiling masked the leak for two more days."),
    ];
    let added_uids: Vec<_> = added
        .iter()
        .map(|r| match r {
            Record::Unit(u) => canonical_uid(u),
            _ => unreachable!(),
        })
        .collect();
    store.append(&added).expect("the data appends");

    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();
    let saved = load(&store, &saved_uid).expect("loads");
    assert_eq!(saved.results, {
        let mut r = results.clone();
        r.sort();
        r
    });

    let again = rerun(&saved, &env, &opts, false).expect("reruns");
    assert!(
        again.schema_effect.is_none(),
        "no lens engine, no schema effect"
    );
    assert!(again.recipe_matches, "same options, same recipe");
    let mut want = added_uids.clone();
    want.sort();
    assert_eq!(
        again.data_effect.added, want,
        "the data effect is the appended set"
    );
    assert!(again.data_effect.removed.is_empty());
}

/// §5.4, last bullet: OQ-49's guard, on the scores.
#[test]
fn saving_a_query_changes_no_result_s_salience() {
    let mut store = store_of(NAME);
    let (results, records) = saved_records(&store, QUERY);
    let before = salience(&store, &SalienceRequest::default());
    store.append(&records).expect("the save appends");
    let after = salience(&store, &SalienceRequest::default());
    for uid in &results {
        assert_eq!(
            before.get(uid).to_bits(),
            after.get(uid).to_bits(),
            "{uid}: salience moved when the query was saved"
        );
    }
    // And not only the results: a kernel `answers` edge would have moved its targets, and an
    // inert one moves nothing anywhere. Over the units that existed before the save — the
    // saved query itself is new, and a score it did not have is not a score that moved.
    for uid in before.scores.keys() {
        assert_eq!(before.get(uid).to_bits(), after.get(uid).to_bits(), "{uid}");
    }
}

/// The unit round-trips: the text is the canonical text, the keys are the eight, the edges are
/// the result, and the recipe is the one the plan would compute again.
#[test]
fn a_saved_query_loads_back_as_it_was_written() {
    let mut store = store_of(NAME);
    let (results, records) = saved_records(&store, QUERY);
    let saved_uid = match &records[0] {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    };
    assert_eq!(records.len(), 1 + results.len(), "one edge per result");
    store.append(&records).expect("appends");

    let saved = load(&store, &saved_uid).expect("loads");
    assert_eq!(saved.text, QUERY);
    assert_eq!(saved.sq, "sq/1");
    assert!(saved.models.is_empty(), "a pure plan used no model");
    assert!(saved.schemas.is_empty(), "no `using` pins were written");
    assert_eq!(saved.lens_chain, "", "no chain was applied");
    assert_eq!(
        saved.per_partition,
        Some(10),
        "the limit it ran under (OQ-84)"
    );
    assert_eq!(saved.recipe.len(), 64);
    assert_eq!(saved.result_hash.len(), 64);
    let mut sorted = results.clone();
    sorted.sort();
    assert_eq!(saved.results, sorted);

    // The unit cites the query it carries.
    let unit = store.get(&saved_uid).expect("in the store");
    assert_eq!(unit.core.schema.as_str(), smysl_core::schemas::query::SAVED);
    assert_eq!(unit.core.status, Status::Cited);
    let source = unit.core.source.as_ref().expect("a doc source");
    assert_eq!(source.reference, format!("sq:{}", &saved.recipe[..12]));
}

/// The text saved is the text after binding, and an unbound parameter is `SMY-E460`.
#[test]
fn parameters_are_bound_into_the_saved_text() {
    let parsed =
        smysl_query::parse("from store:F1-incident\nfind $what\nlimit $n").expect("parses");
    let mut params = BTreeMap::new();
    params.insert("what".to_string(), "pool".to_string());
    params.insert("n".to_string(), "3".to_string());
    let bound = bind(&parsed.query, &params).expect("binds");
    assert_eq!(
        smysl_query::print(&bound),
        "from store:F1-incident\nfind \"pool\"\nlimit 3"
    );

    params.remove("n");
    let d = bind(&parsed.query, &params).expect_err("unbound");
    assert_eq!(d.code.as_str(), "SMY-E460");
    assert!(d.message.contains("$n"), "{}", d.message);

    params.insert("n".to_string(), "three".to_string());
    let d = bind(&parsed.query, &params).expect_err("not a count");
    assert_eq!(d.code.as_str(), "SMY-E460");
}

/// With no declarations nothing changes between the revisions a query was saved at and the
/// current ones, so `--current` reports an empty schema effect — asked, and answered (TX-P12
/// step 4; `tests/lensed.rs` has §19.5's, which is not empty).
#[test]
fn a_current_rerun_over_undeclared_data_has_no_schema_effect() {
    let mut store = store_of(NAME);
    let (_, records) = saved_records(&store, QUERY);
    let saved_uid = match &records[0] {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    };
    store.append(&records).expect("appends");
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let saved = load(&store, &saved_uid).expect("loads");
    let again = rerun(&saved, &env, &PlanOptions::default(), true).expect("reruns");
    assert_eq!(again.schema_effect, Some(saved::SetDiff::between(&[], &[])));
    assert!(again.data_effect.is_empty());
}

/// A uid that is not a saved query says what it is instead.
#[test]
fn a_unit_that_is_not_a_saved_query_is_named_for_what_it_is() {
    let store = store_of(NAME);
    let (uid, _) = store.units().next().expect("a unit");
    match load(&store, uid) {
        Err(LoadError::NotASavedQuery(schema)) => assert_eq!(schema, "claim"),
        other => panic!("{other:?}"),
    }
    let absent =
        smysl_core::ids::Uid::parse("b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .expect("a well-formed uid");
    assert_eq!(load(&store, &absent), Err(LoadError::NotInStore));
    let _ = saved::RESULT_EDGE;
}

/// The result hash is a function of the set, not of the order.
#[test]
fn the_result_hash_ignores_order_and_the_recipe_frames_its_inputs() {
    let store = store_of(NAME);
    let uids: Vec<_> = store.units().map(|(u, _)| *u).collect();
    let mut reversed = uids.clone();
    reversed.reverse();
    assert_eq!(saved::result_hash(&uids), saved::result_hash(&reversed));
    assert_ne!(saved::result_hash(&uids), saved::result_hash(&uids[1..]));
}

/// OQ-85: a `find` over the store a query was saved in does not find the saved query, which
/// would otherwise rank first for its own terms; a query that names the schema does.
#[test]
fn find_leaves_saved_queries_out_unless_the_query_names_them() {
    let mut store = store_of(NAME);
    let (_, records) = saved_records(&store, QUERY);
    let saved_uid = match &records[0] {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    };
    store.append(&records).expect("appends");
    let scope = StoreScope::new(NAME, &store);
    let env = Env::new(&store, &scope);
    let opts = PlanOptions::default();
    let answer = |text: &str| {
        let parsed = smysl_query::parse(text).expect("parses");
        let plan = plan(&parsed.query, &env, &opts).expect("plans");
        let uids = run(&plan, &env, &opts).expect("runs").answer.uids();
        (uids, smysl_query::explain(&plan))
    };

    let (plain, explained) = answer(QUERY);
    assert!(!plain.is_empty(), "the corpus still answers");
    assert!(
        !plain.contains(&saved_uid),
        "the save is not data the query found"
    );
    assert!(
        explained.contains("(1 saved query left out: `find` asks of the corpus, OQ-85)"),
        "{explained}"
    );

    let (asked, explained) = answer(
        "from store:F1-incident\nfind \"connection pool\"\nwhere schema = \"x.query/saved\"",
    );
    assert_eq!(
        asked,
        vec![saved_uid],
        "asked for by its schema, it is found"
    );
    assert!(!explained.contains("left out"), "{explained}");

    // A pipeline that does not `find` scores nothing, and leaves nothing out.
    let (all, _) =
        answer("from store:F1-incident\nwhere schema = \"x.query/saved\" or kind = claim");
    assert!(all.contains(&saved_uid));
}
