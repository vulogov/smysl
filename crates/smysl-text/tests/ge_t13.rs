//! **GE-T13 — effective time.** The TX-P3 phase exit (SMYSL-2.4 §5.5, §6 TX-P3 step 5).
//!
//! The gate asks four things, and this file is the test for all four. What it is *run against*
//! answers three of them honestly and the fourth only in part, which is stated here rather than
//! left for a reader to discover:
//!
//! | clause | answered by |
//! |---|---|
//! | no false contested interval on consistent data | the three chat fixtures, unmodified |
//! | synthetic chats with planted skews, every skew detected | the same stores, one instant moved |
//! | `P-E4`/`P-E5` on real data | the same stores — real timestamps and real reply structure, **synthesised units** |
//! | a public chronology with known relative orders | **owed**; the harness takes one, nothing here holds one |
//!
//! # Why the units are synthesised, and why that is not a cheat
//!
//! A chat reaches a *unit* only through ingest, which is TX-P5. So there is no store of units
//! from a chat for this gate to read, and there will not be one until two phases from now —
//! while the gate that blocks TX-P4 is this one. What exists is the reading: one row per
//! message, carrying the instant the export recorded, the speaker, and the platform's own
//! message and reply ids.
//!
//! So the store is built from the rows: one unit per message at its recorded instant, and an
//! `x.text/quotes` relation wherever a row replies to another. That is what ingest will do
//! minus the model, and it makes the **data** real — these are the fixtures' own timestamps and
//! the fixtures' own reply graph — while the units around them are this file's. Which half is
//! which matters: a planted skew detected here is a claim about the engine over real chat
//! timing, and a `P-E4` interval containing the truth is a claim about real instants. Neither
//! is a claim about ingest.
//!
//! # The clause that is owed
//!
//! "A public chronology with known relative orders" is material this repository does not hold,
//! and authoring one here would make it a development set — the same shape as GE-T1's five
//! Bibles and TX-P2 step 1's 500-sentence boundary gold. Point the harness at one with
//! `SMYSL_CHRONOLOGY=<file>`, a TSV of `earlier<TAB>later` over EDTF dates, and that clause is
//! measured with no code change. Without it the harness says so and the debt stays recorded in
//! SMYSL-2.4 §5.5.

use std::collections::BTreeMap;

use smysl_core::ids::{AgentId, LangTag, Tid};
use smysl_core::types::provenance::Hlc;
use smysl_core::types::{
    Axis, Dating, DatingTarget, DatingValue, RelKind, Relation, SourceKind, SourceRef,
};
use smysl_core::{KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_text::limits::{Budget, Caps};
use smysl_text::part::{self, Policy};
use smysl_text::readers::{self, Input, Params};
use smysl_text::structure::Structure;
use smysl_text::time::constraints::QUOTES;
use smysl_text::time::engine::effective;
use smysl_text::time::{Bound, Instant, Subject};

/// A chat fixture, with the parameters its reader needs to read it at all.
struct Chat {
    file: &'static str,
    reader: &'static str,
    params: &'static [(&'static str, &'static str)],
}

const CHATS: &[Chat] = &[
    Chat {
        file: "telegram.json",
        reader: "telegram/1",
        params: &[],
    },
    Chat {
        file: "whatsapp.txt",
        reader: "whatsapp/1",
        // `03/04/2024` is two different days and the transcript does not say which, so the
        // reader requires the order. A sample read without it is not a sample.
        params: &[("date-format", "dmy")],
    },
    Chat {
        file: "slack.zip",
        reader: "slack/1",
        params: &[],
    },
];

/// A seeded xorshift, as the other property harnesses in this crate have it: a failure is
/// reproducible from its seed alone.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn agent() -> AgentId {
    AgentId::new("human:vu").expect("an agent")
}

/// The chats this build can read.
///
/// The three readers are behind features, so a default `cargo test -p smysl-text` has none of
/// them — and a harness that panicked on the first absent reader would make the phase exit
/// unrunnable in the configuration most people build. The guard below is what keeps the filter
/// honest: with no reader built there is nothing to measure, and saying so is not the same as
/// passing.
fn built() -> Vec<&'static Chat> {
    CHATS
        .iter()
        .filter(|c| readers::reader(c.reader).is_ok())
        .collect()
}

/// Whether this build can measure GE-T13 at all, printed rather than silently skipped.
fn have_chats() -> bool {
    if built().is_empty() {
        println!(
            "  no chat reader in this build: GE-T13 needs --features \
             reader-telegram,reader-whatsapp,reader-slack (the `cli` feature turns all three on)"
        );
        return false;
    }
    true
}

/// A store built from a chat's reading, and the instants it was built from.
struct Chronology {
    records: Vec<Record>,
    /// The instant each unit's own record carries, which is the truth `P-E4` is checked against.
    truth: BTreeMap<Uid, u64>,
    /// The part every unit's source names, so a window target has a tid to narrow.
    tid: Tid,
    /// `(earlier, later)` for every reply the export recorded.
    orders: Vec<(Uid, Uid)>,
}

/// Read a chat and build the store a corpus would hold for it.
///
/// `skew` moves one message's recorded instant by that many milliseconds — the planted fault.
/// Which message is not arbitrary: it is the one that is replied to, because moving a message
/// nobody answers breaks no order and would be a skew the engine is right not to detect.
fn chronology(chat: &Chat, skew: i64) -> Chronology {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/library/readers")
        .join(chat.file);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("fixtures/.../{}: {e}", chat.file));
    let reader = readers::reader(chat.reader).unwrap_or_else(|e| panic!("{}: {e}", chat.reader));
    let mut params = Params::new();
    for (k, v) in chat.params {
        params.set(chat.reader, *k, *v).expect("a parameter");
    }
    let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("a budget");
    let out = readers::read_with(reader, &Input::new(&bytes), &params, &mut budget)
        .unwrap_or_else(|e| panic!("{}: {e}", chat.file));

    // The tid of the first part, cut the way `text add` cuts: the reader's own top level,
    // because a policy whose boundary level names no row produces no parts at all.
    let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
        .unwrap_or_else(|e| panic!("{}: the rows are not a structure: {e}", chat.file));
    let policy = Policy {
        boundary_level: out.top_level.clone(),
        ..Policy::default()
    };
    let boundaries: Vec<std::ops::Range<u64>> = structure
        .at_level(&out.top_level)
        .map(|n| n.range.clone())
        .collect();
    let plans = part::group(&boundaries, &policy, out.text.len() as u64, &mut budget)
        .unwrap_or_else(|e| panic!("{}: grouping: {e}", chat.file));
    let first = part::text_of(&out.text, plans[0].range.clone())
        .unwrap_or_else(|| panic!("{}: part 0 is not a normalised slice", chat.file));
    let tid = first.tid;

    // The message that is answered, if any: the one a skew has to move to break an order.
    //
    // Two keys, because the two readers that record an ordering spell it differently.
    // `telegram/1` gives `reply` — the message this one replies to — and `slack/1` gives
    // `thread`, the root of the thread this one is in. A thread's root names itself, which is
    // not an ordering and is skipped below.
    let answered: Option<String> = out
        .rows
        .iter()
        .filter_map(|r| {
            let target = r.ids.get("reply").or_else(|| r.ids.get("thread"))?;
            (Some(target) != r.ids.get("msg")).then(|| target.clone())
        })
        .next();

    let mut records = Vec::new();
    let mut truth = BTreeMap::new();
    let mut by_msg: BTreeMap<String, Uid> = BTreeMap::new();
    let mut replies: Vec<(String, Uid)> = Vec::new();
    for (i, row) in out.rows.iter().enumerate() {
        let Some(observed) = row.observed else {
            // A day or a channel node, which has no instant of its own. Only the message rows
            // become units, which is the same choice ingest will make.
            continue;
        };
        let moved = row
            .ids
            .get("msg")
            .is_some_and(|m| Some(m) == answered.as_ref());
        let at = if moved {
            observed.saturating_add_signed(skew)
        } else {
            observed
        };
        let core = UnitCoreBuilder::new(
            KernelType::Evidence,
            format!(
                "message {i} of {}, long enough to clear admission",
                chat.file
            ),
            Status::Cited,
        )
        .source(
            SourceRef::new(
                SourceKind::Doc,
                format!("{}#{}", tid.canonical(), row.locator),
            )
            .observed_at(at),
        )
        .build()
        .expect("a well-formed unit");
        let uid = smysl_core::canonical_uid(&core);
        truth.insert(uid, at);
        if let Some(msg) = row.ids.get("msg") {
            by_msg.insert(msg.clone(), uid);
        }
        if let Some(target) = row.ids.get("reply").or_else(|| row.ids.get("thread")) {
            // A thread's root names itself. That is a message, not an ordering, and treating
            // it as one would put a unit before itself and plant a fault the export does not
            // contain.
            if Some(target) != row.ids.get("msg") {
                replies.push((target.clone(), uid));
            }
        }
        records.push(Record::Unit(core));
    }

    // A reply quotes what it answers, so the answered message is earlier. This is the only
    // ordering in the store, and it is the export's, not this file's.
    let mut orders = Vec::new();
    for (reply, later) in replies {
        let Some(earlier) = by_msg.get(&reply).copied() else {
            continue;
        };
        records.push(Record::Relation(Relation::new(
            RelKind::Extension(QUOTES.to_string()),
            later,
            earlier,
        )));
        orders.push((earlier, later));
    }

    let _ = LangTag::new("mul");
    Chronology {
        records,
        truth,
        tid,
        orders,
    }
}

fn store_of(records: &[Record]) -> Store {
    Store::from_records(records.to_vec())
}

#[test]
fn no_false_contested_interval_on_consistent_data() {
    if !have_chats() {
        return;
    }
    for chat in built() {
        let c = chronology(chat, 0);
        assert!(
            !c.truth.is_empty(),
            "{}: no message carried an instant, so this proves nothing",
            chat.file
        );
        let store = store_of(&c.records);
        let e = effective(&store, Axis::Said);
        assert!(
            e.inconsistent.is_empty(),
            "{}: a consistent chat must raise no SMY-W413: {:?}",
            chat.file,
            e.inconsistent
        );
        assert_eq!(
            e.contested().count(),
            0,
            "{}: and nothing may be reported contested",
            chat.file
        );
        assert!(
            e.not_applied.is_empty(),
            "{}: nor may anything be reported unapplied, there being no datings: {:?}",
            chat.file,
            e.not_applied
        );
        println!(
            "  {:<16} {} units, {} orderings, clean",
            chat.file,
            c.truth.len(),
            c.orders.len()
        );
    }
}

#[test]
fn p_e4_every_interval_contains_the_instant_the_export_recorded() {
    if !have_chats() {
        return;
    }
    for chat in built() {
        let c = chronology(chat, 0);
        let store = store_of(&c.records);
        let e = effective(&store, Axis::Said);
        for (uid, at) in &c.truth {
            let i = e.interval(&Subject::Unit(*uid));
            let inside = match (i.lo, i.hi) {
                (Bound::At(lo), Bound::At(hi)) => lo.0 <= *at as i64 && (*at as i64) < hi.0,
                (Bound::At(lo), Bound::Open) => lo.0 <= *at as i64,
                (Bound::Open, Bound::At(hi)) => (*at as i64) < hi.0,
                (Bound::Open, Bound::Open) => true,
            };
            assert!(
                inside,
                "{}: {uid} is recorded at {at} and the engine says {i}",
                chat.file
            );
        }
    }
}

#[test]
fn p_e5_every_planted_skew_is_detected() {
    // Four skews, each large enough to put a message after the reply that answers it. The
    // smallest is chosen to be bigger than any gap in the fixtures and the largest to be a
    // fault nobody could mistake for a rounding error.
    const SKEWS: &[i64] = &[2 * 3_600_000, 26 * 3_600_000, 400 * 86_400_000, -1];
    if !have_chats() {
        return;
    }
    for chat in built() {
        let baseline = chronology(chat, 0);
        if baseline.orders.is_empty() {
            // An export with no reply ids carries no ordering, so a skew in it contradicts
            // nothing and detecting one would be the engine inventing a fault. Named rather
            // than skipped quietly: it is a property of the source, and `telegram/1` is the
            // reader of the three that records a reply.
            println!(
                "  {:<16} no reply ids in the export; no order for a skew to break",
                chat.file
            );
            continue;
        }
        for skew in SKEWS {
            let c = chronology(chat, *skew);
            let store = store_of(&c.records);
            let e = effective(&store, Axis::Said);
            let moved: Vec<Uid> = c
                .truth
                .iter()
                .filter(|(uid, at)| {
                    baseline.truth.get(uid) != Some(at) || !baseline.truth.contains_key(uid)
                })
                .map(|(uid, _)| *uid)
                .collect();
            assert!(
                !moved.is_empty() || *skew == 0,
                "{}: skew {skew} moved no instant, so the case is empty",
                chat.file
            );
            // A skew that still leaves the order true is not a fault. Only the ones that put a
            // message after its own reply have to be detected, which is the claim GE-T13 makes.
            let broken = c
                .orders
                .iter()
                .any(|(earlier, later)| c.truth[earlier] > c.truth[later]);
            if !broken {
                println!("  {:<16} skew {skew:>14} leaves the order true", chat.file);
                continue;
            }
            assert!(
                !e.inconsistent.is_empty(),
                "{}: skew {skew} puts a message after its own reply and was not detected",
                chat.file
            );
            assert!(
                e.contested().count() > 0,
                "{}: skew {skew} was reported without contesting anything",
                chat.file
            );
            println!(
                "  {:<16} skew {skew:>14} detected, {} contested",
                chat.file,
                e.contested().count()
            );
        }
    }
}

#[test]
fn a_window_dating_repairs_a_planted_skew_and_the_contention_goes() {
    let skew = 26 * 3_600_000i64;
    if !have_chats() {
        return;
    }
    for chat in built() {
        let c = chronology(chat, skew);
        if c.orders.is_empty() {
            continue;
        }
        if !c
            .orders
            .iter()
            .any(|(earlier, later)| c.truth[earlier] > c.truth[later])
        {
            continue;
        }
        // The correction: a window over the part, narrowed to the skewed instant, carrying the
        // offset that undoes it. The basis is one of the units, which is `cited` — enough to
        // move a `cited` bound and not more.
        let skewed = *c
            .orders
            .iter()
            .find(|(earlier, later)| c.truth[earlier] > c.truth[later])
            .map(|(earlier, _)| earlier)
            .expect("a broken order");
        let at = c.truth[&skewed];
        let basis = *c
            .truth
            .keys()
            .find(|u| **u != skewed)
            .expect("a second unit to cite");
        let mut records = c.records.clone();
        records.push(Record::Dating(
            Dating::new(
                DatingTarget::Window {
                    tid: c.tid,
                    from_ms: at,
                    to_ms: at + 1,
                },
                Axis::Said,
                DatingValue::Offset(-skew),
                agent(),
                Hlc::new(1, 0, agent()),
            )
            .with_basis(basis),
        ));
        let store = store_of(&records);
        let e = effective(&store, Axis::Said);
        assert!(
            e.inconsistent.is_empty(),
            "{}: the correction must settle the contradiction: {:?}",
            chat.file,
            e.inconsistent
        );
        assert!(
            e.not_applied.is_empty(),
            "{}: and must itself have been applied: {:?}",
            chat.file,
            e.not_applied
        );
        let i = e.interval(&Subject::Unit(skewed));
        assert_eq!(
            i.lo,
            Bound::At(Instant(at as i64 - skew)),
            "{}: and must put the message back where it belongs",
            chat.file
        );
        println!(
            "  {:<16} skew {skew} corrected by one window dating",
            chat.file
        );
    }
}

/// **Synthetic chats with planted skews**, which is GE-T13's own wording.
///
/// The three fixtures carry five real orderings between them, and `whatsapp.txt` carries none
/// because the export format records no reply. Five is not a measurement of a detector. So this
/// generates chats instead: a run of messages in time order, each quoting the one before it, and
/// one instant moved by a planted skew. The orderings are synthetic and the arithmetic is not —
/// what is under test is whether a skew that contradicts a known order is always found, and
/// whether one that does not is never reported.
///
/// Both halves matter equally. A detector that flagged every skew would pass the first clause
/// and fail the gate, because GE-T13 blocks TX-P4 on *either* an undetected skew or a false
/// contested interval on consistent data.
#[test]
fn synthetic_chats_with_planted_skews() {
    let mut rng = Rng(0x6E7_1335_D47E_0013);
    let tid = Tid::of(b"a synthetic chat");
    let mut planted = 0usize;
    let mut benign = 0usize;
    for case in 0..64 {
        let n = 3 + rng.below(6);
        // A minute apart, so a skew smaller than a minute cannot break an order and a skew
        // larger than the whole run certainly can.
        let base = 1_705_312_680_000u64;
        let gap = 60_000u64;
        let at: Vec<u64> = (0..n).map(|i| base + i as u64 * gap).collect();

        // The planted fault: move one message, sometimes by too little to matter.
        let victim = rng.below(n);
        let skew: i64 = match case % 4 {
            0 => -(gap as i64) * (n as i64),
            1 => (gap as i64) * (n as i64),
            2 => -1,
            _ => 1,
        };
        let moved: Vec<u64> = at
            .iter()
            .enumerate()
            .map(|(i, a)| {
                if i == victim {
                    a.saturating_add_signed(skew)
                } else {
                    *a
                }
            })
            .collect();

        let mut records = Vec::new();
        let mut uids = Vec::new();
        for (i, a) in moved.iter().enumerate() {
            let core = UnitCoreBuilder::new(
                KernelType::Evidence,
                format!("synthetic message {i} of case {case}, long enough to admit"),
                Status::Cited,
            )
            .source(
                SourceRef::new(SourceKind::Doc, format!("{}#chat.{i}", tid.canonical()))
                    .observed_at(*a),
            )
            .build()
            .expect("a well-formed unit");
            uids.push(smysl_core::canonical_uid(&core));
            records.push(Record::Unit(core));
        }
        // Each message quotes the one before it: a chain, so a skew anywhere inside it breaks
        // a link. A star would let a skew at a leaf break only one.
        for i in 1..n {
            records.push(Record::Relation(Relation::new(
                RelKind::Extension(QUOTES.to_string()),
                uids[i],
                uids[i - 1],
            )));
        }

        let store = store_of(&records);
        let e = effective(&store, Axis::Said);
        let breaks = (1..n).any(|i| moved[i - 1] > moved[i]);
        if breaks {
            assert!(
                !e.inconsistent.is_empty(),
                "case {case}: message {victim} moved by {skew} breaks the chain and was not \
                 detected"
            );
            planted += 1;
        } else {
            assert!(
                e.inconsistent.is_empty(),
                "case {case}: a skew of {skew} leaves every order true and must not be \
                 reported: {:?}",
                e.inconsistent
            );
            assert_eq!(
                e.contested().count(),
                0,
                "case {case}: nor may anything be contested"
            );
            benign += 1;
        }
    }
    println!("  synthetic: {planted} skews detected, {benign} benign skews not reported");
    assert!(
        planted > 0 && benign > 0,
        "both halves must have been exercised"
    );
}

/// The clause this repository cannot answer for itself.
///
/// A chronology is a list of pairs whose order is known independently of any corpus — which is
/// what makes it a measurement rather than a restatement of the engine's own arithmetic. The
/// harness is here; the material is owed, and recorded in SMYSL-2.4 §5.5 beside GE-T1's Bibles.
#[test]
fn a_public_chronology_orders_as_it_should() {
    let Some(path) = std::env::var_os("SMYSL_CHRONOLOGY") else {
        println!(
            "  no chronology: set SMYSL_CHRONOLOGY=<file> (TSV of earlier<TAB>later, EDTF).\n  \
             GE-T13's fourth clause is owed, not met."
        );
        return;
    };
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.to_string_lossy()));
    let mut pairs = 0usize;
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((a, b)) = line.split_once('\t') else {
            panic!("line {}: `{line}` is not `earlier<TAB>later`", n + 1);
        };
        let ea =
            smysl_core::edtf::parse(a.trim()).unwrap_or_else(|e| panic!("line {}: {e}", n + 1));
        let eb =
            smysl_core::edtf::parse(b.trim()).unwrap_or_else(|e| panic!("line {}: {e}", n + 1));
        let ia = smysl_text::time::edtf::to_interval(&ea);
        let ib = smysl_text::time::edtf::to_interval(&eb);
        // The claim is that the engine does not *contradict* a known order: the earlier date's
        // extent may not start after the later one's ends. It is not that the extents are
        // disjoint — `1066` before `11xx` is a true order between overlapping extents.
        if let (Bound::At(lo), Bound::At(hi)) = (ia.lo, ib.hi) {
            assert!(
                lo.0 < hi.0,
                "line {}: `{a}` is known to precede `{b}` and its extent starts after the other ends",
                n + 1
            );
        }
        pairs += 1;
    }
    assert!(pairs > 0, "the chronology is empty");
    println!("  chronology: {pairs} known orders, none contradicted");
}
