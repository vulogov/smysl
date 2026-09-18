//! Retrieval over extension-schema units (1.7).
//!
//! Until now a unit whose schema was not a kernel type was left out of every index — not filtered
//! from a result, absent from the corpus — so `find` and `pack --query` could not reach a store
//! authored under an extension schema at all, and said nothing about why. The code called that "a
//! real gap, recorded rather than papered over"; this is the record of it closing.

use smysl_core::surface::parse_surface;
use smysl_core::{KernelType, Label, SchemaId, Status, Uid};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

const SRC: &str = "\
@schema x.code/v1 { version: 1 }

@claim c/kernel { status: speculative }
~ The connection pool saturated under load.

@x.code/decision d/ext { status: speculative }
~ Pin the connection pool size rather than tracking the range.

@x.code/consequence k/ext { status: inferred, grounds: [d/ext] }
~ Connection pool churn becomes a deliberate act.
";

fn corpus() -> (Store, Vec<(&'static str, Uid)>) {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
    let named = vec![
        ("c/kernel", uid("c/kernel")),
        ("d/ext", uid("d/ext")),
        ("k/ext", uid("k/ext")),
    ];
    (Store::from_records(out.records), named)
}

fn hits(index: &Bm25, q: &Query) -> Vec<Uid> {
    index.search(q).into_iter().map(|h| h.uid).collect()
}

/// A query that names no kind finds every unit that matches its words, whatever its schema.
#[test]
fn an_extension_unit_is_reachable_at_all() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    let all = hits(&index, &Query::new("connection pool", 10));
    for label in ["c/kernel", "d/ext", "k/ext"] {
        assert!(
            all.contains(&by(label)),
            "{label} is missing from an unfiltered result: {all:?}"
        );
    }
    assert_eq!(index.len(), 3, "every unit is in the index");
}

/// A kinds filter names kernel types, so a unit that has none cannot satisfy one — asking for
/// `claim` must not return an `x.code/decision`.
#[test]
fn a_kernel_kind_filter_excludes_extension_units() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    let claims = hits(
        &index,
        &Query::new("connection pool", 10).kinds([KernelType::Claim]),
    );
    assert_eq!(claims, vec![by("c/kernel")]);
}

/// And `schemas` is how a caller asks for them by name, kernel or extension.
#[test]
fn schemas_names_what_to_retrieve() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;
    let ext = |s: &str| SchemaId::parse(s).unwrap();

    assert_eq!(
        hits(
            &index,
            &Query::new("connection pool", 10).schemas([ext("x.code/decision")])
        ),
        vec![by("d/ext")]
    );

    // Two schemas, one of them a kernel type: the filter is over schemas, not over a category.
    let both = hits(
        &index,
        &Query::new("connection pool", 10)
            .schemas([ext("x.code/decision"), SchemaId::Kernel(KernelType::Claim)]),
    );
    assert_eq!(both.len(), 2);
    assert!(both.contains(&by("d/ext")) && both.contains(&by("c/kernel")));

    // A schema nothing in the store uses returns nothing, rather than everything.
    assert!(hits(
        &index,
        &Query::new("connection pool", 10).schemas([ext("x.other/thing")])
    )
    .is_empty());
}

/// The other filters still apply to an extension unit, since they say nothing about kernel type.
#[test]
fn status_and_candidate_filters_still_apply() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    // `k/ext` is inferred, `d/ext` is speculative.
    let inferred = hits(
        &index,
        &Query::new("connection pool", 10).min_status(Status::Inferred),
    );
    assert_eq!(inferred, vec![by("k/ext")]);

    let within = hits(
        &index,
        &Query::new("connection pool", 10).within([by("d/ext")]),
    );
    assert_eq!(within, vec![by("d/ext")]);
}
