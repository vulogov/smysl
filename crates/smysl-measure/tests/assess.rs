//! TX-P9 N6's exit (SMYSL-2.6 §6), §5.2's assessment tests: a key from a view and from a
//! dossier's slots; a reader with planted answers — correct, inflated, holder-changed,
//! fabricated, unanswered, unscorable, and one item whose answer was retracted — every count
//! exact; a class whose id moves when a smaller member joins between two states, which the
//! diff reports as kept; and Krippendorff's α on his own worked example.

use std::collections::BTreeSet;

use smysl_core::diag::Code;
use smysl_core::ids::{AgentId, LangTag, Mid, Uid, ViewId};
use smysl_core::schemas::query;
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::object_to_payload;
use smysl_core::types::{Hlc, RelKind, SourceKind, SourceRef};
use smysl_core::{
    canonical_uid, Attestation, KernelType, Op, Record, Relation, Rung, SchemaId, Span, Status,
    UnitCoreBuilder, View,
};
use smysl_graph::proposition::SAME_AS;
use smysl_graph::Store;
use smysl_measure::assess::agreement::{alpha, alpha_nominal};
use smysl_measure::assess::{diff, key_from_view, score, state, Judge, Key, Mode, State};
use smysl_measure::ClassPolicy;
use smysl_text::proposition::Lexical;

fn reader() -> AgentId {
    AgentId::new("human:ada").unwrap()
}

/// A planted store, built in steps so a key can be made before the responses exist.
#[derive(Default)]
struct Planted {
    records: Vec<Record>,
    n: u32,
    anchor: Option<Uid>,
}

impl Planted {
    fn unit(&mut self, kind: KernelType, gist: &str, status: Status, holder: Option<Uid>) -> Uid {
        self.n += 1;
        let anchor = *self.anchor.get_or_insert_with(|| {
            let a = UnitCoreBuilder::new(KernelType::Claim, "the anchor", Status::Speculative)
                .build()
                .unwrap();
            let uid = canonical_uid(&a);
            self.records.push(Record::Unit(a));
            uid
        });
        let mut b = UnitCoreBuilder::new(kind, gist, status).lang(LangTag::new("en").unwrap());
        if matches!(status, Status::Cited | Status::Measured) {
            b = b.source(SourceRef::new(SourceKind::Doc, format!("doc-{}", self.n)));
        }
        if matches!(status, Status::Inferred | Status::Derived) {
            b = b.grounds([anchor]);
        }
        if let Some(h) = holder {
            let mut o = HObject::new();
            o.insert(
                Spanned::new("text:holder".to_string(), Span::new(0, 0)),
                Spanned::new(HValue::Str(h.canonical()), Span::new(0, 0)),
            );
            b = b.payload(object_to_payload(&o).unwrap());
        }
        let u = b.build().unwrap();
        let uid = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        uid
    }

    fn question(&mut self, gist: &str) -> Uid {
        self.unit(KernelType::Question, gist, Status::Speculative, None)
    }

    fn claim(&mut self, gist: &str, status: Status) -> Uid {
        self.unit(KernelType::Claim, gist, status, None)
    }

    fn rel(&mut self, kind: RelKind, from: Uid, to: Uid) {
        self.records
            .push(Record::Relation(Relation::new(kind, from, to)));
    }

    fn same(&mut self, a: Uid, b: Uid) {
        self.rel(RelKind::Extension(SAME_AS.into()), a, b);
    }

    /// A response: `uid`, attested by the reader at rung document, answering `q` if given.
    fn respond(&mut self, uid: Uid, q: Option<Uid>) {
        let agent = reader();
        self.records.push(Record::Attestation(Attestation::new(
            uid,
            agent.clone(),
            Op::Authored,
            Rung::Document,
            Hlc::new(1, 0, agent),
        )));
        if let Some(q) = q {
            self.rel(RelKind::Answers, uid, q);
        }
    }

    fn store(&self) -> Store {
        let mut s = Store::new();
        s.append(&self.records).expect("appends");
        s
    }
}

fn view(roots: &[Uid]) -> View {
    View::new(ViewId::new("v/quiz").unwrap(), "quiz").with_roots(roots.iter().copied())
}

/// K1 from a view: each question with a live answer is an item, its answers in uid order;
/// `d_path` walks grounds to a unit drawn from a span and `d_spans` counts the places reached.
/// A view whose questions nobody answered is refused, and a key survives its own unit.
#[test]
fn a_key_is_made_from_a_view() {
    let mut p = Planted::default();
    let q1 = p.question("Where does the river rise?");
    let q2 = p.question("Who built the bridge?");
    let q3 = p.question("Nobody answered this one");
    // a1 is drawn from a span; a2 rests on a unit that is, two steps away.
    let spanned = |p: &mut Planted, gist: &str, at: u64| {
        let core = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .source(
                SourceRef::new(SourceKind::Doc, "t3:part#1")
                    .in_part(at, at + 5, Mid::of(b"a manifestation"))
                    .unwrap(),
            )
            .build()
            .unwrap();
        let uid = canonical_uid(&core);
        p.records.push(Record::Unit(core));
        uid
    };
    let a1 = spanned(&mut p, "The river rises in the hills", 0);
    let e = spanned(&mut p, "A bridge was built in 1820", 10);
    let mid = UnitCoreBuilder::new(KernelType::Claim, "the bridge is old", Status::Inferred)
        .grounds([e])
        .build()
        .unwrap();
    let mid_uid = canonical_uid(&mid);
    p.records.push(Record::Unit(mid));
    let a2 = UnitCoreBuilder::new(
        KernelType::Claim,
        "The town built the bridge",
        Status::Inferred,
    )
    .grounds([mid_uid])
    .build()
    .unwrap();
    let a2_uid = canonical_uid(&a2);
    p.records.push(Record::Unit(a2));
    p.rel(RelKind::Answers, a1, q1);
    p.rel(RelKind::Answers, a2_uid, q2);
    let s = p.store();
    let key = key_from_view(&s, &view(&[q1, q2, q3]), ClassPolicy::Strict, "", Mode::Any).unwrap();
    assert_eq!(key.items.len(), 2);
    let i1 = key.items.iter().find(|i| i.q == q1).unwrap();
    let i2 = key.items.iter().find(|i| i.q == q2).unwrap();
    assert_eq!(i1.answers, [a1]);
    assert_eq!((i1.d_path, i1.d_spans), (Some(0), 1));
    assert_eq!((i2.d_path, i2.d_spans), (Some(2), 1));

    let unit = key.to_unit().unwrap();
    let uid = canonical_uid(&unit);
    assert_eq!(
        unit.grounds.iter().copied().collect::<BTreeSet<_>>(),
        [q1, q2].into()
    );
    let mut with = p.records.clone();
    with.push(Record::Unit(unit));
    let mut s2 = Store::new();
    s2.append(&with).unwrap();
    assert_eq!(Key::from_unit(&s2, &uid).unwrap(), key);
    assert_eq!(
        key.as_view(ViewId::new("v/key").unwrap()).roots,
        [q1, q2].into()
    );

    let none = key_from_view(&s, &view(&[q3]), ClassPolicy::Strict, "", Mode::Any);
    assert_eq!(none.unwrap_err().code, Code::W485);
}

/// K1 from a dossier's view: a slot's saved query with results is an item, its results the
/// answers.
#[test]
fn a_dossier_slot_is_an_item() {
    let mut p = Planted::default();
    let slot = UnitCoreBuilder::new(
        SchemaId::Extension(query::SAVED.to_string()),
        "slot: causes",
        Status::Cited,
    )
    .source(SourceRef::new(SourceKind::Doc, "sq"))
    .build()
    .unwrap();
    let slot_uid = canonical_uid(&slot);
    p.records.push(Record::Unit(slot));
    let r1 = p.claim("the flood came from the rain", Status::Speculative);
    let r2 = p.claim("the dam was open", Status::Speculative);
    p.rel(RelKind::Extension(query::ANSWERS.into()), slot_uid, r1);
    p.rel(RelKind::Extension(query::ANSWERS.into()), slot_uid, r2);
    let s = p.store();
    let key = key_from_view(&s, &view(&[slot_uid]), ClassPolicy::Strict, "", Mode::All).unwrap();
    assert_eq!(key.items.len(), 1);
    let mut want = vec![r1, r2];
    want.sort();
    assert_eq!(key.items[0].answers, want);
    assert_eq!(key.items[0].mode, Mode::All);
}

/// K3 (§5.2): seven items and a reader who gets three right (one of them inflated, one with
/// its holder changed), two wrong (one fabricated, one wrong but in the texts), leaves one
/// unanswered, answers one whose keyed answer is retracted after the key was made, and writes
/// one unit that answers nothing. Every count is exact.
#[test]
fn a_reader_is_scored_against_planted_answers() {
    let mut p = Planted::default();
    let mut qs = Vec::new();
    let mut answers = Vec::new();
    for i in 0..7 {
        qs.push(p.question(&format!("question {i}")));
        answers.push(p.claim(&format!("keyed answer {i}"), Status::Inferred));
    }
    for i in 0..7 {
        p.rel(RelKind::Answers, answers[i], qs[i]);
    }
    let text_unit = p.claim("something the texts say", Status::Inferred);
    let s = p.store();
    let key = key_from_view(&s, &view(&qs), ClassPolicy::Strict, "from texts", Mode::Any).unwrap();
    let key_unit = key.to_unit().unwrap();
    let key_uid = canonical_uid(&key_unit);
    p.records.push(Record::Unit(key_unit));

    let entity = UnitCoreBuilder::new(
        SchemaId::Extension(smysl_core::schemas::text::ENTITY.into()),
        "Somebody",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let holder = canonical_uid(&entity);
    p.records.push(Record::Unit(entity));
    let r: Vec<Uid> = vec![
        p.claim("response 0", Status::Inferred),
        p.claim("response 1", Status::Cited),
        p.unit(
            KernelType::Claim,
            "response 2",
            Status::Inferred,
            Some(holder),
        ),
        p.claim("response 3", Status::Inferred),
        p.claim("response 4", Status::Inferred),
        p.claim("response 6", Status::Inferred),
    ];
    for (k, i) in [(0, 0), (1, 1), (2, 2)] {
        p.same(r[k], answers[i]);
    }
    p.same(r[4], text_unit);
    for (k, i) in [(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (5, 6)] {
        p.respond(r[k], Some(qs[i]));
    }
    let stray = p.claim("a note to self", Status::Speculative);
    p.respond(stray, None);
    let retractor = p.claim("the answer to question 6 was wrong", Status::Speculative);
    p.rel(RelKind::Retracts, retractor, answers[6]);
    let s = p.store();

    let scope: BTreeSet<Uid> = answers.iter().copied().chain([text_unit]).collect();
    let sc = score(&s, &key, &reader(), None, &Judge::Accepted, Some(&scope)).unwrap();
    assert_eq!(sc.accuracy, (3, 6));
    assert_eq!((sc.unanswered, sc.wrong), (1, 2));
    assert_eq!(sc.inflation, (1, 3));
    assert_eq!(sc.holder, (1, 3));
    assert_eq!(sc.fabrication, Some((1, 5)));
    assert_eq!((sc.unscorable, sc.drifted), (1, 1));
    let codes: Vec<Code> = sc.diagnostics.iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::W487, Code::W488]);

    let unit = sc.to_unit(&key_uid).unwrap();
    assert!(unit.grounds.contains(&key_uid));
    assert_eq!(unit.status, Status::Derived);

    // Without a scope there is no fabrication figure.
    let sc = score(&s, &key, &reader(), None, &Judge::Accepted, None).unwrap();
    assert_eq!(sc.fabrication, None);
}

/// A judge proposes a match the store does not have, and writes nothing: under `lexical`, a
/// response closest to its own item's answer matches it, and one closest to another item's
/// answer matches nothing.
#[test]
fn a_lexical_judge_matches_the_nearest_answer() {
    let mut p = Planted::default();
    let q1 = p.question("Where does the river rise?");
    let q2 = p.question("Who built the bridge?");
    let a1 = p.claim("the river rises in the northern hills", Status::Inferred);
    let a2 = p.claim("the bridge was built by the town masons", Status::Inferred);
    p.rel(RelKind::Answers, a1, q1);
    p.rel(RelKind::Answers, a2, q2);
    let s = p.store();
    let key = key_from_view(&s, &view(&[q1, q2]), ClassPolicy::Strict, "", Mode::Any).unwrap();
    let right = p.claim("in the northern hills the river rises", Status::Inferred);
    let wrong = p.claim("town masons built it, the bridge", Status::Inferred);
    p.respond(right, Some(q1));
    p.respond(wrong, Some(q1));
    let s = p.store();
    let lexical = Judge::Lexical(Lexical {
        k: 1,
        adjacent_only: false,
    });
    let accepted = score(&s, &key, &reader(), None, &Judge::Accepted, None).unwrap();
    assert_eq!(accepted.accuracy, (0, 2));
    let judged = score(&s, &key, &reader(), None, &lexical, None).unwrap();
    assert_eq!(judged.accuracy, (1, 2));
    assert_eq!(
        judged.items.iter().find(|i| i.q == q1).unwrap().matched,
        (1, 1)
    );
    assert_eq!(judged.judge, "lexical");
    assert_eq!(
        s.relations_of_kind(&RelKind::Extension(SAME_AS.into()))
            .len(),
        0
    );
}

/// K4 (§5.2): a class whose id moves when a smaller member joins between two states. The
/// first state's representative is not the second's, and the diff calls the class kept, not
/// lost and gained; the second state also gained a class, and its kept class's strongest
/// member rose from inferred to cited. A state survives its own unit.
#[test]
fn a_moved_class_id_is_kept_in_the_diff() {
    let mut p = Planted::default();
    let q1 = p.question("first");
    let q2 = p.question("second");
    let a1 = p.claim("answer one", Status::Inferred);
    let a2 = p.claim("answer two", Status::Inferred);
    p.rel(RelKind::Answers, a1, q1);
    p.rel(RelKind::Answers, a2, q2);
    let s = p.store();
    let key = key_from_view(&s, &view(&[q1, q2]), ClassPolicy::Strict, "", Mode::Any).unwrap();
    let key_uid = canonical_uid(&key.to_unit().unwrap());
    let r1 = p.claim("my answer one", Status::Inferred);
    p.same(r1, a1);
    p.respond(r1, Some(q1));
    let s = p.store();
    let before = state(&s, &key, &key_uid, &reader(), None, &Judge::Accepted).unwrap();
    assert_eq!(before.classes.len(), 1);

    // A cited unit smaller than both joins their class, and the reader answers again.
    let floor = *[a1, r1].iter().min().unwrap();
    let mut i = 0;
    let small = loop {
        let core =
            UnitCoreBuilder::new(KernelType::Claim, format!("the source {i}"), Status::Cited)
                .source(SourceRef::new(SourceKind::Doc, "doc"))
                .build()
                .unwrap();
        if canonical_uid(&core) < floor {
            let uid = canonical_uid(&core);
            p.records.push(Record::Unit(core));
            break uid;
        }
        i += 1;
    };
    p.same(small, a1);
    p.same(small, r1);
    p.respond(small, Some(q1));
    let r2 = p.claim("my answer two", Status::Inferred);
    p.same(r2, a2);
    p.respond(r2, Some(q2));
    let s = p.store();
    let after = state(&s, &key, &key_uid, &reader(), None, &Judge::Accepted).unwrap();
    assert_eq!(after.classes.len(), 2);
    let rep = |st: &State| -> BTreeSet<Uid> { st.classes.iter().map(|c| c[0]).collect() };
    assert!(rep(&after).contains(&small));
    assert!(!rep(&before).contains(&small), "the class id moved");
    let d = diff(&s, &before, &after, ClassPolicy::Strict).unwrap();
    assert_eq!((d.kept, d.lost, d.gained), (1, 0, 1));
    assert_eq!((d.split, d.merged), (0, 0));
    assert_eq!((d.rose, d.fell), (1, 0));

    let unit = after.to_unit().unwrap();
    let uid = canonical_uid(&unit);
    p.records.push(Record::Unit(unit));
    assert_eq!(State::from_unit(&p.store(), &uid).unwrap(), after);
    assert_eq!(
        after.as_view(ViewId::new("v/state").unwrap()).roots,
        rep(&after)
    );
}

/// α (§5.2): Krippendorff's worked example — four observers, twelve units, values 1 to 5 with
/// gaps — is 0.743 nominal in his paper, and 113/152 exactly here. The three cases the formula
/// has no answer in are `None`, as GE-T5's harness has them.
#[test]
fn alpha_reproduces_krippendorffs_example() {
    let n = None;
    let s = Some;
    let rows: [[Option<u8>; 12]; 4] = [
        [
            s(1),
            s(2),
            s(3),
            s(3),
            s(2),
            s(1),
            s(4),
            s(1),
            s(2),
            n,
            n,
            n,
        ],
        [
            s(1),
            s(2),
            s(3),
            s(3),
            s(2),
            s(2),
            s(4),
            s(1),
            s(2),
            s(5),
            n,
            s(3),
        ],
        [
            n,
            s(3),
            s(3),
            s(3),
            s(2),
            s(3),
            s(4),
            s(2),
            s(2),
            s(5),
            s(1),
            n,
        ],
        [
            s(1),
            s(2),
            s(3),
            s(3),
            s(2),
            s(4),
            s(4),
            s(1),
            s(2),
            s(5),
            s(1),
            n,
        ],
    ];
    let units: Vec<Vec<Option<u8>>> = (0..12)
        .map(|j| rows.iter().map(|r| r[j]).collect())
        .collect();
    let a = alpha_nominal(&units).unwrap();
    assert!(a.is(113, 152), "{a:?}");
    assert_eq!(a.to_num(), Some(smysl_measure::Num::Fixed(779_533)));

    assert_eq!(
        alpha_nominal(&[vec![Some(1)], vec![Some(2)]]),
        None,
        "one coder"
    );
    assert_eq!(
        alpha_nominal(&[vec![Some(1), Some(1)], vec![Some(1), Some(1)]]),
        None,
        "one value"
    );
    assert_eq!(
        alpha_nominal::<u8>(&[vec![None, None]]),
        None,
        "nothing pairable"
    );
}

/// α from attestations: two annotators at rung document over four items, agreeing on two and
/// each marking one the other did not, is −1/6; an attestation at another rung is no mark.
#[test]
fn alpha_reads_document_attestations() {
    let mut p = Planted::default();
    let items: Vec<Uid> = (0..4)
        .map(|i| p.claim(&format!("item {i}"), Status::Speculative))
        .collect();
    let (x, y) = (
        AgentId::new("human:x").unwrap(),
        AgentId::new("human:y").unwrap(),
    );
    let mark = |p: &mut Planted, g: &AgentId, i: usize, rung: Rung| {
        p.records.push(Record::Attestation(Attestation::new(
            items[i],
            g.clone(),
            Op::Attested,
            rung,
            Hlc::new(1, 0, g.clone()),
        )));
    };
    for i in [0, 1, 2] {
        mark(&mut p, &x, i, Rung::Document);
    }
    for i in [0, 1, 3] {
        mark(&mut p, &y, i, Rung::Document);
    }
    mark(&mut p, &y, 2, Rung::Model);
    let a = alpha(&p.store(), &items, &[x, y]).unwrap();
    assert!(a.is(-1, 6), "{a:?}");
}
