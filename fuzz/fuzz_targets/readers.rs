//! Every reader, over arbitrary bytes, under a budget.
//!
//! What this target asserts is not "no crash" — libfuzzer gets that for free — but the three
//! properties a reader's callers rely on and which a malformed input is the likeliest way to
//! break:
//!
//! 1. **The rows are a structure.** `Structure::build` is what every span, window and locator
//!    resolution goes through, so a reader that emits rows it refuses has produced a reading
//!    nothing downstream can open.
//! 2. **Every range is a character boundary of the text.** A range that cuts a character in
//!    half panics the moment anything slices the text with it, and that is not this crate.
//! 3. **Every locator resolves to its own node.** An address a reader emits and the structure
//!    cannot resolve points at nothing in a corpus.
//!
//! The first byte picks the reader, so one corpus feeds all of them and a finding names the
//! reader in its own input. A refusal is a pass: these readers are specified to refuse.
//!
//! # The chat readers, and the one of them that is a parser of a container
//!
//! TX-P2 step 2 added three. `whatsapp/1` needs its date order, so the first byte picks that
//! too — `choice / READERS.len() % 3` — rather than the target always passing `dmy` and never
//! reaching the other two orderings. `slack/1` is the first target input that is an **archive**,
//! which is the one shape where arbitrary bytes get nowhere on their own: a random string is
//! not a zip, so without a seeded corpus this target would spend its whole budget being
//! refused by the four-byte magic. `make seed-fuzz` therefore writes every reader fixture into
//! this target's corpus once per choice byte.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_text::limits::{Budget, Caps};
use smysl_text::part::{self, Policy};
use smysl_text::readers::{self, Input, Params};
use smysl_text::structure::Structure;

/// The readers this target drives.
///
/// **`md/1` is missing on purpose.** `pulldown-cmark` 0.13.4 panics inside the offset iterator
/// `md/1` needs (`parse.rs:2199`, a tight paragraph), and `smysl-text` contains that with
/// `catch_unwind` so the reader refuses instead of crashing — which `md/1`'s own test asserts.
/// It cannot be asserted *here*: `libfuzzer-sys` installs a panic hook that aborts the process
/// before unwinding, deliberately, so that a target cannot swallow a panic. Under that hook a
/// caught panic is still an abort, and this target would report the dependency's defect on
/// every run for as long as it is unfixed.
///
/// So the coverage is split, and the split is written down rather than left to be noticed:
/// five readers are fuzzed, and `md/1`'s containment is covered by
/// `an_input_that_panics_the_parser_is_a_refusal`. `md/1` comes back into this list with the
/// upstream fix, at the same commit that removes the `=0.13.4` pin and the wrapper.
const READERS: &[&str] = &[
    "txt/1",
    "usfm/1",
    "osis/1",
    "zefania/1",
    "json/1",
    "telegram/1",
    "whatsapp/1",
    "slack/1",
];

/// The date orders `whatsapp/1` accepts. Each is a different partition of the same transcript
/// into days, so each is a different reading and worth reaching.
const ORDERS: &[&str] = &["dmy", "mdy", "ymd"];

fuzz_target!(|data: &[u8]| {
    let Some((choice, body)) = data.split_first() else {
        return;
    };
    let id = READERS[*choice as usize % READERS.len()];
    let Ok(reader) = readers::reader(id) else {
        return;
    };
    // The parameters the chosen reader needs. Required ones only: an optional `tz` is covered
    // by the unit tests, and a target that varied everything would spend its corpus on
    // combinations rather than on inputs.
    let mut params = Params::new();
    if id == "whatsapp/1" {
        let order = ORDERS[(*choice as usize / READERS.len()) % ORDERS.len()];
        params
            .set(id, "date-format", order)
            .expect("a declared parameter");
    }
    // A small input cap keeps the corpus small and the runs fast; it is not what is being
    // tested, and a refusal from it is as valid an outcome as any other.
    let caps = Caps {
        input_bytes: 1 << 20,
        ..Caps::DEFAULT
    };
    let Ok(mut budget) = Budget::new(caps, body.len() as u64) else {
        return;
    };
    let Ok(out) = readers::read_with(reader, &Input::new(body), &params, &mut budget) else {
        // Refusing is the specified behaviour for input that is not the format.
        return;
    };

    let text = out.text.as_str();
    for row in &out.rows {
        let (start, end) = (row.start as usize, row.end as usize);
        assert!(
            end <= text.len(),
            "{id}: {} ends past the text ({end} > {})",
            row.locator,
            text.len()
        );
        assert!(start <= end, "{id}: {} is backwards", row.locator);
        assert!(
            text.is_char_boundary(start) && text.is_char_boundary(end),
            "{id}: {} does not land on character boundaries",
            row.locator
        );
    }

    let mut budget = Budget::new(Caps::DEFAULT, text.len() as u64).expect("a budget");
    let structure = Structure::build(&out.rows, text.len() as u64, &mut budget)
        .unwrap_or_else(|e| panic!("{id}: the reader emitted rows it cannot read back: {e}"));
    for row in &out.rows {
        assert_eq!(
            structure.resolve(&row.locator),
            Some(row.range()),
            "{id}: {} does not resolve to its own node",
            row.locator
        );
    }

    // Cutting the text into parts, which until now this target never did.
    //
    // The run that re-ran this after TX-P1 step 6 found nothing in fourteen million
    // executions, and then reading the target back showed why: it stopped here, over the whole
    // text, so none of the part-cutting arithmetic was reachable from it. Step 6's defect —
    // parts bounded by their nodes, dropping the separator at every cut and the text's head and
    // tail — was found by the first hand-written multi-part test instead, and would have been
    // found here years earlier if the target had gone this far.
    //
    // The policy is sized for fuzz inputs rather than for a Bible: the default `target_min` is
    // 64 KiB, so every input under that is one part, which is exactly the case that hid the
    // defect.
    let policy = Policy::new(out.top_level.clone(), 16, 64);
    let boundaries: Vec<std::ops::Range<u64>> =
        structure.at_level(&out.top_level).map(|n| n.range.clone()).collect();
    if boundaries.is_empty() {
        // A reader whose own top level names no node is a refusal in `Library::add`, not a
        // defect here.
        return;
    }
    let Ok(plans) = part::group(&boundaries, &policy, text.len() as u64, &mut budget) else {
        // A cap refusal is as valid an outcome as any other.
        return;
    };

    // **The parts partition the text.** Every byte is in exactly one part, including the ones
    // no node claims.
    assert!(!plans.is_empty(), "{id}: boundaries but no parts");
    assert_eq!(plans[0].range.start, 0, "{id}: the first part does not start at 0");
    assert_eq!(
        plans[plans.len() - 1].range.end,
        text.len() as u64,
        "{id}: the last part does not end at the text"
    );
    for pair in plans.windows(2) {
        assert_eq!(
            pair[0].range.end, pair[1].range.start,
            "{id}: a byte is in no part, or in two"
        );
    }

    for plan in &plans {
        // A part's rows are its own offsets, and a structure built from them has exactly the
        // part's length. This is the assertion the absolute-offset defect would have failed:
        // a part beginning at byte 70 whose rows still said 70 would build a table reaching
        // past its own end, and `Structure::build` would refuse it.
        let rows = part::local_rows(&out.rows, &plan.range);
        let mut part_budget = Budget::new(Caps::DEFAULT, plan.len()).expect("a budget");
        let part_structure = Structure::build(&rows, plan.len(), &mut part_budget)
            .unwrap_or_else(|e| panic!("{id}: a part's own rows are not a structure: {e}"));
        for row in &rows {
            assert_eq!(
                part_structure.resolve(&row.locator),
                Some(row.range()),
                "{id}: {} does not resolve inside its own part",
                row.locator
            );
        }

        // And the part's bytes are a normalised slice, or cutting it is refused. `text_of`
        // returning `None` is the refusal `Library::add` turns into an error, so it is an
        // outcome rather than a failure — what must not happen is a part whose text exists and
        // does not hash to the tid the manifest would record for it.
        if let Some(part_text) = part::text_of(&out.text, plan.range.clone()) {
            assert!(
                part_text.verify(),
                "{id}: a part's bytes do not hash to its own tid"
            );
            assert_eq!(
                part_text.text.len() as u64,
                plan.len(),
                "{id}: a part's length disagrees with its range"
            );
        }
    }
});
