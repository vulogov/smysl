//! C8 (1.5): a selected unit brings what it rests on over edges the caller names.
//!
//! A producer that links a prerequisite by `conditions` — so rewording it does not move the uid of
//! every decision resting on it — states the dependency in a relation. C1 and C2 read `deps` and
//! `grounds` inside the unit and cannot see it, so a decision packed alone left its prerequisite
//! behind: a rationale that reads as if the decision rested on nothing.

use smysl_core::surface::parse_surface;
use smysl_core::{Label, Lod, Uid};
use smysl_graph::{salience, EdgeSet, SalienceRequest, Store};
use smysl_pack::{pack, verify, PackRequest, Reason};

const SRC: &str = "\
@constraint p/locked { status: speculative }
~ The lockfile must stay unchanged on a release build.

@decision d/pin { status: speculative }
~ Pin the dependency rather than tracking the range.

@claim c/consequence { status: speculative, grounds: [d/pin] }
~ Upgrades become a deliberate act.

@rel p/locked --conditions--> d/pin
";

fn corpus() -> (Store, Uid, Uid, Uid) {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let uid = |l: &str| out.labels[&Label::new(l).unwrap()];
    let (p, d, c) = (uid("p/locked"), uid("d/pin"), uid("c/consequence"));
    (Store::from_records(out.records), p, d, c)
}

fn packed(store: &Store, req: &PackRequest) -> smysl_pack::Pack {
    let p =
        pack(store, &salience(store, &SalienceRequest::default()), req).expect("the floor fits");
    assert!(
        verify(store, &p, req).is_empty(),
        "{:?}",
        verify(store, &p, req)
    );
    p
}

/// Scoped to the decision and pinned, so anything else selected is an obligation rather than a
/// unit that earned its place.
fn focus(store: &Store, d: Uid, support: EdgeSet) -> PackRequest {
    let mut req = PackRequest::budget(10_000)
        .focusing([d])
        .resting_on(support);
    req.scope = [d].into();
    let _ = store;
    req
}

#[test]
fn without_a_support_set_a_conditioned_decision_packs_alone() {
    let (store, p, d, _) = corpus();
    let out = packed(&store, &focus(&store, d, EdgeSet::of([])));
    assert!(out.selection.contains_key(&d));
    assert!(!out.selection.contains_key(&p), "C1–C7 as they were");
}

#[test]
fn with_premises_the_prerequisite_travels_and_says_why() {
    let (store, p, d, _) = corpus();
    let req = focus(&store, d, EdgeSet::premises());
    let out = packed(&store, &req);
    assert_eq!(out.selection.get(&p), Some(&Lod::L0), "{:?}", out.why);
    assert_eq!(out.why.get(&p), Some(&Reason::SupportOf(d)));
    assert_eq!(out.why[&p].constraint(), "C8");
}

/// The obligation binds at L1+, as C1 and C2 do, and `verify` reports a selection that breaks it.
#[test]
fn a_selection_missing_what_it_rests_on_is_a_c8_violation() {
    let (store, p, d, _) = corpus();
    let req = focus(&store, d, EdgeSet::premises());
    let mut selection = smysl_pack::Selection::new();
    selection.insert(d, Lod::L1);
    let v = smysl_pack::violations(&store, &selection, 0, &{
        let mut c = smysl_pack::Constraints::default();
        c.budget = u64::MAX;
        c.support = EdgeSet::premises();
        c
    });
    assert_eq!(v.len(), 1, "{v:?}");
    assert_eq!(v[0].constraint(), "C8");
    assert!(v[0].to_string().contains(&p.to_string()), "{}", v[0]);
    let _ = req;
}

/// C8 binds at L1+, as C1 and C2 do: a gist alone rests on nothing, and a budget too small for
/// the pair refuses rather than presenting a decision whose prerequisite could not come.
#[test]
fn c8_binds_at_l1_and_an_unaffordable_floor_refuses() {
    let (store, p, d, _) = corpus();
    let e = smysl_pack::Estimator::default();
    let decision_l1 = e.unit(&store.get(&d).unwrap().core, Lod::L1);

    // At L0 the decision travels alone, as it does with grounds.
    let mut low = PackRequest::budget(10_000).resting_on(EdgeSet::premises());
    low.scope = [d].into();
    low.max_lod = Some(Lod::L0);
    let out = packed(&store, &low);
    assert_eq!(out.selection.get(&d), Some(&Lod::L0));
    assert!(!out.selection.contains_key(&p));

    // Pinned at L1 with room for the decision alone, the floor cannot be met.
    let req = PackRequest::budget(decision_l1 + 1)
        .focusing([d])
        .resting_on(EdgeSet::premises());
    match pack(&store, &salience(&store, &SalienceRequest::default()), &req) {
        Err(smysl_core::PackError::Infeasible { budget, required }) => {
            assert!(required > budget, "{required} against {budget}");
        }
        other => panic!("{other:?}"),
    }
}
