//! TX-P1 step 4's exit: a store's view of a library, and what a manifest costs it.
//!
//! Four claims, each of which a corpus depends on:
//!
//! 1. a manifest is absorbed, indexed by alias, and idempotent on replay;
//! 2. a head is a manifest nothing supersedes, two heads are reported rather than resolved,
//!    and a truncated chain has no head rather than an invented one;
//! 3. a part text or a reading offered to a log is refused with `SMY-E452`, whole batch and
//!    before any byte is written (OQ-39);
//! 4. a manifest-only append does **not** rebuild the adjacency, which is what makes
//!    `text add` cost `O(batch)` instead of the store.

use smysl_core::diag::Code;
use smysl_core::error::LibError;
use smysl_core::ids::{LangTag, Rdid, Tid};
use smysl_core::types::SourceKind;
use smysl_core::types::{Carry, Manifest, PartEntry, PartReading, PartText};
use smysl_core::{Error, KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::Store;

fn claim(gist: &str) -> smysl_core::UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .expect("a unit")
}

fn tid(byte: u8) -> Tid {
    Tid::from_normalised_bytes(&[byte])
}

fn manifest(alias: &str, part: u8) -> Manifest {
    let mut m = Manifest::new(
        alias,
        LangTag::new("en").expect("a tag"),
        "txt/1",
        "public-domain",
        "smysl/parts/1 level=line min=65536 max=4194304",
    );
    m.carry = Carry::Text;
    m.parts.push(PartEntry::new(
        tid(part),
        128,
        [part; 32],
        Rdid::from_bytes([part; 32]),
    ));
    m
}

/// A manifest lands, is found by mid and by alias, and a replay changes nothing.
#[test]
fn a_manifest_is_absorbed_once_however_often_it_arrives() {
    let m = manifest("kjv1769", 1);
    let mut store = Store::new();
    let first = store
        .append(&[Record::Manifest(m.clone())])
        .expect("appended");
    assert_eq!(first.added, 1);

    assert_eq!(store.manifest_count(), 1);
    assert_eq!(
        store.manifest(&m.mid()).map(|k| k.alias.as_str()),
        Some("kjv1769")
    );
    assert_eq!(store.aliases().collect::<Vec<_>>(), vec!["kjv1769"]);
    assert_eq!(store.manifests_of("kjv1769"), vec![m.mid()]);

    let again = store
        .append(&[Record::Manifest(m.clone())])
        .expect("appended");
    assert_eq!(again.added, 0, "the same manifest is not a second manifest");
    assert_eq!(again.duplicates, 1);
    assert_eq!(store.manifest_count(), 1);
}

/// A chain of three leaves one head, and the two it superseded are not heads.
#[test]
fn a_chain_leaves_one_head() {
    let first = manifest("kjv1769", 1);
    let mut second = manifest("kjv1769", 2);
    second.supersedes = Some(first.mid());
    let mut third = manifest("kjv1769", 3);
    third.supersedes = Some(second.mid());

    let mut store = Store::new();
    store
        .append(&[
            Record::Manifest(first.clone()),
            Record::Manifest(second.clone()),
            Record::Manifest(third.clone()),
        ])
        .expect("appended");

    assert_eq!(store.heads("kjv1769"), vec![third.mid()]);
    assert!(store.is_superseded(&first.mid()));
    assert!(store.is_superseded(&second.mid()));
    assert!(!store.is_superseded(&third.mid()));
}

/// Order does not matter: the heads are the same whichever way the chain arrives.
///
/// The property that makes merge safe. Two peers receiving a three-manifest chain in opposite
/// orders must agree about which manifest is current, or a corpus means different things on
/// two machines.
#[test]
fn the_heads_do_not_depend_on_arrival_order() {
    let first = manifest("kjv1769", 1);
    let mut second = manifest("kjv1769", 2);
    second.supersedes = Some(first.mid());
    let mut third = manifest("kjv1769", 3);
    third.supersedes = Some(second.mid());
    let records = [
        Record::Manifest(first),
        Record::Manifest(second),
        Record::Manifest(third.clone()),
    ];

    let mut forwards = Store::new();
    forwards.append(&records).expect("appended");
    let mut backwards = Store::new();
    let mut reversed = records.to_vec();
    reversed.reverse();
    backwards.append(&reversed).expect("appended");
    // One at a time, in the awkward order: last, first, middle.
    let mut piecemeal = Store::new();
    for i in [2usize, 0, 1] {
        piecemeal.append(&[records[i].clone()]).expect("appended");
    }

    assert_eq!(forwards.heads("kjv1769"), vec![third.mid()]);
    assert_eq!(backwards.heads("kjv1769"), vec![third.mid()]);
    assert_eq!(piecemeal.heads("kjv1769"), vec![third.mid()]);
}

/// Two manifests superseding the same one is a fork, and both are heads.
///
/// Reported, not resolved: `check` raises `SMY-W418`. Picking a winner here would make the
/// fork unreportable, which is the same argument that keeps a contention a record.
#[test]
fn a_fork_leaves_two_heads() {
    let root = manifest("kjv1769", 1);
    let mut left = manifest("kjv1769", 2);
    left.supersedes = Some(root.mid());
    let mut right = manifest("kjv1769", 3);
    right.supersedes = Some(root.mid());

    let mut store = Store::new();
    store
        .append(&[
            Record::Manifest(root.clone()),
            Record::Manifest(left.clone()),
            Record::Manifest(right.clone()),
        ])
        .expect("appended");

    let mut heads = store.heads("kjv1769");
    heads.sort();
    let mut expected = vec![left.mid(), right.mid()];
    expected.sort();
    assert_eq!(heads, expected);
}

/// A manifest whose predecessor is absent is still a head; a chain whose head is absent has none.
#[test]
fn a_truncated_chain_has_no_invented_head() {
    let absent = manifest("kjv1769", 9);
    let mut only = manifest("kjv1769", 1);
    only.supersedes = Some(absent.mid());

    let mut store = Store::new();
    store
        .append(&[Record::Manifest(only.clone())])
        .expect("appended");
    assert_eq!(
        store.heads("kjv1769"),
        vec![only.mid()],
        "the manifest held is the head; the one it supersedes was never here"
    );

    // The other way round: the store holds the superseded one and not its successor.
    let mut later = manifest("kjv1769", 2);
    later.supersedes = Some(absent.mid());
    let mut partial = Store::new();
    partial
        .append(&[Record::Manifest(absent.clone()), Record::Manifest(later)])
        .expect("appended");
    assert!(
        !partial.heads("kjv1769").contains(&absent.mid()),
        "a superseded manifest is not a head even when its successor's own successor is absent"
    );

    assert!(store.heads("unknown-alias").is_empty());
}

/// Aliases do not leak into each other.
#[test]
fn each_alias_has_its_own_heads() {
    let kjv = manifest("kjv1769", 1);
    let luther = manifest("luther1912", 2);
    let mut store = Store::new();
    store
        .append(&[
            Record::Manifest(kjv.clone()),
            Record::Manifest(luther.clone()),
        ])
        .expect("appended");
    assert_eq!(store.heads("kjv1769"), vec![kjv.mid()]);
    assert_eq!(store.heads("luther1912"), vec![luther.mid()]);
    assert_eq!(
        store.aliases().collect::<Vec<_>>(),
        vec!["kjv1769", "luther1912"]
    );
}

/// A part text offered to a log is refused with `SMY-E452`, and so is a reading.
#[test]
fn a_log_refuses_text_with_e452() {
    for record in [
        Record::PartText(PartText::new(b"In the beginning.".to_vec())),
        Record::PartReading(PartReading::new(tid(1), "txt/1", vec![0x80])),
    ] {
        let mut store = Store::new();
        let err = store.append(&[record]).expect_err("refused");
        match err {
            Error::Lib(lib @ LibError::TextInLog { .. }) => {
                assert_eq!(lib.code(), Some(Code::E452));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(store.len(), 0, "nothing was written");
    }
}

/// A time contention offered to a log is refused: rule E derives those, nobody writes them.
///
/// A-8.2 says an implementation MUST NOT write a record 6 with detection kind 4 or 5, and the
/// cheapest place to keep a MUST NOT is the door — the same decision as `SMY-E452` above, and
/// for a related reason. A store that accepted one would hold a contention an implementation
/// predating the kinds can preserve and cannot act on, and the whole point of opening the
/// enumeration in 1.9 was that an unknown code should cost a reader nothing.
///
/// No diagnostic code, deliberately: refusing it makes the violation unrepresentable rather
/// than reportable. A **resolution** naming the derived id is a different record and is
/// accepted as it always was — that is the one a reviewer writes.
#[test]
fn a_log_refuses_a_contention_rule_e_derives() {
    use smysl_core::types::{Contention, Detected, DetectionKind};
    use smysl_core::{AgentId, ContentionId, Hlc, Uid};
    let agent = AgentId::new("human:vu").expect("an agent");
    let uid = |n: u8| Uid::from_bytes([n; 32]);
    for kind in DetectionKind::DERIVED_ONLY.iter().copied() {
        let c = Contention::new(
            ContentionId::derive(kind, &uid(1), &[uid(2)]),
            uid(1),
            vec![uid(2)],
            Detected::new(kind, Hlc::new(1, 0, agent.clone())),
        );
        let mut store = Store::new();
        let err = store.append(&[Record::Contention(c)]).expect_err("refused");
        match err {
            Error::Lib(lib @ LibError::DerivedContention { .. }) => {
                assert_eq!(lib.code(), None, "the refusal is not a diagnostic");
                assert!(lib.to_string().contains("rule E"), "{lib}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(store.len(), 0, "nothing was written");
    }
    // And the four kinds merge does write are accepted, which is what makes the refusal narrow
    // rather than a ban on record 6.
    for kind in [
        DetectionKind::SupersessionFork,
        DetectionKind::LiveRebuttal,
        DetectionKind::LabelCollision,
        DetectionKind::CommitmentFork,
    ] {
        let c = Contention::new(
            ContentionId::derive(kind, &uid(1), &[uid(2)]),
            uid(1),
            vec![uid(2)],
            Detected::new(kind, Hlc::new(1, 0, agent.clone())),
        );
        let mut store = Store::new();
        store
            .append(&[Record::Contention(c)])
            .expect("a kind merge writes");
        assert_eq!(store.len(), 1);
    }
}

/// The whole batch is refused, not the records before the offending one.
///
/// A half-appended delivery would leave a store the sender cannot reason about: the manifest
/// landed, the text did not, and the report said nothing about which. Refusing the batch keeps
/// `append` all-or-nothing for this case, as it already is for an I/O failure.
#[test]
fn the_whole_batch_is_refused_not_the_prefix() {
    let m = manifest("kjv1769", 1);
    let mut store = Store::new();
    let err = store
        .append(&[
            Record::Manifest(m.clone()),
            Record::PartText(PartText::new(b"text".to_vec())),
        ])
        .expect_err("refused");
    assert!(
        matches!(err, Error::Lib(LibError::TextInLog { .. })),
        "{err:?}"
    );
    assert_eq!(
        store.manifest_count(),
        0,
        "the manifest before it did not land"
    );
    assert_eq!(store.len(), 0);
}

/// A manifest-only append leaves the adjacency alone.
///
/// This is the step's cost claim. `absorb` used to rebuild the adjacency on every batch
/// whatever arrived, so appending one manifest charged the cost of the whole store — 1336 µs a
/// record at 1.8's single-append figure, growing with the store. The adjacency is built from
/// units, relations and withdrawals; a manifest cannot move an edge.
///
/// Asserted through the public surface, by taking a traversal before and after: if the
/// adjacency had been rebuilt it would still be *equal*, so what this really pins is the
/// second half — that the graph is unchanged — while the timing harness (`make bench`) pins
/// the cost. Equality after a no-op rebuild is cheap to assert and would not catch the
/// regression; the cost is what the `moves_edges` flag is for, and `cargo bench`'s
/// manifest-only row is where it shows.
#[test]
fn a_manifest_only_append_does_not_disturb_the_graph() {
    let unit = claim("the engine is ready");
    let mut store = Store::new();
    store
        .append(&[Record::Unit(unit.clone())])
        .expect("appended");
    let before: Vec<_> = store.units().map(|(uid, _)| *uid).collect();
    let edges_before = store.adjacency().len();

    store
        .append(&[Record::Manifest(manifest("kjv1769", 1))])
        .expect("appended");

    let after: Vec<_> = store.units().map(|(uid, _)| *uid).collect();
    assert_eq!(before, after, "the units are untouched");
    assert_eq!(
        store.adjacency().len(),
        edges_before,
        "the adjacency is untouched"
    );
    assert_eq!(store.manifest_count(), 1, "and the manifest did land");
}

/// A unit whose source names a part is indexed by that part, exactly.
#[test]
fn units_are_indexed_by_the_part_they_came_from() {
    use smysl_core::types::SourceRef;

    let part = tid(1);
    let other = tid(2);
    let from_part = UnitCoreBuilder::new(KernelType::Prose, "in the beginning", Status::Cited)
        .source(SourceRef::new(
            SourceKind::Doc,
            format!("{}#Gen.1.1", part.canonical()),
        ))
        .build()
        .expect("a unit");
    let from_elsewhere = UnitCoreBuilder::new(KernelType::Prose, "something else", Status::Cited)
        .source(SourceRef::new(SourceKind::Doc, other.canonical()))
        .build()
        .expect("a unit");
    let no_source = claim("no source at all");

    let mut store = Store::new();
    store
        .append(&[
            Record::Unit(from_part.clone()),
            Record::Unit(from_elsewhere.clone()),
            Record::Unit(no_source),
        ])
        .expect("appended");

    assert_eq!(
        store.units_with_tid(&part),
        vec![smysl_core::canonical_uid(&from_part)]
    );
    assert_eq!(
        store.units_with_tid(&other),
        vec![smysl_core::canonical_uid(&from_elsewhere)]
    );
    assert!(store.units_with_tid(&tid(3)).is_empty());

    // The exact index agrees with the prefix scan it replaces, which is public contract.
    let by_prefix = store.units_with_source_prefix(&format!("{}#", part.canonical()));
    assert_eq!(by_prefix, store.units_with_tid(&part));
}

/// Every part a manifest records is findable with the length it was recorded at.
///
/// What `check` needs for `SMY-E404` (a span past the part's length) without opening an object.
#[test]
fn parts_are_findable_with_their_recorded_lengths() {
    let m = manifest("kjv1769", 1);
    let mut store = Store::new();
    store.append(&[Record::Manifest(m)]).expect("appended");
    let parts = store.parts();
    assert_eq!(parts.len(), 1);
    assert_eq!(
        parts
            .get(&tid(1))
            .map(|lengths| lengths.iter().copied().collect::<Vec<_>>()),
        Some(vec![128])
    );
}

/// A manifest survives a round trip through the log.
#[test]
fn a_manifest_survives_reopening_the_store() {
    let dir = std::env::temp_dir().join(format!("smysl-library-store-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join("log.smyl");
    let m = manifest("kjv1769", 1);
    {
        let mut store = Store::open(&path).expect("opened");
        store
            .append(&[Record::Manifest(m.clone())])
            .expect("appended");
    }
    let reopened = Store::open(&path).expect("opened");
    assert_eq!(reopened.manifest_count(), 1);
    assert_eq!(reopened.heads("kjv1769"), vec![m.mid()]);
    std::fs::remove_dir_all(&dir).ok();
}

/// A manifest travels in a bundle with the units that came out of its text, and not otherwise.
///
/// Both halves matter. A recipient holding units whose source names a part of a manifest it
/// does not have cannot check a span against the part's length, or know whether the text may
/// be carried at all. And a bundle that carried *every* manifest would tell the recipient
/// which other texts the sender holds, which is a disclosure the sender did not choose.
#[test]
fn a_manifest_travels_with_the_units_that_came_from_it() {
    use smysl_core::ids::ViewId;
    use smysl_core::types::{SourceRef, View};

    let part = tid(1);
    let unrelated = manifest("luther1912", 7);
    let m = manifest("kjv1769", 1);
    let unit = UnitCoreBuilder::new(KernelType::Prose, "in the beginning", Status::Cited)
        .source(SourceRef::new(
            SourceKind::Doc,
            format!("{}#Gen.1.1", part.canonical()),
        ))
        .build()
        .expect("a unit");
    let uid = smysl_core::canonical_uid(&unit);

    let mut store = Store::new();
    store
        .append(&[
            Record::Unit(unit),
            Record::Manifest(m.clone()),
            Record::Manifest(unrelated.clone()),
        ])
        .expect("appended");

    let mut view = View::new(ViewId::new("v/bundle").expect("a view id"), "one unit");
    view.roots.insert(uid);
    let bytes = store.bundle(&view);
    let (records, _) = smysl_core::from_cbor_seq(&bytes).expect("the bundle decodes");
    let mids: Vec<_> = records
        .iter()
        .filter_map(|r| match r {
            Record::Manifest(k) => Some(k.mid()),
            _ => None,
        })
        .collect();
    assert_eq!(
        mids,
        vec![m.mid()],
        "the manifest of the text the unit came from travels, and only that one"
    );
}
