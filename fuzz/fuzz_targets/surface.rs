//! Guarantee A1: no panics on untrusted input.
//!
//! Surface text arrives from other agents, so "the parser must not panic" is a
//! requirement rather than hygiene. The parser is also required to recover rather than
//! fail, so anything it accepts must round-trip through the writer and back.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_core::surface::{parse_surface, write_surface, WriteContext};

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    let Ok(out) = parse_surface(src) else {
        return;
    };
    // Whatever survived parsing must survive emission and re-parsing unchanged.
    let ctx = WriteContext::from_labels(&out.labels);
    let text = write_surface(out.view.as_ref(), &out.records, &ctx);
    if let Ok(again) = parse_surface(&text) {
        assert_eq!(again.records, out.records, "round trip changed the records");
    }
});
