//! GE-T8 (SMYSL-2.6 §5.5), the instrument: D1's precision on the `answers` edges it counts,
//! against a sheet of hand-labelled question/answer pairs. Below 0.8, D1 leaves the defaults.
//!
//! # What runs here, and what is owed
//!
//! The gate's material is 300 hand-labelled pairs from 3 chat exports. Neither half exists in
//! this repository: an `answers` edge between two chat units is written by extraction (a model
//! reading the conversation, SMYSL-2.4 TX-P5), so a store with real ones costs model turns,
//! and the labels are a person's reading of each edge. So the clause is **owed**, and what is
//! here is the instrument and the place to hand it the material:
//!
//! ```text
//! SMYSL_GE_T8_STORE=<library root>   a library whose store holds the extracted chats
//! SMYSL_GE_T8_SHEET=<file.tsv>       `answer<TAB>question<TAB>yes|no`, one row a labelled edge
//! ```
//!
//! With both set the harness reads the edges D1 counts over every unit of the store
//! (`dialogue::d1::answered`), looks each up on the sheet, and decides the clause — at least
//! 300 labelled edges, and a precision of at least 0.8 — failing when the bar is missed.
//! Without them it runs the **scripted arm**: a planted chat whose edges are right or wrong by
//! construction, which shows that the computation and the bar do what they say and decides
//! nothing about any real conversation.
//!
//! An edge D1 counts that the sheet has no row for is **unlabelled**: counted and printed,
//! and out of the precision, since a precision over edges nobody read is a guess about them.
//! "Leaves the defaults" has nothing to act on yet: `smysl measure` runs the ids it is given,
//! and has no default set for D1 to leave.

use std::collections::BTreeMap;
use std::path::PathBuf;

use smysl_core::types::{RelKind, SourceKind, SourceRef};
use smysl_core::{canonical_uid, KernelType, Record, Relation, Status, Uid, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_measure::dialogue::d1::answered;
use smysl_text::library::Library;

/// The bar: below it, D1 leaves the defaults.
const BAR: (u64, u64) = (4, 5);

/// The labelled edges a decision needs.
const PAIRS: u64 = 300;

/// What the harness found: edges counted, and how the sheet labelled them.
#[derive(Debug, PartialEq, Eq)]
struct Tally {
    counted: u64,
    yes: u64,
    no: u64,
    unlabelled: u64,
}

impl Tally {
    fn labelled(&self) -> u64 {
        self.yes + self.no
    }

    /// `yes / labelled ≥ 4/5`, in integers.
    fn meets_bar(&self) -> bool {
        self.labelled() > 0 && self.yes * BAR.1 >= BAR.0 * self.labelled()
    }
}

/// D1's edges over every unit of `store`, against `sheet`.
fn tally(store: &Store, sheet: &BTreeMap<(Uid, Uid), bool>) -> Tally {
    let uids: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
    let mut t = Tally {
        counted: 0,
        yes: 0,
        no: 0,
        unlabelled: 0,
    };
    for (q, answers) in answered(store, &uids) {
        for a in answers {
            t.counted += 1;
            match sheet.get(&(a, q)) {
                Some(true) => t.yes += 1,
                Some(false) => t.no += 1,
                None => t.unlabelled += 1,
            }
        }
    }
    t
}

fn read_sheet(path: &PathBuf) -> BTreeMap<(Uid, Uid), bool> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() || line.starts_with("answer\t") {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            f.len(),
            3,
            "a sheet row is `answer question yes|no`: {line}"
        );
        let uid = |s: &str| Uid::parse(s).unwrap_or_else(|e| panic!("`{s}`: {e}"));
        let verdict = match f[2].trim() {
            "yes" => true,
            "no" => false,
            other => panic!("a verdict is yes or no, not `{other}`"),
        };
        out.insert((uid(f[0]), uid(f[1])), verdict);
    }
    out
}

/// A planted chat of ten questions, each answered once: `wrong` of the answers point at the
/// question after the one they answer, which is the mistake an extractor reading a fast chat
/// makes. The sheet labels every edge from the planting, so it is of record by construction.
fn scripted(wrong: usize) -> (Store, BTreeMap<(Uid, Uid), bool>) {
    let mut records = Vec::new();
    let mut questions = Vec::new();
    let mut answers = Vec::new();
    for i in 0..10u64 {
        for (kind, list) in [
            (KernelType::Question, &mut questions),
            (KernelType::Claim, &mut answers),
        ] {
            let core = UnitCoreBuilder::new(kind, format!("{kind:?} {i}"), Status::Speculative)
                .source(SourceRef::new(SourceKind::Doc, format!("chat#{i}")).observed_at(1000 * i))
                .build()
                .unwrap();
            list.push(canonical_uid(&core));
            records.push(Record::Unit(core));
        }
    }
    let mut sheet = BTreeMap::new();
    for i in 0..10 {
        let q = if i < wrong {
            questions[(i + 1) % 10]
        } else {
            questions[i]
        };
        records.push(Record::Relation(Relation::new(
            RelKind::Answers,
            answers[i],
            q,
        )));
        sheet.insert((answers[i], q), i >= wrong);
    }
    let mut s = Store::new();
    s.append(&records).unwrap();
    (s, sheet)
}

#[test]
fn ge_t8() {
    let store = std::env::var_os("SMYSL_GE_T8_STORE").map(PathBuf::from);
    let sheet = std::env::var_os("SMYSL_GE_T8_SHEET").map(PathBuf::from);
    if let (Some(root), Some(sheet)) = (store, sheet) {
        let lib = Library::open(&root).unwrap_or_else(|e| panic!("{}: {e}", root.display()));
        let t = tally(lib.store(), &read_sheet(&sheet));
        println!(
            "GE-T8: D1 counts {} answers edges; {} labelled yes, {} no, {} unlabelled",
            t.counted, t.yes, t.no, t.unlabelled
        );
        assert!(
            t.labelled() >= PAIRS,
            "GE-T8 needs {PAIRS} labelled edges and the sheet labels {} of D1's",
            t.labelled()
        );
        assert!(
            t.meets_bar(),
            "GE-T8 fails: D1's precision is {}/{}, below {}/{}; D1 leaves the defaults",
            t.yes,
            t.labelled(),
            BAR.0,
            BAR.1
        );
        return;
    }

    // The scripted arm: eight of ten right is the bar exactly, seven is below it.
    let (s, sheet) = scripted(2);
    let t = tally(&s, &sheet);
    assert_eq!(
        t,
        Tally {
            counted: 10,
            yes: 8,
            no: 2,
            unlabelled: 0
        }
    );
    assert!(t.meets_bar());
    let (s, sheet) = scripted(3);
    let t = tally(&s, &sheet);
    assert_eq!((t.yes, t.no), (7, 3));
    assert!(!t.meets_bar());

    // An edge the sheet has no row for is counted and left out of the precision.
    let (s, mut sheet) = scripted(0);
    let first: (Uid, Uid) = *sheet.keys().next().unwrap();
    sheet.remove(&first);
    let t = tally(&s, &sheet);
    assert_eq!((t.counted, t.labelled(), t.unlabelled), (10, 9, 1));

    println!(
        "GE-T8 is OWED: no chat store with extracted answers edges and no hand-labelled sheet \
         exist here. SMYSL_GE_T8_STORE=<library root> and SMYSL_GE_T8_SHEET=<file.tsv> decide it."
    );
}
