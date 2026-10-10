//! `date`: §19.2's clock fault through the binary (SMYSL-2.4 §5.2, TX-P3 step 4).
//!
//! Two messages, one of which was sent by a device whose clock ran ninety-three seconds ahead.
//! The reply quotes the first message, so the corpus says the reply came second and the clocks
//! say it came first — `SMY-W413`, and nothing is chosen for either unit. One dating with a
//! window target and an `offset` corrects exactly the messages in the window, the contradiction
//! goes, and no uid moves: a dating stands *beside* the unit, because correcting the unit would
//! change what it is.
//!
//! The same dating with nothing behind it is `speculative` (D-4) and is reported rather than
//! applied, which is the whole mechanism of rule E in one pair of invocations: what decides
//! whether a correction takes effect is the evidence named for it, not who typed it.
//!
//! The last scenario is the liveness lock. A `canonical` commitment on a dating holds it, and a
//! resolution naming the contention the hold derives releases it — a contention that is
//! **derived and never written** (A-8.2), so neither `review` nor the log has it, and both the
//! diagnostic that names it and the `resolve` that accepts it had to learn about it here.

#![cfg(all(feature = "cli", feature = "text"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

/// A canonical tid, borrowed from the wire fixtures. Nothing reads the part; what the tid is for
/// is to put two units in the same part so a window target can select between them.
const TID: &str = "t3:2ruhul6f4o3a735poauxd3snjdxdzol44imwk7gfse5jed2rb5ra";

/// The instant the skewed message records, and the one it should have.
const SKEWED: u64 = 1_700_000_093_000;
const TRUE_AT: u64 = 1_700_000_000_000;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smysl-date-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary under test must run")
}

fn out(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The document: two messages in one part, the second quoting the first.
///
/// `@rel` and not a bare arrow: an arrow line is read as a reference from the body, which rule C
/// then wants in `deps` — the dep is there, and the ordering constraint rule E reads comes from
/// the `x.text/quotes` relation rather than from the dep.
fn document(dir: &Path) -> PathBuf {
    let p = dir.join("chat.smy");
    std::fs::write(
        &p,
        format!(
            "@doc smysl/0.1 {{ id: v/clock-fault }}\n\
             \n\
             @claim c/m1 {{ status: cited, source: {{ kind: doc, ref: \"{TID}#1\", \
             observed: {SKEWED} }} }}\n\
             ~ The first message, sent by the device whose clock ran ninety-three seconds ahead.\n\
             \n\
             @claim c/m2 {{ status: cited, deps: [c/m1], source: {{ kind: doc, \
             ref: \"{TID}#2\", observed: {TRUE_AT} }} }}\n\
             ~ The reply, which quotes the first message and was therefore sent after it was sent.\n\
             \n\
             @rel c/m2 --x.text/quotes--> c/m1\n",
        ),
    )
    .expect("the fixture document");
    p
}

/// How the two stores partition by uid, which is the question "did any uid move" exactly.
///
/// `diff` and not a scrape of `check`'s output: the first version of this helper collected every
/// `b3:` string a `check` printed, which is mostly the *diagnostics* — so it changed when the
/// correction removed the `SMY-W413` that mentioned the units, and reported that uids had moved
/// when none had. The test was measuring its own subject's error messages.
fn partition(before: &str, after: &str) -> String {
    let o = run(&["diff", before, after]);
    assert!(o.status.success(), "{}", out(&o));
    out(&o).trim().to_string()
}

fn codes(store: &str) -> String {
    out(&run(&["check", "-s", store, "--pass", "time"]))
}

#[test]
fn a_window_dating_retimes_exactly_the_messages_in_the_window() {
    let dir = scratch("window");
    let doc = document(&dir);
    let store = doc.to_str().unwrap();

    let before = codes(store);
    assert!(
        before.contains("SMY-W413"),
        "the clocks and the quotation must contradict each other to begin with: {before}"
    );
    let untouched = dir.join("before.smy");
    std::fs::copy(&doc, &untouched).expect("a copy to compare against");

    // The window selects the first message and not the reply: the skewed instant is inside it
    // and the true one is not. `--basis c/m2` is what gives the correction a status — the
    // reply is `cited`, so the dating is `cited`, which is enough to move a `cited` bound.
    let o = run(&[
        "date",
        "set",
        TID,
        "-s",
        store,
        "--window",
        "1700000050000..1700000100000",
        "--value",
        "offset:-93000",
        "--as",
        "human:vu",
        "--at",
        "1700001000000",
        "--basis",
        "c/m2",
    ]);
    assert!(o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("selects 1 subject"),
        "exactly the messages in the window, which is one of the two: {}",
        out(&o)
    );

    let after = codes(store);
    assert!(
        !after.contains("SMY-W413"),
        "the correction must settle the contradiction: {after}"
    );
    assert!(
        !after.contains("SMY-W412"),
        "and must itself have been applied: {after}"
    );
    assert!(
        partition(untouched.to_str().unwrap(), store).ends_with("0 only, 0 only, 2 common"),
        "a dating stands beside the unit, so no uid may move: {}",
        partition(untouched.to_str().unwrap(), store)
    );

    // The corrected instant, and the chain that says where it came from.
    let shown = out(&run(&["date", "show", "c/m1", "-s", store, "--why"]));
    assert!(
        shown.contains("2023-11-14T22:13:20Z"),
        "the first message is now at the reply's own second: {shown}"
    );
    assert!(
        shown.contains("offset d3:"),
        "--why must name the dating that moved the bound: {shown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_same_dating_with_nothing_behind_it_is_reported_and_not_applied() {
    let dir = scratch("unevidenced");
    let doc = document(&dir);
    let store = doc.to_str().unwrap();

    // No `--basis`, so `speculative` (D-4) however confident its author.
    let o = run(&[
        "date",
        "set",
        TID,
        "-s",
        store,
        "--window",
        "1700000050000..1700000100000",
        "--value",
        "offset:-93000",
        "--as",
        "human:vu",
        "--at",
        "1700001000000",
    ]);
    assert!(o.status.success(), "{}", out(&o));

    let after = codes(store);
    assert!(
        after.contains("SMY-W412"),
        "an unevidenced correction must be reported: {after}"
    );
    assert!(
        after.contains("SMY-W413"),
        "and must not have settled the contradiction: {after}"
    );
    let shown = out(&run(&["date", "show", "c/m1", "-s", store, "--why"]));
    assert!(
        shown.contains("as recorded"),
        "the bound must still be the one the record carries: {shown}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn order_is_sugar_for_a_relative_dating() {
    let dir = scratch("order");
    let doc = document(&dir);
    let store = doc.to_str().unwrap();

    let o = run(&[
        "date",
        "order",
        "c/m1",
        "before",
        "c/m2",
        "-s",
        store,
        "--as",
        "human:vu",
        "--at",
        "1700001000000",
        "--basis",
        "c/m2",
    ]);
    assert!(o.status.success(), "{}", out(&o));
    let written = std::fs::read_to_string(&doc).expect("the document");
    assert!(
        written.contains("@date c/m1 { axis: said, before: c/m2"),
        "`order` writes the relative dating `set` would have: {written}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_canonical_commitment_holds_a_dating_until_a_resolution_names_it() {
    let dir = scratch("hold");
    let doc = document(&dir);
    // A CBOR store, because a commitment naming a **did** has no surface spelling: `@commit`
    // takes a uid or a label, which is OQ-73. The refusal is correct and this is the way round
    // it, not a workaround for a defect.
    let cbor = dir.join("store.cbor");
    let store = cbor.to_str().unwrap();
    let o = run(&["merge", doc.to_str().unwrap(), "-o", store]);
    assert!(o.status.success(), "{}", out(&o));

    let first = did_of(&run(&[
        "date",
        "set",
        "c/m1",
        "-s",
        store,
        "--value",
        "1066",
        "--as",
        "human:vu",
        "--at",
        "1700001000000",
        "--basis",
        "c/m2",
        "--json",
    ]));
    let o = run(&[
        "commit",
        &first,
        store,
        "--level",
        "canonical",
        "--as",
        "human:vu",
        "--at",
        "1700002000000",
    ]);
    assert!(o.status.success(), "{}", out(&o));

    // A second, different dating of the same target. The lock holds it.
    let second = did_of(&run(&[
        "date",
        "set",
        "c/m1",
        "-s",
        store,
        "--value",
        "1067",
        "--as",
        "human:vu",
        "--at",
        "1700003000000",
        "--basis",
        "c/m2",
        "--json",
    ]));
    let held = codes(store);
    // A diagnostic prints a did **short** — the prefix and 26 characters — where `--json` gives
    // the canonical 52. Comparing the two spellings is what the first version of this assertion
    // did, and it failed on a store that was behaving correctly.
    assert!(
        held.contains(&second[..smysl::Did::PREFIX.len() + 26])
            && held.contains("held by a canonical commitment"),
        "the later dating must be held: {held}"
    );

    // The id the hold derives, named in the diagnostic because nothing else can show it: rule
    // E's contentions are derived and never written, so `review` does not list them.
    let id = held
        .split_whitespace()
        .find(|w| w.starts_with("(k/c"))
        .map(|w| w.trim_matches(|c| c == '(' || c == ')').to_string())
        .unwrap_or_else(|| panic!("the diagnostic must name a contention id: {held}"));

    let o = run(&[
        "resolve",
        &id,
        store,
        "--as",
        "human:vu",
        "--at",
        "1700004000000",
    ]);
    assert!(o.status.success(), "{}", out(&o));
    let released = codes(store);
    assert!(
        !released.contains(&id),
        "a resolution naming the derived contention releases the hold: {released}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn did_of(o: &Output) -> String {
    assert!(o.status.success(), "{}", out(o));
    let text = String::from_utf8_lossy(&o.stdout);
    let i = text
        .find("d3:")
        .unwrap_or_else(|| panic!("a did in {text}"));
    let rest = &text[i..];
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphanumeric() && *c != ':')
        .map_or(rest.len(), |(i, _)| i);
    rest[..end].to_string()
}

#[test]
fn a_malformed_date_is_refused_at_the_writing_end() {
    let dir = scratch("malformed");
    let doc = document(&dir);
    let store = doc.to_str().unwrap();
    let o = run(&[
        "date", "set", "c/m1", "-s", store, "--value", "1984-6-2", "--as", "human:vu",
    ]);
    assert!(!o.status.success(), "a second spelling must be refused");
    assert!(
        out(&o).contains("SMY-E410"),
        "and named as the code the check pass would raise: {}",
        out(&o)
    );
    assert_eq!(
        std::fs::read_to_string(&doc)
            .expect("the document")
            .matches("@date")
            .count(),
        0,
        "nothing may have been written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_window_is_refused_rather_than_written() {
    let dir = scratch("empty-window");
    let doc = document(&dir);
    let store = doc.to_str().unwrap();
    let o = run(&[
        "date",
        "set",
        TID,
        "-s",
        store,
        "--window",
        "1700000100000..1700000050000",
        "--value",
        "offset:-1",
        "--as",
        "human:vu",
    ]);
    assert!(!o.status.success(), "{}", out(&o));
    assert!(
        out(&o).contains("selects nothing"),
        "the message has to say why: {}",
        out(&o)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
