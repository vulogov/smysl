//! R23 (1.6): retrieval filtered by an extension schema's own kind.
//!
//! A corpus built on an extension schema distinguishes its units by a payload field, not by
//! kernel type — a rejected alternative and an anticipated consequence are both `Claim`. `within`
//! answers this correctly and expensively: the caller walks the whole store per query to rebuild
//! an eligible set that is a property of the corpus. Naming the field moves that to indexing time.

use smysl_core::surface::parse_surface;
use smysl_core::{Label, Uid};
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

const SRC: &str = "\
@schema x.code/v1 { version: 1 }

@decision d/pin { status: speculative, \"code:kind\": \"decision\" }
~ Pin the connection pool size rather than tracking the range.

@claim p/drain { status: speculative, \"code:kind\": \"prerequisite\" }
~ The connection pool must be drained before a reconfigure.

@claim r/raise { status: speculative, \"code:kind\": \"rejected-alternative\" }
~ Raise the connection pool ceiling instead.

@claim k/churn { status: speculative, \"code:kind\": \"consequence\" }
~ Connection churn becomes a deliberate act.

@claim n/plain { status: speculative }
~ The connection pool saturated under load.
";

fn corpus() -> (Store, Vec<(&'static str, Uid)>) {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
    let named = vec![
        ("d/pin", uid("d/pin")),
        ("p/drain", uid("p/drain")),
        ("r/raise", uid("r/raise")),
        ("k/churn", uid("k/churn")),
        ("n/plain", uid("n/plain")),
    ];
    (Store::from_records(out.records), named)
}

fn hits(index: &Bm25, q: &Query) -> Vec<Uid> {
    index.search(q).into_iter().map(|h| h.uid).collect()
}

/// The filter returns only the named kinds, and exactly what `within` over the same set returns.
#[test]
fn a_payload_filter_matches_within_over_the_same_set() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    let eligible = vec![by("d/pin"), by("p/drain")];
    let filtered = hits(
        &index,
        &Query::new("connection pool", 10).with_payload("code:kind", ["decision", "prerequisite"]),
    );
    let within = hits(
        &index,
        &Query::new("connection pool", 10).within(eligible.clone()),
    );

    assert_eq!(filtered, within, "the two ways of saying it must agree");
    assert!(!filtered.is_empty());
    assert!(filtered.iter().all(|u| eligible.contains(u)));
    assert!(!filtered.contains(&by("r/raise")));
    assert!(!filtered.contains(&by("k/churn")));
}

/// A unit that does not carry the key is excluded — including every unit of a store built under
/// another schema, which returns nothing rather than everything.
#[test]
fn a_unit_without_the_key_is_excluded() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let plain = named.iter().find(|(l, _)| *l == "n/plain").unwrap().1;

    let all = hits(&index, &Query::new("connection pool", 10));
    assert!(all.contains(&plain), "it matches the query on its words");

    let filtered = hits(
        &index,
        &Query::new("connection pool", 10).with_payload("code:kind", ["decision"]),
    );
    assert!(!filtered.contains(&plain));

    // A key no unit carries admits nothing, rather than everything.
    assert!(hits(
        &index,
        &Query::new("connection pool", 10).with_payload("other:kind", ["decision"])
    )
    .is_empty());
    // And so does naming no value at all: a caller that selected nothing asked for nothing.
    let none: [&str; 0] = [];
    assert!(hits(
        &index,
        &Query::new("connection pool", 10).with_payload("code:kind", none)
    )
    .is_empty());
}

/// Applied before the limit, as `within` is, and scoring is untouched: IDF still comes from the
/// whole index, so the surviving hits keep the scores they had.
#[test]
fn the_restriction_is_applied_before_the_limit_and_does_not_rescore() {
    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    let one = index.search(
        &Query::new("connection pool", 1).with_payload("code:kind", ["rejected-alternative"]),
    );
    assert_eq!(
        one.len(),
        1,
        "a limit of one over a filtered set is one hit"
    );
    assert_eq!(one[0].uid, by("r/raise"));

    let unfiltered = index.search(&Query::new("connection pool", 10));
    let scored = unfiltered
        .iter()
        .find(|h| h.uid == by("r/raise"))
        .expect("it is in the unfiltered result too");
    assert_eq!(one[0].score, scored.score, "a filter must not re-weight");
}

/// Both filters at once, and the payload filter composing with `kinds`.
#[test]
fn the_payload_filter_composes_with_the_others() {
    use smysl_core::KernelType;

    let (store, named) = corpus();
    let index = Bm25::index(&store);
    let by = |n: &str| named.iter().find(|(l, _)| *l == n).unwrap().1;

    // `decision` the payload says, `Decision` the kernel says: the same unit.
    let both = hits(
        &index,
        &Query::new("connection pool", 10)
            .kinds([KernelType::Decision])
            .with_payload("code:kind", ["decision"]),
    );
    assert_eq!(both, vec![by("d/pin")]);

    // Contradictory filters return nothing rather than one of the two.
    assert!(hits(
        &index,
        &Query::new("connection pool", 10)
            .kinds([KernelType::Decision])
            .with_payload("code:kind", ["prerequisite"]),
    )
    .is_empty());

    // And with `within`, the narrower of the two wins.
    assert!(hits(
        &index,
        &Query::new("connection pool", 10)
            .within([by("p/drain")])
            .with_payload("code:kind", ["decision"]),
    )
    .is_empty());
}
