//! TX-P9 N3's exit (SMYSL-2.6 §6): A9 recovers planted communities exactly on a two-clique
//! graph, with and without a bridge between the cliques; and its centrality, betweenness and
//! event chains equal what a planted graph makes them by hand.
//!
//! Every graph here is built from units that mention entities, as an extraction writes them,
//! so the entity graph under test is the one A9 derives and not one handed to it.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schemas::text::{ENTITY, MENTIONS, PRECEDES};
use smysl_core::types::RelKind;
use smysl_core::{
    canonical_uid, KernelType, Record, Relation, SchemaId, Status, Uid, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_measure::graph::entity::{self, betweenness, centrality, communities, entity_graph};
use smysl_measure::num::q20;
use smysl_measure::{run, Input, MeasureResult, Num, Outcome, Param, Recipe, Resolved, Value};

const SCOPE: &str = "from texts";

struct Planted {
    records: Vec<Record>,
    units: Vec<Uid>,
}

impl Planted {
    fn new() -> Planted {
        Planted {
            records: Vec::new(),
            units: Vec::new(),
        }
    }

    fn entity(&mut self, name: &str) -> Uid {
        self.entity_in(name, None)
    }

    /// An entity in another language is another unit of the same name, as two translations'
    /// "Ruth" are.
    fn entity_in(&mut self, name: &str, lang: Option<&str>) -> Uid {
        let mut b = UnitCoreBuilder::new(
            SchemaId::Extension(ENTITY.to_string()),
            name,
            Status::Speculative,
        );
        if let Some(l) = lang {
            b = b.lang(smysl_core::ids::LangTag::new(l).unwrap());
        }
        let u = b.build().unwrap();
        let uid = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        uid
    }

    /// A claim of the scope that mentions each of `entities`; `reversed` writes the edges from
    /// the entity, which the schema allows.
    fn mentioning(&mut self, entities: &[Uid], reversed: bool) -> Uid {
        let n = self.units.len();
        let u = UnitCoreBuilder::new(KernelType::Claim, format!("claim {n}"), Status::Speculative)
            .build()
            .unwrap();
        let uid = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        for e in entities {
            let (from, to) = if reversed { (*e, uid) } else { (uid, *e) };
            self.records.push(Record::Relation(Relation::new(
                RelKind::Extension(MENTIONS.to_string()),
                from,
                to,
            )));
        }
        self.units.push(uid);
        uid
    }

    fn edge(&mut self, kind: RelKind, from: Uid, to: Uid) {
        self.records
            .push(Record::Relation(Relation::new(kind, from, to)));
    }

    fn store(&self) -> Store {
        Store::from_records(self.records.clone())
    }
}

fn a9(recipe: &Recipe, store: &Store, uids: &[Uid]) -> MeasureResult {
    let scope = Resolved::of_uids(SCOPE, uids.iter().copied().collect());
    run(&["A9"], recipe, &Input::new(store, &scope)).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("A9 did not run: {other:?}"),
    }
}

/// Two cliques of five entities: every pair in a clique mentioned together by one unit, and a
/// bridge — one unit mentioning an entity of each — when asked for.
fn two_cliques(bridge: bool) -> (Planted, [Vec<Uid>; 2]) {
    let mut p = Planted::new();
    let cliques = [0, 1].map(|c| {
        (0..5)
            .map(|i| p.entity(&format!("entity {c}.{i}")))
            .collect::<Vec<_>>()
    });
    for (c, clique) in cliques.iter().enumerate() {
        for i in 0..clique.len() {
            for j in i + 1..clique.len() {
                p.mentioning(&[clique[i], clique[j]], (i + j + c) % 2 == 1);
            }
        }
    }
    if bridge {
        p.mentioning(&[cliques[0][4], cliques[1][4]], false);
    }
    (p, cliques)
}

/// §6's exit: the planted communities, exactly — each clique one community named by its
/// smallest uid, with and without a bridge, the rounds settled, and the counts A9 reports.
#[test]
fn planted_communities_are_recovered_exactly_on_two_cliques() {
    for bridge in [false, true] {
        let (p, cliques) = two_cliques(bridge);
        let store = p.store();
        let g = entity_graph(&store, &p.units);
        assert_eq!(g.edges.len(), 10);
        let (label, settled) = communities(&g);
        assert!(settled, "bridge {bridge}: the rounds settled");
        for clique in &cliques {
            let min = *clique.iter().min().unwrap();
            for e in clique {
                assert_eq!(
                    label[e], min,
                    "bridge {bridge}: {e} is in its clique's community"
                );
            }
        }
        let r = a9(&Recipe::new("", 0, SCOPE), &store, &p.units);
        let v = value(&r);
        assert_eq!(v.values["communities"], Num::Count(2));
        assert_eq!(v.series["sizes"], [Num::Count(5), Num::Count(5)]);
        assert_eq!(v.values["community:largest"], Num::Ratio(5, 10));
        assert_eq!(v.n["mentions"], 2 * 20 + if bridge { 2 } else { 0 });
        assert_eq!(v.n["pairs"], 20 + u64::from(bridge));
        assert!(v.flags.is_empty(), "{:?}", v.flags);
        // Each entity is mentioned by the four units pairing it with the rest of its clique,
        // and a bridge end by one more.
        assert_eq!(
            v.values["degree:entity 0.4"],
            Num::Count(4 + u64::from(bridge))
        );
    }
}

/// The plain synchronous rule swaps the two labels of a single edge for ever; counting an
/// entity's own label settles it in one round, on the smaller.
#[test]
fn one_edge_settles_rather_than_oscillating() {
    let mut p = Planted::new();
    let (a, b) = (p.entity("a"), p.entity("b"));
    p.mentioning(&[a, b], false);
    let g = entity_graph(&p.store(), &p.units);
    let (label, settled) = communities(&g);
    assert!(settled);
    assert_eq!(label[&a], a.min(b));
    assert_eq!(label[&b], a.min(b));
    let c = centrality(&g);
    assert_eq!(
        (c[&a], c[&b]),
        (1.0, 1.0),
        "a bipartite graph does not oscillate"
    );
}

/// A star of four leaves: over `A + I` the centre's eigenvector entry is √4 = 2 times each
/// leaf's, so scaled to a maximum of 1 the leaves are ½. The iteration's error shrinks by ⅓ a
/// step, so after 100 it is far below the payload's 2⁻²⁰ and the stored numbers are exact.
#[test]
fn a_stars_centrality_is_its_eigenvector() {
    let mut p = Planted::new();
    let centre = p.entity("centre");
    let leaves: Vec<Uid> = (0..4).map(|i| p.entity(&format!("leaf {i}"))).collect();
    for l in &leaves {
        p.mentioning(&[centre, *l], false);
    }
    let store = p.store();
    let g = entity_graph(&store, &p.units);
    let c = centrality(&g);
    assert_eq!(q20(c[&centre]), Some(1 << 20));
    for l in &leaves {
        assert_eq!(q20(c[l]), Some(1 << 19), "{l}");
    }
    let v = value(&a9(&Recipe::new("", 0, SCOPE), &store, &p.units)).clone();
    assert_eq!(v.values["centrality:centre"], Num::Fixed(1 << 20));
    assert_eq!(v.values["degree:centre"], Num::Count(4));
    assert_eq!(v.values["communities"], Num::Count(1));
}

/// A path of four entities a–b–c–d: two of the three pairs not adjacent to b pass through it,
/// (a, c) and (a, d), so b's betweenness is 2 of the 3 pairs it could lie between; c's the
/// same; the ends 0. Reported only when the recipe asks.
#[test]
fn betweenness_on_a_path_and_only_on_request() {
    let mut p = Planted::new();
    let e: Vec<Uid> = ["a", "b", "c", "d"].iter().map(|n| p.entity(n)).collect();
    for w in e.windows(2) {
        p.mentioning(w, false);
    }
    let store = p.store();
    let b = betweenness(&entity_graph(&store, &p.units));
    assert_eq!((b[&e[0]], b[&e[3]]), (0.0, 0.0));
    assert_eq!(q20(b[&e[1]]), q20(2.0 / 3.0));
    assert_eq!(q20(b[&e[2]]), q20(2.0 / 3.0));

    let mut r = Recipe::new("", 0, SCOPE);
    let v = value(&a9(&r, &store, &p.units)).clone();
    assert!(!v.values.keys().any(|k| k.starts_with("betweenness:")));
    r.params.insert("betweenness".into(), Param::Bool(true));
    let v = value(&a9(&r, &store, &p.units)).clone();
    assert_eq!(
        v.values["betweenness:b"],
        Num::Fixed(q20(2.0 / 3.0).unwrap())
    );
    assert_eq!(v.values["betweenness:a"], Num::Fixed(0));
}

/// Event chains: a → b → c → d by `sequences` with a shortcut a → c, e → f by
/// `x.text/precedes`, and g ⇄ h a cycle. The longest chain is four units; eight units are on
/// an edge and the two on the cycle are in no chain. An edge leaving the scope is not read.
#[test]
fn event_chains_are_longest_paths_and_a_cycle_is_counted() {
    let mut p = Planted::new();
    let u: Vec<Uid> = (0..9).map(|_| p.mentioning(&[], false)).collect();
    let precedes = RelKind::Extension(PRECEDES.to_string());
    for (a, b) in [(0, 1), (1, 2), (2, 3), (0, 2)] {
        p.edge(RelKind::Sequences, u[a], u[b]);
    }
    p.edge(precedes.clone(), u[4], u[5]);
    p.edge(precedes.clone(), u[6], u[7]);
    p.edge(precedes, u[7], u[6]);
    p.edge(RelKind::Sequences, u[3], u[8]);
    let store = p.store();
    let scope: Vec<Uid> = u[..8].to_vec();
    assert_eq!(entity::chains(&store, &scope), (4, 8, 2));
    let v = value(&a9(&Recipe::new("", 0, SCOPE), &store, &scope)).clone();
    assert_eq!(v.values["chain:max"], Num::Count(4));
    assert_eq!((v.n["chained"], v.n["cyclic"]), (8, 2));
    assert_eq!(
        v.values["communities"],
        Num::Count(0),
        "no entity is mentioned"
    );
    assert!(!v.values.contains_key("community:largest"));
    assert_eq!(
        entity::chains(&store, &u),
        (5, 9, 2),
        "with the ninth in scope"
    );
}

/// Only the scope's mentions count, entities with one name are told apart in their keys, and a
/// recipe A9 cannot read is refused.
#[test]
fn a9_reads_its_scope_and_refuses_what_it_cannot_read() {
    let mut p = Planted::new();
    let (x, y, z) = (
        p.entity("Ruth"),
        p.entity_in("Ruth", Some("de")),
        p.entity("Boaz"),
    );
    let a = p.mentioning(&[x, z], false);
    let b = p.mentioning(&[y, z], false);
    let store = p.store();
    let only_a = a9(&Recipe::new("", 0, SCOPE), &store, &[a]);
    assert_eq!(value(&only_a).n["entities"], 2);
    let both = a9(&Recipe::new("", 0, SCOPE), &store, &[a, b]);
    let keys: BTreeSet<&String> = value(&both)
        .values
        .keys()
        .filter(|k| k.starts_with("degree:Ruth"))
        .collect();
    assert_eq!(keys.len(), 2, "two Ruths, two keys: {keys:?}");
    assert_eq!(value(&both).values["degree:Boaz"], Num::Count(2));

    let refused = |r: &Recipe| match a9(r, &store, &[a, b]).outcome {
        Outcome::NotRun { code: Some(c), .. } => c,
        other => panic!("expected a refusal: {other:?}"),
    };
    let mut r = Recipe::new("", 0, SCOPE);
    r.params.insert("top:k".into(), Param::Int(-1));
    assert_eq!(refused(&r), smysl_core::diag::Code::E489);
    let mut r = Recipe::new("", 0, SCOPE);
    r.params
        .insert("betweenness".into(), Param::Text("yes".into()));
    assert_eq!(refused(&r), smysl_core::diag::Code::E489);
    let mut r = Recipe::new("", 0, SCOPE);
    r.params.insert("top:k".into(), Param::Int(1));
    let v = value(&a9(&r, &store, &[a, b])).clone();
    let degrees: BTreeMap<&String, &Num> = v
        .values
        .iter()
        .filter(|(k, _)| k.starts_with("degree:"))
        .collect();
    assert_eq!(degrees.len(), 1, "{degrees:?}");
}
