//! `otlp/1` and `otlp-json/1` (RFC SMYSL-3 §20.1, TL-P3 step 3): an export is a pusher's bytes,
//! read or refused without a panic, and what is read records against an empty substrate without
//! one.
//!
//! The bytes go to the protobuf reader as they are, and to the JSON reader when they are UTF-8.
//! Each export read is also recorded twice over, the second time against the first's samples,
//! so the resets and stales a later export decides are reached as well as the first sight.
//!
//! Then every series recorded is materialised as window evidence (TL-P4), exact and sketched,
//! over all time: the summaries, the exact sum and mean, and the sketch see whatever values an
//! export can carry. A window is materialised twice and must have one uid.
//!
//! The bytes are also read as a window payload, as `check`'s rule TW pass reads one from a
//! store it did not write.

#![no_main]
use libfuzzer_sys::fuzz_target;
use smysl_series::{MemBackend, Substrate};
use smysl_tel::otel::{self, SCHEME};
use smysl_tel::readers::otlp::{self, Export};
use smysl_tel::evidence::{self, DEFAULT_QS};
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
    let qs: Vec<smysl_core::Decimal> = DEFAULT_QS
        .iter()
        .map(|q| smysl_core::Decimal::parse_token(q).expect("a rank"))
        .collect();
    let alpha = smysl_core::Decimal::parse_token(evidence::DEFAULT_ALPHA).expect("an alpha");
    for (series, _) in sub.series(None).unwrap_or_default() {
        if let Ok(a) = evidence::materialise(&sub, &series, &inst, 0, u64::MAX) {
            let b = evidence::materialise(&sub, &series, &inst, 0, u64::MAX).expect("again");
            assert_eq!(
                smysl_core::canonical_uid(&a.unit),
                smysl_core::canonical_uid(&b.unit),
                "one window, one uid"
            );
        }
        let _ = evidence::materialise_sketched(&sub, &series, &inst, 0, u64::MAX, alpha, &qs);
    }
}

fuzz_target!(|data: &[u8]| {
    // A window payload as `check` reads it from a store (TL-P4 step 3): read or refused.
    if let Ok(w) = smysl_core::schemas::tel::window_fields(data) {
        let _ = smysl_core::schemas::tel::rule_tw(&w);
    }
    // And as a payload a rule TQ quote reads a number from, and as a quote (TL-P6 step 2).
    for (field, index) in [("max", None), ("q", Some(1)), ("count", None)] {
        if let Some(v) = smysl_core::schemas::tel::quoted_value(data, field, index) {
            assert!(!v.is_empty(), "a number has a spelling");
        }
    }
    if let Ok(text) = core::str::from_utf8(data) {
        let _ = smysl_core::schemas::tel::parse_quote(text);
    }
    if let Ok(e) = otlp::read_proto(data) {
        twice(&e);
    }
    if let Ok(text) = core::str::from_utf8(data) {
        if let Ok(e) = otlp::read_json(text) {
            twice(&e);
        }
    }
});
