//! `smysl-tel`'s exact JSON reader (RFC SMYSL-3, TL-P2): no panic, and one canonical form.
//!
//! A snapshot is input from the system being observed, so the reader owes its caller what the
//! CBOR and surface readers owe theirs: anything it accepts it accepts without panicking, and
//! the canonical text of what it read reads back to the same canonical text. That second law is
//! the one a change unit's uid rests on: a `json` leaf is that text, and a text that moved when
//! read again would give one value two uids.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_tel::{jcs, json};

fuzz_target!(|data: &[u8]| {
    let Ok(text) = core::str::from_utf8(data) else {
        return;
    };
    if let Ok(v) = json::parse(text) {
        let (canon, verbatim) = jcs::canonical(&v);
        let again = json::parse(&canon).expect("canonical JSON is JSON");
        let (canon2, _) = jcs::canonical(&again);
        if !verbatim {
            assert_eq!(canon, canon2, "the canonical form moved when read again");
        }
    }
    let _ = json::parse_lines(text);
});
