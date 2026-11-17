//! `smysl-tel`'s exact JSON reader (RFC SMYSL-3, TL-P2): no panic, and one canonical form.
//!
//! A snapshot is input from the system being observed, so the reader owes its caller what the
//! CBOR and surface readers owe theirs: anything it accepts it accepts without panicking, and
//! the canonical text of what it read reads back to the same canonical text. That second law is
//! the one a change unit's uid rests on: a `json` leaf is that text, and a text that moved when
//! read again would give one value two uids.
//!
//! The readers ride along (TL-P2 step 4): `k8s/1` and `zabbix/1` take exports from systems the
//! caller does not control, so whatever they are handed they refuse or read, without a panic,
//! and every object they read records without one.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_tel::profile::builtin;
use smysl_tel::readers::{k8s, zabbix};
use smysl_tel::snapshot::record;
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
    let pods = builtin("k8s-pods/1").expect("shipped");
    if let Ok(r) = k8s::read(text, &pods) {
        for o in &r.objects {
            let _ = record(o, None, &pods, k8s::ID, "unknown", None, 0);
        }
    }
    let items = builtin("zabbix-items/1").expect("shipped");
    if let Ok(r) = zabbix::read(text, &items, "7.0") {
        for o in &r.objects {
            let _ = record(o, None, &items, zabbix::ID, "unknown", None, 0);
        }
    }
});
