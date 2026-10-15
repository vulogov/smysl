//! A reader marks reported speech, and the mode comes out of the structure (TX-P5 step 3b).
//!
//! §3.4 says `text:mode` is written from "reported-speech spans the reader marks", and until
//! the readers learned the two markers real sources carry there were none to write from: the
//! punctuation scan handled a modern edition and nothing handled a 1769 Bible. This is the
//! whole path, end to end and with no model in it: a USFM text with `\wj` goes into a library,
//! the reader puts a [`smysl_text::reading::Mark`] on the verse row, `marks_of` takes it off
//! again, windowing clips it to the node, and `attribution` reads `reported` off it.
//!
//! The claim worth asserting is the pair, not either half. A harness that only checked the
//! marked clause would pass over a reader that marked the whole verse — so the same passage is
//! asked twice, once about words inside the marker and once about the narration around it, and
//! the second answer has to be *nothing at all*: §3.4 forbids `asserted` as a default, and a
//! USFM verse has no speaker to assert it.

#![cfg(feature = "text")]

use std::ops::Range;
use std::path::{Path, PathBuf};

use smysl_core::ids::{KernelType, LangTag, Tid};
use smysl_core::quote::{Normaliser, QUOTE_KEY};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::payload::{object_to_payload, payload_to_object};
use smysl_core::types::{Carry, Status, UnitCore, UnitCoreBuilder};
use smysl_core::Span;
use smysl_ingest::text::{attach, marks_of, windows, Opts, Pane};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;
use smysl_text::reading::Level;

/// Matthew 4:4 as an edition that marks the words of Jesus writes it.
const MATTHEW: &str = "\\id MAT\n\\c 4\n\\p\n\\v 4 But he answered and said, \
                       \\wj It is written, Man shall not live by bread alone.\\wj*\n";

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "smysl-reported-speech-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        Scratch(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A unit as an extractor leaves it: a quote in the payload and no span yet.
fn unit_with(quote: &str) -> UnitCore {
    let span = Span::new(0, 0);
    let mut obj = HObject::default();
    obj.insert(
        Spanned::new(QUOTE_KEY.to_string(), span),
        Spanned::new(HValue::Str(quote.to_string()), span),
    );
    let mut b = UnitCoreBuilder::new(KernelType::Claim, quote, Status::Speculative);
    b.payload = object_to_payload(&obj);
    b.build().expect("a unit")
}

/// The library, and what one part of it holds: text, rows, structure, marks.
struct Read {
    _dir: Scratch,
    tid: Tid,
    text: String,
    rows: Vec<smysl_text::reading::Segment>,
    structure: smysl_text::structure::Structure,
    marks: Vec<Range<u64>>,
    manifest: smysl_core::types::Manifest,
}

fn read_usfm(tag: &str, source: &str) -> Read {
    let dir = Scratch::new(tag);
    let mut lib = Library::create(dir.path()).expect("a library");
    let added = lib
        .add(
            &Input::new(source.as_bytes()),
            &AddSpec::new("usfm/1", "matthew", "public-domain")
                .with_lang(LangTag::new("en").expect("a tag"))
                .with_carry(Carry::Text),
            &Caps::DEFAULT,
        )
        .expect("added");
    let manifest = lib
        .store()
        .manifest(&added.mid)
        .expect("a manifest")
        .clone();
    let tid = added.parts[0];
    let text = String::from_utf8(lib.part(&tid).expect("the part").text).expect("UTF-8");
    let rows = lib
        .part_reading(&added.mid, &tid)
        .expect("the reading")
        .rows;
    let structure = lib
        .structure(&added.mid, &tid, &Caps::DEFAULT)
        .expect("a tree");
    let marks = marks_of(&rows, &text);
    Read {
        _dir: dir,
        tid,
        text,
        rows,
        structure,
        marks,
        manifest,
    }
}

/// The `text:mode` a unit quoting `quote` comes out of attachment with: `Some(None)` for a
/// quote that attached and established no mode, `None` for one that did not attach at all.
fn mode_of(r: &Read, quote: &str) -> Option<Option<String>> {
    let panes = [Pane::new(r.tid, &r.text, &r.structure, &r.rows).with_reported(&r.marks)];
    let (ws, _) = windows(
        &panes,
        &Level::new("verse").expect("a level"),
        u32::MAX / 2,
        smysl_core::TokenEstimator::Utf8Div4,
        &smysl_text::Segmenter::neutral(),
    );
    let o = Opts {
        manifest: &r.manifest,
        normaliser: Normaliser::V1,
        lang: None,
    };
    for w in &ws {
        let out = attach(vec![unit_with(quote)], w, &o);
        if out.spanned == 0 {
            continue;
        }
        return Some(
            out.units[0]
                .payload
                .as_deref()
                .and_then(|p| payload_to_object(p).ok())
                .and_then(|o| {
                    o.get("text:mode")
                        .and_then(|v| v.value.as_str())
                        .map(str::to_string)
                }),
        );
    }
    None
}

/// The marker's words are `reported`; the narration beside them establishes nothing.
#[test]
fn a_claim_drawn_from_inside_wj_is_reported_and_one_beside_it_is_not() {
    let r = read_usfm("wj", MATTHEW);
    assert_eq!(
        r.marks.len(),
        1,
        "the reader marked the clause: {:?}",
        r.marks
    );
    assert_eq!(
        &r.text[r.marks[0].start as usize..r.marks[0].end as usize],
        "It is written, Man shall not live by bread alone."
    );

    assert_eq!(
        mode_of(&r, "Man shall not live by bread alone."),
        Some(Some("reported".to_string())),
        "a claim inside the marker is speech the text reports"
    );
    // Not `asserted`: §3.4 forbids that as a default, and a USFM verse carries no speaker who
    // could be asserting it. Nothing written is the honest record.
    assert_eq!(
        mode_of(&r, "But he answered and said,"),
        Some(None),
        "the narration has no structural evidence of a mode, so none is written"
    );
}

/// The same text without the marker: the gap step 3b found, now visible as a two-line diff.
///
/// This is the control the finding needs. The clause is the same clause, in the same edition,
/// at the same locator; the only difference is whether the source marked it — and before the
/// readers could see `\wj`, *this* was the answer for both.
#[test]
fn the_same_clause_unmarked_and_unpunctuated_establishes_nothing() {
    let plain = MATTHEW.replace("\\wj ", "").replace("\\wj*", "");
    let r = read_usfm("plain", &plain);
    assert!(
        r.marks.is_empty(),
        "nothing marks it and nothing punctuates it: {:?}",
        r.marks
    );
    assert_eq!(
        mode_of(&r, "Man shall not live by bread alone."),
        Some(None),
        "an edition that marks nothing supports no mode, which is the finding and not a defect"
    );
}

/// And a punctuating edition is still handled by the scan, with no marker at all.
#[test]
fn a_punctuated_quotation_is_still_found_by_the_scan() {
    let punctuated = "\\id MAT\n\\c 4\n\\p\n\\v 4 But he answered and said, \
                      \u{201c}It is written, Man shall not live by bread alone.\u{201d}\n";
    let r = read_usfm("punctuated", punctuated);
    assert_eq!(
        r.marks.len(),
        1,
        "no reader mark, so the punctuation scan supplied one: {:?}",
        r.marks
    );
    assert_eq!(
        mode_of(&r, "Man shall not live by bread alone."),
        Some(Some("reported".to_string()))
    );
}
