//! TX-P9 N5's exit (SMYSL-2.6 §6), §5.2's dialogue tests: D1–D6 on synthetic chats with planted
//! answers, take-ups, returns, introductions, drift and reversals, and clocks that disagree
//! with the reply graph.
//!
//! Each measure has its own small conversation, so that what it is expected to count can be
//! read off the planting. Speakers are `text:speaker`, modes `text:mode`, and times the said
//! axis, which a source's `observed` instant gives (rule E). Every expectation is exact: a
//! ratio as its two integers, a duration as its fixed-point integer of seconds.

use std::path::{Path, PathBuf};

use smysl_core::diag::Code;
use smysl_core::ids::{LangTag, Uid};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::object_to_payload;
use smysl_core::types::{Carry, RelKind, SourceKind, SourceRef};
use smysl_core::{canonical_uid, KernelType, Record, Relation, Span, Status, UnitCoreBuilder};
use smysl_graph::proposition::SAME_AS;
use smysl_graph::Store;
use smysl_measure::text::Corpus;
use smysl_measure::{
    run, ClassPolicy, Input, Level, MeasureResult, Num, Outcome, Recipe, Resolved, Stream, Value,
};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

const SCOPE: &str = "from channel:planted";

/// 2026-01-02T09:00:00Z, the planted chats' first instant.
const T0: u64 = 1_767_344_400_000;

/// `s` seconds into the chat.
fn at(s: u64) -> u64 {
    T0 + 1000 * s
}

/// `s` seconds, as a fixed-point duration is stored.
fn secs(s: u64) -> Num {
    Num::Fixed((s << 20) as i64)
}

fn payload(fields: &[(&str, &str)]) -> Vec<u8> {
    let mut o = HObject::new();
    for (k, v) in fields {
        o.insert(
            Spanned::new(k.to_string(), Span::new(0, 0)),
            Spanned::new(HValue::Str(v.to_string()), Span::new(0, 0)),
        );
    }
    object_to_payload(&o).unwrap()
}

/// A planted conversation.
#[derive(Default)]
struct Chat {
    records: Vec<Record>,
    scope: Vec<Uid>,
    n: u32,
}

impl Chat {
    /// A unit of the conversation: `kind`, said by `speaker` in `mode` at `when` seconds;
    /// `None` leaves each out.
    fn say(
        &mut self,
        kind: KernelType,
        speaker: Option<&str>,
        mode: Option<&str>,
        when: Option<u64>,
    ) -> Uid {
        let u = self.unit(kind, speaker, mode, when, None);
        self.scope.push(u);
        u
    }

    fn unit(
        &mut self,
        kind: KernelType,
        speaker: Option<&str>,
        mode: Option<&str>,
        when: Option<u64>,
        source: Option<SourceRef>,
    ) -> Uid {
        self.n += 1;
        let mut b = UnitCoreBuilder::new(kind, format!("message {}", self.n), Status::Speculative);
        let mut src = source
            .unwrap_or_else(|| SourceRef::new(SourceKind::Doc, format!("planted#{}", self.n)));
        if let Some(s) = when {
            src = src.observed_at(at(s));
        }
        b = b.source(src);
        let mut fields = Vec::new();
        if let Some(s) = speaker {
            fields.push(("text:speaker", s));
        }
        if let Some(m) = mode {
            fields.push(("text:mode", m));
        }
        if !fields.is_empty() {
            b = b.payload(payload(&fields));
        }
        let core = b.build().unwrap();
        let uid = canonical_uid(&core);
        self.records.push(Record::Unit(core));
        uid
    }

    /// A claim said outside the conversation: in the store, not in the scope.
    fn aside(&mut self, speaker: &str, when: u64) -> Uid {
        self.unit(
            KernelType::Claim,
            Some(speaker),
            Some("asserted"),
            Some(when),
            None,
        )
    }

    fn claim(&mut self, speaker: &str, mode: &str, when: u64) -> Uid {
        self.say(KernelType::Claim, Some(speaker), Some(mode), Some(when))
    }

    fn question(&mut self, speaker: &str, when: u64) -> Uid {
        self.say(KernelType::Question, Some(speaker), None, Some(when))
    }

    fn rel(&mut self, kind: RelKind, from: Uid, to: Uid) {
        self.records
            .push(Record::Relation(Relation::new(kind, from, to)));
    }

    fn same(&mut self, a: Uid, b: Uid) {
        self.rel(RelKind::Extension(SAME_AS.into()), a, b);
    }

    fn store(&self) -> Store {
        let mut s = Store::new();
        s.append(&self.records).expect("appends");
        s
    }

    fn resolved(&self) -> Resolved {
        Resolved::of_uids(SCOPE, self.scope.iter().copied().collect())
    }
}

fn strict() -> Recipe {
    let mut r = Recipe::new("", 0, SCOPE);
    r.policy = Some(ClassPolicy::Strict);
    r
}

fn one(id: &str, recipe: &Recipe, input: &Input<'_>) -> MeasureResult {
    run(&[id], recipe, input).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("{} did not run: {other:?}", r.recipe.id),
    }
}

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was expected to refuse: {other:?}", r.recipe.id),
    }
}

/// D1: four questions. Two are answered a minute and five minutes after they were asked, the
/// first twice; one is answered by a message whose clock puts it *before* the question; one is
/// answered only from outside the conversation, which is no answer here. So `[3, 4]`, one
/// unanswered, one inverted, and times of 60 s and 300 s: a lower median of 60 and a p90 of
/// 300.
#[test]
fn d1_counts_planted_answers_and_times_them() {
    let mut c = Chat::default();
    let q0 = c.question("ada", 0);
    let a0 = c.claim("bea", "asserted", 60);
    let a0b = c.claim("cyd", "asserted", 120);
    let q1 = c.question("ada", 180);
    let a1 = c.claim("bea", "asserted", 480);
    let q2 = c.question("bea", 540);
    let q3 = c.question("cyd", 600);
    let a3 = c.claim("ada", "asserted", 30);
    let outside = c.aside("bea", 700);
    for (a, q) in [(a0, q0), (a0b, q0), (a1, q1), (a3, q3), (outside, q2)] {
        c.rel(RelKind::Answers, a, q);
    }
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    let r = one("D1", &Recipe::new("", 0, SCOPE), &input);
    let v = value(&r);
    assert_eq!(v.values["answered"], Num::Ratio(3, 4));
    assert_eq!(v.n["questions"], 4);
    assert_eq!(v.n["unanswered"], 1);
    assert_eq!(v.n["inverted"], 1);
    assert_eq!(v.n["timed"], 2);
    assert_eq!(v.values["time:median"], secs(60));
    assert_eq!(v.values["time:p90"], secs(300));
    assert!(r.diagnostics.is_empty(), "{:?}", r.diagnostics);
    assert_eq!(
        smysl_measure::gist(&r.recipe, v),
        "D1 answered 0.7500 (3/4), over from channel:planted"
    );

    // No question at all is nothing to count.
    let mut none = Chat::default();
    none.claim("ada", "asserted", 0);
    let s = none.store();
    let sc = none.resolved();
    let input = Input::new(&s, &sc);
    assert_eq!(
        refusal(&one("D1", &Recipe::new("", 0, SCOPE), &input)),
        Code::W485
    );
}

/// D1, D3 and D6 are time-ordered: an undated question is left out of the times and said
/// (`SMY-W415`), and a scope too little of which is dated is refused (`SMY-E482`).
#[test]
fn time_ordered_measures_obey_undated_max() {
    let mut c = Chat::default();
    let q = c.question("ada", 0);
    let a = c.claim("bea", "asserted", 90);
    c.rel(RelKind::Answers, a, q);
    for i in 0..9 {
        c.claim("cyd", "asserted", 100 + i);
    }
    let late = c.say(KernelType::Question, Some("ada"), None, None);
    let b = c.claim("bea", "asserted", 200);
    c.rel(RelKind::Answers, b, late);
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    // 1 of 13 undated: under 1/10.
    let r = one("D1", &Recipe::new("", 0, SCOPE), &input);
    let v = value(&r);
    assert_eq!(v.values["answered"], Num::Ratio(2, 2));
    assert_eq!((v.n["undated"], v.n["untimed"], v.n["timed"]), (1, 1, 1));
    assert_eq!(v.values["time:median"], secs(90));
    assert!(v.flags.iter().any(|f| f == "undated-excluded"));
    assert!(r.diagnostics.iter().any(|d| d.code == Code::W415));

    // Two more undated, 3 of 15: refused, unless the recipe allows a fifth.
    c.say(KernelType::Claim, Some("cyd"), Some("asserted"), None);
    c.say(KernelType::Claim, Some("cyd"), Some("asserted"), None);
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    for id in ["D1", "D3", "D6"] {
        assert_eq!(refusal(&one(id, &strict(), &input)), Code::E482, "{id}");
        let mut lax = strict();
        lax.undated_max = (1, 5);
        value(&one(id, &lax, &input));
    }
    // `known` is not an axis rule E derives.
    let mut known = strict();
    known.axis = smysl_measure::Axis::Known;
    assert_eq!(refusal(&one("D1", &known, &input)), Code::E489);
}

/// D2: eleven assertions with a speaker. Bea elaborates one of Ada's; Bea and Cyd restate each
/// other by `same-as`; Ada elaborates her own; a unit with no speaker rebuts one of Cyd's; and
/// answers and questions do not assert. Three are taken up: Ada `[1, 4]`, Bea `[1, 4]`,
/// Cyd `[1, 3]`, and `[3, 11]` over all.
#[test]
fn d2_counts_what_another_speaker_takes_up() {
    let mut c = Chat::default();
    let q = c.question("ada", 0);
    let b1 = c.claim("bea", "asserted", 10);
    c.rel(RelKind::Answers, b1, q);
    c.claim("cyd", "asserted", 20);
    c.claim("bea", "asserted", 30);
    c.claim("ada", "asserted", 40);
    let a1 = c.claim("ada", "asserted", 50);
    let b2 = c.claim("bea", "asserted", 60);
    c.rel(RelKind::Elaborates, b2, a1);
    let a2 = c.claim("ada", "asserted", 70);
    let a3 = c.claim("ada", "asserted", 80);
    c.rel(RelKind::Elaborates, a3, a2);
    let c1 = c.claim("cyd", "asserted", 90);
    let nobody = c.say(KernelType::Claim, None, None, Some(100));
    c.rel(RelKind::Rebuts, nobody, c1);
    let b3 = c.claim("bea", "asserted", 110);
    let c2 = c.claim("cyd", "asserted", 120);
    c.same(b3, c2);
    // A rebuttal of Ada's from outside the conversation is not a take-up in it.
    let outside = c.aside("cyd", 130);
    let a4 = c.claim("ada", "reported", 140);
    c.rel(RelKind::Rebuts, outside, a1);
    c.rel(RelKind::Rebuts, b1, a4);
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    let v = value(&one("D2", &Recipe::new("", 0, SCOPE), &input)).clone();
    assert_eq!(v.values["uptake"], Num::Ratio(3, 11));
    assert_eq!(v.values["uptake:ada"], Num::Ratio(1, 4));
    assert_eq!(v.values["uptake:bea"], Num::Ratio(1, 4));
    assert_eq!(v.values["uptake:cyd"], Num::Ratio(1, 3));
    assert_eq!((v.n["speakers"], v.n["unspoken"]), (3, 1));
}

/// D3: under `strict`, a class of two said 100 s apart that a later message elaborates at
/// 400 s lives 400 s; a claim a rebuttal reaches 800 s later lives 800 s; the elaboration, the
/// rebuttal and a claim nothing returns to live 0 s; and neither a reference from outside the
/// conversation nor a saved query's edge is one. Five classes, two returned to: a lower median of 0 and a p90 of
/// 800. Without a policy, refused.
#[test]
fn d3_measures_how_long_a_class_lasts() {
    let mut c = Chat::default();
    let x0 = c.claim("ada", "asserted", 0);
    let x1 = c.claim("bea", "asserted", 100);
    c.same(x0, x1);
    let e = c.claim("cyd", "asserted", 400);
    c.rel(RelKind::Elaborates, e, x0);
    let lone = c.claim("bea", "asserted", 50);
    let y = c.claim("ada", "asserted", 200);
    let r = c.claim("bea", "asserted", 1000);
    c.rel(RelKind::Rebuts, r, y);
    let outside = c.aside("cyd", 5000);
    c.rel(RelKind::Elaborates, outside, lone);
    // A saved query's edge is a query's record, not a reply: `lone` would otherwise live
    // until `r`, 950 s.
    c.rel(
        RelKind::Extension(smysl_core::schemas::query::ANSWERS.into()),
        r,
        lone,
    );
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    let v = value(&one("D3", &strict(), &input)).clone();
    assert_eq!(v.n["classes"], 5);
    assert_eq!(v.values["returned"], Num::Ratio(2, 5));
    assert_eq!(v.values["life:median"], secs(0));
    assert_eq!(v.values["life:p90"], secs(800));
    assert_eq!(
        refusal(&one("D3", &Recipe::new("", 0, SCOPE), &input)),
        Code::E481
    );

    // The same chat with Bea's late claim joined to the class of two: that class now lives
    // until 1000 s, the rebuttal's reference to `y` aside.
    c.same(x0, r);
    c.same(x1, r);
    let s = c.store();
    let input = Input::new(&s, &sc);
    let v = value(&one("D3", &strict(), &input)).clone();
    assert_eq!(v.n["classes"], 4);
    assert_eq!(v.values["life:p90"], secs(1000));
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// D4 in reading order, over units drawn from seven places of Ruth's first part standing in
/// for seven messages: a reading order is a manifest's and a span's, whichever reader made
/// them, and this crate's tests build no chat reader. Ada introduces three of six classes in two messages, Bea one in two, Cyd one
/// in two, and a unit with no speaker the sixth: Ada's gap is 3/6 − 2/6 = 1/6, the others'
/// −1/6. A unit with no place is left out.
#[test]
fn d4_compares_what_a_speaker_introduced_with_how_much_they_talked() {
    let root = std::env::temp_dir().join(format!("smysl-measure-dialogue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    let raw = std::fs::read(repo("fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm")).unwrap();
    let spec = AddSpec::new("usfm/1", "kjv/08-rut", "public-domain")
        .with_lang(LangTag::new("en").unwrap())
        .with_carry(Carry::Text);
    library
        .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
        .unwrap();
    let (mid, manifest) = library
        .store()
        .manifests()
        .next()
        .map(|(m, x)| (*m, x.clone()))
        .unwrap();
    let tid = manifest.parts[0].tid;
    let mut c = Chat::default();
    c.records.push(Record::Manifest(manifest));
    // (message, speaker) per unit, in the order the units are planted, which is not the
    // reading order: the spans give that.
    let drawn = |c: &mut Chat, msg: u64, slot: u64, speaker: Option<&str>| {
        let src = SourceRef::new(
            SourceKind::Doc,
            format!("{}#Ruth.1.{}", tid.canonical(), msg + 1),
        )
        .in_part(20 * msg + slot, 20 * msg + slot + 1, mid)
        .unwrap();
        let u = c.unit(
            KernelType::Claim,
            speaker,
            Some("asserted"),
            None,
            Some(src),
        );
        c.scope.push(u);
        u
    };
    let g = drawn(&mut c, 4, 0, Some("cyd"));
    let a = drawn(&mut c, 0, 0, Some("ada"));
    let b = drawn(&mut c, 0, 1, Some("ada"));
    let cc = drawn(&mut c, 1, 0, Some("bea"));
    let d = drawn(&mut c, 1, 1, Some("bea"));
    drawn(&mut c, 2, 0, Some("ada"));
    let f = drawn(&mut c, 3, 0, Some("bea"));
    drawn(&mut c, 5, 0, Some("cyd"));
    drawn(&mut c, 6, 0, None);
    // A unit with no source has no place to be read at.
    let placeless = UnitCoreBuilder::new(KernelType::Claim, "nowhere", Status::Speculative)
        .payload(payload(&[("text:speaker", "ada")]))
        .build()
        .unwrap();
    c.scope.push(canonical_uid(&placeless));
    c.records.push(Record::Unit(placeless));
    c.same(a, cc);
    c.same(b, f);
    c.same(d, g);
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    let v = value(&one("D4", &strict(), &input)).clone();
    let sixth = Num::Fixed(174_763);
    assert_eq!(v.n["classes"], 6);
    assert_eq!(v.n["messages"], 6);
    assert_eq!((v.n["unplaced"], v.n["unspoken:classes"]), (1, 1));
    assert_eq!(v.values["intro:ada"], Num::Ratio(3, 6));
    assert_eq!(v.values["talk:ada"], Num::Ratio(2, 6));
    assert_eq!(v.values["gap:ada"], sixth);
    assert_eq!(v.values["gap:bea"], Num::Fixed(-174_763));
    assert_eq!(v.values["gap:cyd"], Num::Fixed(-174_763));
    assert_eq!(v.values["max:gap"], sixth);
    assert_eq!(v.values["min:gap"], Num::Fixed(-174_763));
    assert_eq!(
        refusal(&one("D4", &Recipe::new("", 0, SCOPE), &input)),
        Code::E481
    );
    drop(library);
    let _ = std::fs::remove_dir_all(&root);
}

/// D5: windows of four terms — two the same, then four new terms, then a partial window of
/// two. The pairs are 0 bits and 1 bit exactly, the mean 1/2; the partial window is not
/// compared. At any level but a window, refused.
#[test]
fn d5_is_the_divergence_of_consecutive_full_windows() {
    let w = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<String>>();
    let corpus = Corpus::from_terms(
        "smysl/an-en/1:surface",
        Stream::Surface,
        Level::Window(4),
        vec![
            w(&["a", "b", "c", "d"]),
            w(&["d", "c", "b", "a"]),
            w(&["e", "f", "g", "h"]),
            w(&["a", "b"]),
        ],
    );
    let s = Store::new();
    let sc = Resolved::of_uids(SCOPE, Default::default());
    let input = Input::new(&s, &sc).with_text(&corpus);
    let mut r = Recipe::new("", 0, SCOPE);
    r.level = Level::Window(4);
    let v = value(&one("D5", &r, &input)).clone();
    assert_eq!(v.series["jsd"], [Num::Fixed(0), Num::Fixed(1 << 20)]);
    assert_eq!(v.values["jsd:mean"], Num::Fixed(1 << 19));
    assert_eq!(v.values["jsd:max"], Num::Fixed(1 << 20));
    assert_eq!((v.n["windows"], v.n["partial"], v.n["pairs"]), (3, 1, 2));

    let part = Corpus::from_terms(
        "smysl/an-en/1:surface",
        Stream::Surface,
        Level::Part,
        vec![],
    );
    let input = Input::new(&s, &sc).with_text(&part);
    assert_eq!(
        refusal(&one("D5", &Recipe::new("", 0, SCOPE), &input)),
        Code::E489
    );
}

/// D6: Cyd asserts, negates and asserts one proposition again (two reversals); Bea asserts one
/// twice (none); Ada asserts and negates one at the same instant, ordered by uid (one); Ada
/// reports a claim and asserts it, which is no reversal, since reporting takes no position.
/// Three reversals in four speaker–class pairs, two of them with a reversal.
#[test]
fn d6_counts_reversals_in_time() {
    let mut c = Chat::default();
    let x0 = c.claim("cyd", "asserted", 0);
    let x1 = c.claim("cyd", "negated", 60);
    let x2 = c.claim("cyd", "asserted", 120);
    c.same(x0, x1);
    c.same(x0, x2);
    c.same(x1, x2);
    let y0 = c.claim("bea", "asserted", 10);
    let y1 = c.claim("bea", "asserted", 20);
    c.same(y0, y1);
    let z0 = c.claim("ada", "asserted", 300);
    let z1 = c.claim("ada", "negated", 300);
    c.same(z0, z1);
    let w0 = c.claim("ada", "reported", 400);
    let w1 = c.claim("ada", "asserted", 410);
    c.same(w0, w1);
    let s = c.store();
    let sc = c.resolved();
    let input = Input::new(&s, &sc);
    let v = value(&one("D6", &strict(), &input)).clone();
    assert_eq!(v.values["reversals"], Num::Count(3));
    assert_eq!(v.values["reversals:cyd"], Num::Count(2));
    assert_eq!(v.values["reversals:bea"], Num::Count(0));
    assert_eq!(v.values["reversals:ada"], Num::Count(1));
    assert_eq!(v.values["reversed"], Num::Ratio(2, 4));
    assert_eq!(
        refusal(&one("D6", &Recipe::new("", 0, SCOPE), &input)),
        Code::E481
    );
}
