//! GE-M2 (RFC SMYSL-3 §26, D-T5): is a millisecond enough? On simulated traces.
//!
//! The traces ([`common::traces`]) go through `otlp-traces/1`'s path: OTLP spans, recorded as
//! events, materialised as `x.tel/span` units. Then, as fixed in §26 before the run:
//!
//! - **sub-ms**: the share of spans shorter than 1 ms;
//! - **affected**: the share of spans with a sibling (same parent) that ran strictly before or
//!   after it while the two units' `start_ms` are equal. Their order is in the substrate's
//!   nanoseconds and not in any unit. The run also checks that the substrate does order every
//!   such pair as it ran.
//!
//! The stop condition is the RFC's: under 1 % affected releases `source` key 7. The data is
//! simulated, which the report says; DeathStarBench at production load is still owed.
//!
//! `cargo run --release -p smysl-tel --example ge_m2 [-- --traces N --seed S]`

#[path = "common/traces.rs"]
mod common;

use std::collections::BTreeMap;

use smysl_series::{MemBackend, Substrate};
use smysl_tel::readers::otlp_traces;
use smysl_tel::trace;

use common::{offsets, otlp, traces, Rng, SERVICES, T0_NS};

fn arg(name: &str, default: u64) -> u64 {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn pct(a: usize, b: usize) -> f64 {
    100.0 * a as f64 / b.max(1) as f64
}

fn main() {
    let n = arg("--traces", 5_000) as usize;
    let seed = arg("--seed", 2);
    let started = std::time::Instant::now();
    let mut r = Rng(seed);
    let offs = offsets(&mut r);
    let sim = traces(&mut r, T0_NS, n, 200.0);
    let spans = otlp(&sim, &offs, &vec![0; SERVICES.len()]);

    // The reading path.
    let rec = trace::record(&spans, "collector", "otel-collector:sim").expect("a read");
    let mut sub = Substrate::new(MemBackend::default());
    for st in &rec.streams {
        sub.add_events(&st.uid, &rec.inst, &st.info, &st.events, &trace::tag)
            .expect("events");
    }
    // Each span's recorded nanoseconds, as the substrate holds them.
    let mut body: BTreeMap<[u8; 8], (u64, u64)> = BTreeMap::new();
    for st in &rec.streams {
        let read = sub.events(&st.uid, &rec.inst, 0, u64::MAX).expect("a read");
        for e in &read.events {
            let b = otlp_traces::decode_body(&e.body).expect("a span body");
            body.insert(b.span_id, (b.start_ns, b.end_ns));
        }
    }

    let (mut total, mut sub_ms, mut affected_n, mut pairs, mut pairs_store) = (0, 0, 0, 0, 0);
    let mut recovered = 0;
    let mut by_name: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for tr in &sim {
        let m = trace::materialise(&sub, &tr.id).expect("materialised");
        // span id → start_ms, from the units.
        let mut start_ms: BTreeMap<String, u64> = BTreeMap::new();
        for u in m.units.values() {
            let p = u.payload.as_deref().expect("a payload");
            let id = text_field(p, "tel:span_id").expect("a span id");
            start_ms.insert(
                id,
                u.source
                    .as_ref()
                    .and_then(|s| s.observed)
                    .expect("observed"),
            );
        }
        assert_eq!(m.units.len(), tr.spans.len(), "every span materialises");
        let mut children: BTreeMap<[u8; 8], Vec<usize>> = BTreeMap::new();
        for (i, s) in tr.spans.iter().enumerate() {
            total += 1;
            if s.end - s.start < 1_000_000 {
                sub_ms += 1;
            }
            if let Some(p) = s.parent {
                children.entry(p).or_default().push(i);
            }
        }
        let mut affected = vec![false; tr.spans.len()];
        for kids in children.values() {
            for (x, &a) in kids.iter().enumerate() {
                for &b in &kids[x + 1..] {
                    let (sa, sb) = (&tr.spans[a], &tr.spans[b]);
                    let sequential = sa.end <= sb.start || sb.end <= sa.start;
                    let ms = |s: &common::SimSpan| start_ms[&otlp_traces::hex(&s.id)];
                    if sequential && ms(sa) == ms(sb) {
                        affected[a] = true;
                        affected[b] = true;
                        pairs += 1;
                        if !sa.name.ends_with(" call") && !sb.name.ends_with(" call") {
                            pairs_store += 1;
                        }
                        // The substrate orders them as they ran.
                        let (ba, bb) = (body[&sa.id], body[&sb.id]);
                        let truth = sa.start < sb.start;
                        if (ba.0 < bb.0) == truth {
                            recovered += 1;
                        }
                    }
                }
            }
        }
        for (i, s) in tr.spans.iter().enumerate() {
            let e = by_name.entry(s.name.clone()).or_default();
            e.0 += 1;
            if affected[i] {
                e.1 += 1;
                affected_n += 1;
            }
        }
    }

    let share = pct(affected_n, total);
    println!("GE-M2 (simulated): is a millisecond enough?");
    println!("  traces            {n}, seed {seed}");
    println!("  spans             {total}, all materialised from the substrate");
    println!("  sub-ms spans      {sub_ms} ({:.1} %)", pct(sub_ms, total));
    println!(
        "  affected spans    {affected_n} ({share:.2} %): a sequential sibling in the same ms"
    );
    println!(
        "  collapsed pairs   {pairs}, {pairs_store} of them between two store calls; the \
         substrate orders {recovered} of {pairs} as they ran"
    );
    println!("  by span name (affected / all):");
    for (name, (all, aff)) in &by_name {
        if *aff > 0 {
            println!(
                "    {name:<22} {aff:>6} / {all:<6} {:>5.1} %",
                pct(*aff, *all)
            );
        }
    }
    let verdict = if share < 1.0 {
        "under 1 %: key 7 would be released"
    } else {
        "1 % or more: by the stop condition key 7 is not released"
    };
    println!("  verdict           {verdict}");
    println!(
        "  run               {:.1} s; simulated data: DeathStarBench at load is owed",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(
        recovered, pairs,
        "the substrate keeps every order a unit loses"
    );
}

/// A text field of a payload.
fn text_field(p: &[u8], key: &str) -> Option<String> {
    let mut d = smysl_core::cbor::reader::Dec::new(p);
    let n = d.map_head().ok()?;
    for _ in 0..n {
        if d.text().ok()? == key {
            return d.text().ok().map(str::to_string);
        }
        d.skip_item().ok()?;
    }
    None
}
