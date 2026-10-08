//! Every TX-P1 reader, over arbitrary bytes, under a budget.
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
//! The first byte picks the reader, so one corpus feeds all six and a finding names the reader
//! in its own input. A refusal is a pass: these readers are specified to refuse.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_text::limits::{Budget, Caps};
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
const READERS: &[&str] = &["txt/1", "usfm/1", "osis/1", "zefania/1", "json/1"];

fuzz_target!(|data: &[u8]| {
    let Some((choice, body)) = data.split_first() else {
        return;
    };
    let id = READERS[*choice as usize % READERS.len()];
    let Ok(reader) = readers::reader(id) else {
        return;
    };
    // A small input cap keeps the corpus small and the runs fast; it is not what is being
    // tested, and a refusal from it is as valid an outcome as any other.
    let caps = Caps {
        input_bytes: 1 << 20,
        ..Caps::DEFAULT
    };
    let Ok(mut budget) = Budget::new(caps, body.len() as u64) else {
        return;
    };
    let Ok(out) = readers::read_with(reader, &Input::new(body), &Params::new(), &mut budget) else {
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
});
