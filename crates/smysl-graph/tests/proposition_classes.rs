//! TX-P2 step 5's exit: the `strict` classes of a fixture, against the Python reference.
//!
//! A-12.4 is normative because two implementations that disagree about how many classes a store
//! holds disagree about how many propositions it holds, which is the number every corpus measure
//! is divided by. So the exit is not "the algorithm has tests" — it is **two implementations
//! agreeing**, and the only way to have that is for each to check the other's work.
//!
//! The split is deliberate:
//!
//! - **This file writes the input** (`fixtures/proposition/store.cbor`) and never the
//!   expectation. `SMYSL_BLESS=1` rewrites the store.
//! - **`scripts/gen-proposition-classes.py` writes the expectation**
//!   (`fixtures/proposition/classes.json`), computed by an independent reading of A-12.4 over
//!   the Python port's decoder. `SMYSL_BLESS=1` there rewrites it.
//! - Each side then compares against the other's file, and `python/tests/test_proposition.py`
//!   asserts the same expectation from the Python side.
//!
//! A single file that generated both halves would be one implementation agreeing with itself,
//! which is the failure `scripts/verify-spec-tables.py`'s own header is about: three readers
//! "agreed" because all three had read the same fixture.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::ids::{AgentId, Uid};
use smysl_core::types::provenance::{Attestation, Hlc, Op, Rung};
use smysl_core::types::RelKind;
use smysl_core::{
    canonical_uid, from_cbor_seq, to_cbor_seq, KernelType, Record, Relation, Status,
    UnitCoreBuilder, Withdrawal,
};
use smysl_graph::proposition::{classes, Policy, SAME_AS};
use smysl_graph::Store;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/proposition")
}

fn agent(name: &str) -> AgentId {
    AgentId::new(name).expect("an agent id")
}

fn hlc(ms: u64, who: &str) -> Hlc {
    Hlc::new(ms, 0, agent(who))
}

/// Nine units, sorted by uid.
///
/// Sorted because A-12.4 works in uid order, so the fixture's *shape* has to be stated in that
/// order or the expectation is about something else. The gists are numbered for the reader;
/// which number lands at which position is a hash's business.
fn units() -> (Vec<Record>, Vec<Uid>) {
    let mut pairs: Vec<(Record, Uid)> = (0..9)
        .map(|i| {
            let u = UnitCoreBuilder::new(
                KernelType::Claim,
                format!("proposition {i}: a sentence somebody wrote down"),
                Status::Speculative,
            )
            .build()
            .expect("a unit");
            let uid = canonical_uid(&u);
            (Record::Unit(u), uid)
        })
        .collect();
    pairs.sort_by_key(|(_, uid)| *uid);
    (
        pairs.iter().map(|(r, _)| r.clone()).collect(),
        pairs.iter().map(|(_, u)| *u).collect(),
    )
}

fn same_as(from: Uid, to: Uid) -> Relation {
    Relation::new(RelKind::Extension(SAME_AS.to_string()), from, to)
}

/// The fixture: one store holding every shape the three policies have to disagree about.
///
/// | units (in uid order) | shape | why it is here |
/// |---|---|---|
/// | 0, 1, 2 | a triangle | one class under every policy; the easy case, and the one that fails first if the uid ordering is wrong |
/// | 3, 4, 5 | a path `3—4—5` | **two** strict classes and one component of diameter 2 |
/// | 6, 7 | one edge, attested by two agents | survives `attested:2` |
/// | 7, 8 | one edge, attested by one agent | does not |
/// | 0, 3 | an edge, withdrawn | would otherwise merge the triangle and the path |
/// | 2, absent | an edge to a unit the store does not hold | not live |
/// | 8, 8 | a self-edge | true, and not a class |
/// | 0, 1 | an `elaborates` edge | not a sameness claim |
fn fixture() -> Vec<Record> {
    let (mut records, u) = units();
    let (absent, _) = {
        let unit = UnitCoreBuilder::new(
            KernelType::Claim,
            "a proposition this store never received",
            Status::Speculative,
        )
        .build()
        .expect("a unit");
        let uid = canonical_uid(&unit);
        (uid, unit)
    };

    for (a, b) in [(0, 1), (1, 2), (0, 2), (3, 4), (4, 5)] {
        records.push(Record::Relation(same_as(u[a], u[b])));
    }

    // Two agents on 6—7, one on 7—8: the pair that makes `attested:2` a different answer.
    let two_agents = same_as(u[6], u[7]);
    let rid = two_agents.uid();
    records.push(Record::Relation(two_agents));
    for (who, rung) in [
        ("tool:smysl-same-as-lexical", Rung::Computed),
        ("human:vu", Rung::Document),
    ] {
        records.push(Record::Attestation(Attestation::new(
            rid,
            agent(who),
            Op::Attested,
            rung,
            hlc(1_726_500_000_000, who),
        )));
    }
    let one_agent = same_as(u[7], u[8]);
    let rid = one_agent.uid();
    records.push(Record::Relation(one_agent));
    records.push(Record::Attestation(Attestation::new(
        rid,
        agent("tool:smysl-same-as-lexical"),
        Op::Attested,
        Rung::Computed,
        hlc(1_726_500_000_001, "tool:smysl-same-as-lexical"),
    )));

    // Withdrawn: without the withdrawal this edge merges the triangle and the path.
    let withdrawn = same_as(u[0], u[3]);
    let rid = withdrawn.uid();
    records.push(Record::Relation(withdrawn));
    records.push(Record::Withdrawal(Withdrawal::new(
        rid,
        agent("human:vu"),
        hlc(1_726_500_000_002, "human:vu"),
    )));

    records.push(Record::Relation(same_as(u[2], absent)));
    records.push(Record::Relation(same_as(u[8], u[8])));
    records.push(Record::Relation(Relation::new(
        RelKind::Elaborates,
        u[0],
        u[1],
    )));
    records
}

fn store_bytes() -> Vec<u8> {
    to_cbor_seq(&fixture())
}

/// The committed store, decoded.
fn committed() -> Store {
    let path = dir().join("store.cbor");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!("fixtures/proposition/store.cbor: {e} (SMYSL_BLESS=1 writes it)")
    });
    let (records, consumed) = from_cbor_seq(&bytes).expect("the fixture decodes");
    assert_eq!(consumed, bytes.len(), "the fixture decoded in full");
    Store::from_records(records)
}

/// `[["b3:…", "b3:…"], …]` — a policy's classes as the expectation file writes them.
fn rendered(policy: Policy, store: &Store) -> Vec<Vec<String>> {
    classes(store, policy, None)
        .into_iter()
        .map(|c| c.members.iter().map(Uid::canonical).collect())
        .collect()
}

/// The expectation Python wrote, as `{policy: [[uid, …], …]}`.
fn expectation() -> std::collections::BTreeMap<String, Vec<Vec<String>>> {
    let path = dir().join("classes.json");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "fixtures/proposition/classes.json: {e}\n  \
             it is written by the **Python** reference: \
             SMYSL_BLESS=1 python3 scripts/gen-proposition-classes.py"
        )
    });
    // A hand-rolled read of the one shape this file has, so the test needs no serde: the file is
    // written by `json.dump(..., indent=2)`, so every uid is on a line of its own.
    let mut out: std::collections::BTreeMap<String, Vec<Vec<String>>> = Default::default();
    let mut policy = String::new();
    let mut class: Vec<String> = Vec::new();
    let mut open = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_suffix(": [") {
            policy = name.trim_matches('"').to_string();
            out.entry(policy.clone()).or_default();
        } else if t == "[" {
            class = Vec::new();
            open = true;
        } else if t.starts_with(']') && open {
            out.entry(policy.clone()).or_default().push(class.clone());
            open = false;
        } else if t.starts_with('"') && open {
            class.push(t.trim_matches(|c| c == '"' || c == ',').to_string());
        }
    }
    out
}

/// The store the Python reference reads is the store this build produces.
#[test]
fn the_fixture_store_matches_this_build() {
    let want = store_bytes();
    let path = dir().join("store.cbor");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(dir()).expect("the fixture directory");
        std::fs::write(&path, &want).expect("writing the store");
        return;
    }
    let have = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(
        have, want,
        "fixtures/proposition/store.cbor no longer matches what this build produces; \
         `SMYSL_BLESS=1 cargo test -p smysl-graph --test proposition_classes` rewrites it, and \
         the expectation then has to be rewritten by the Python reference"
    );
}

/// **The exit.** Every policy's classes are the ones the Python reference computed.
#[test]
fn the_classes_are_the_ones_the_python_reference_computed() {
    let store = committed();
    let expected = expectation();
    assert_eq!(
        expected.len(),
        3,
        "the expectation covers strict, component and attested:2"
    );
    for (name, policy) in [
        ("strict", Policy::Strict),
        ("component", Policy::Component),
        ("attested:2", Policy::Attested(2)),
    ] {
        let want = expected
            .get(name)
            .unwrap_or_else(|| panic!("no `{name}` in the expectation"));
        assert_eq!(&rendered(policy, &store), want, "policy {name}");
    }
}

/// And they do not depend on the order the records arrived in.
///
/// The second half of the exit. Asserted over the fixture rather than over a generated store,
/// because this is the one whose answer is pinned: if the partition moved with the order, the
/// pinned answer would be an accident of how the fixture happens to be written.
#[test]
fn the_classes_do_not_depend_on_the_order_of_the_log() {
    let mut records = fixture();
    let forward = Store::from_records(records.clone());
    records.reverse();
    let backward = Store::from_records(records);

    for policy in [Policy::Strict, Policy::Component, Policy::Attested(2)] {
        assert_eq!(
            rendered(policy, &forward),
            rendered(policy, &backward),
            "{policy:?} depends on the log order"
        );
    }
}

/// The fixture holds what its table says it holds.
///
/// A fixture whose shape has drifted from its documentation is a fixture whose expectation
/// nobody can read. Checked by counting the classes each policy finds, which is the one summary
/// that moves if any of the eight rows changes.
#[test]
fn the_fixture_still_has_the_shape_its_table_describes() {
    let store = committed();
    assert_eq!(store.units().count(), 9, "nine units, one absent endpoint");

    // Five, and the arithmetic is worth writing out because the first version of this line
    // said four: the triangle is one class, the path `3—4—5` is **two** ({3,4} and {5}), and
    // `6—7—8` is a path as well, so it is two more. Both implementations said five and the
    // comment said four — which is what this test is for, since the two sides agreeing is no
    // use if nobody can say what they agreed on.
    let strict = classes(&store, Policy::Strict, None);
    assert_eq!(
        strict.len(),
        5,
        "the triangle, {{3,4}}, {{5}}, {{6,7}}, {{8}}"
    );
    assert_eq!(
        strict.iter().filter(|c| c.len() == 3).count(),
        1,
        "the triangle"
    );
    assert_eq!(
        strict.iter().filter(|c| c.len() == 2).count(),
        2,
        "each path's head"
    );
    assert_eq!(
        strict.iter().filter(|c| c.len() == 1).count(),
        2,
        "each path's loose end"
    );

    let component = classes(&store, Policy::Component, None);
    assert_eq!(component.len(), 3, "triangle, path, and the 6—7—8 chain");
    assert!(
        component.iter().all(|c| c.members.len() == 3),
        "three components of three: {component:?}"
    );
    let diameters: BTreeSet<Option<u32>> = component.iter().map(|c| c.diameter).collect();
    assert!(diameters.contains(&Some(1)), "the triangle is a clique");
    assert!(diameters.contains(&Some(2)), "the path is not");

    let attested = classes(&store, Policy::Attested(2), None);
    assert_eq!(attested.len(), 1, "one edge carries two agents");
    assert_eq!(attested[0].len(), 2);
}
