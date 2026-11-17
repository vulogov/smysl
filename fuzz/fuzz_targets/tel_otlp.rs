//! `otlp/1` and `otlp-json/1` (RFC SMYSL-3 §20.1, TL-P3 step 3): an export is a pusher's bytes,
//! read or refused without a panic, and what is read records against an empty substrate without
//! one.
//!
//! The bytes go to the protobuf reader as they are, and to the JSON reader when they are UTF-8.
//! Each export read is also recorded twice over, the second time against the first's samples,
//! so the resets and stales a later export decides are reached as well as the first sight.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_series::{MemBackend, Substrate};
use smysl_tel::otel::{self, SCHEME};
use smysl_tel::readers::otlp::{self, Export};
use smysl_tel::scrape::SubstrateHistory;

fn twice(e: &Export) {
    let mut sub = Substrate::new(MemBackend::default());
    let key = "otel-collector:f";
    let inst = smysl_core::canonical_uid(
        &smysl_core::schemas::tel::instrument("collector", key).expect("a valid instrument"),
    );
    for _ in 0..2 {
        let r = {
            let h = SubstrateHistory {
                substrate: &sub,
                instrument: inst,
                scheme: SCHEME,
            };
            otel::record(e, "collector", key, &h)
        };
        let Ok(r) = r else { return };
        for ss in &r.series {
            let _ = sub.add(&ss.uid, &r.inst, &ss.info, &ss.samples);
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if let Ok(e) = otlp::read_proto(data) {
        twice(&e);
    }
    if let Ok(text) = core::str::from_utf8(data) {
        if let Ok(e) = otlp::read_json(text) {
            twice(&e);
        }
    }
});
