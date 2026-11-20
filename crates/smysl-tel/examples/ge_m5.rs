//! GE-M5 (RFC SMYSL-3 §10.4, §26): does causal order catch clock skew? On simulated traces.
//!
//! As fixed in §26 before the run: one host's clock is skewed by ±1 … ±500 ms, in turn for every
//! host, over 300 traces; and twenty runs have no skew. Each run goes through `otlp-traces/1`'s
//! path and is materialised, and the `x.tel/precedes` edges are checked at the unit level, as
//! rule E would check them:
//!
//! - **arm start** (§10.4 as written): an edge is violated when the child's `start_ms` is below
//!   its parent's;
//! - **arm start+end**: also when the child's `end_ms` is above its parent's, the response
//!   constraint a span unit can state and rule E's one instant cannot.
//!
//! A skew is **detected** when the skewed host is the one host touching the most violated
//! edges. A no-skew run with any violated edge is a **false alarm**. The stop condition is the
//! RFC's: detection below 90 % at ≥ 5 ms keeps the constraint out of rule E.
//!
//! **Post hoc, and labelled so:** a skew is **noticed** when an edge touching the skewed host is
//! violated by 2 ms or more, which the ±0.25 ms residual offsets cannot do at millisecond
//! resolution. It asks whether the constraint sees the skew at all, where the pre-registered
//! measure also asks it to name the host, which a pair of clocks cannot say alone.
//!
//! `cargo run --release -p smysl-tel --example ge_m5 [-- --traces N --seed S]`

#[path = "common/traces.rs"]
mod common;

use std::collections::BTreeMap;

use smysl_series::{MemBackend, Substrate};
use smysl_tel::trace;

use common::{offsets, otlp, traces, Rng, SERVICES, T0_NS};

const SKEWS_MS: [i64; 9] = [1, 2, 5, 10, 20, 50, 100, 200, 500];

fn arg(name: &str, default: u64) -> u64 {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn uint_field(p: &[u8], key: &str) -> Option<u64> {
    let mut d = smysl_core::cbor::reader::Dec::new(p);
    let n = d.map_head().ok()?;
    for _ in 0..n {
        if d.text().ok()? == key {
            return d.uint().ok();
        }
        d.skip_item().ok()?;
    }
    None
}

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

/// What one run's units say: per arm, violated edges by host.
struct Run {
    /// [arm start, arm start+end]: violated edges touching each host.
    by_host: [Vec<usize>; 2],
    violated: [usize; 2],
    edges: usize,
    /// [arm start, arm start+end]: each host's largest violation touching it, ms.
    worst: [Vec<u64>; 2],
}

fn run(seed: u64, n: usize, skewed: Option<(usize, i64)>) -> Run {
    let mut r = Rng(seed);
    let offs = offsets(&mut r);
    let sim = traces(&mut r, T0_NS, n, 200.0);
    let mut skew = vec![0i64; SERVICES.len()];
    if let Some((h, ms)) = skewed {
        skew[h] = ms * 1_000_000;
    }
    let spans = otlp(&sim, &offs, &skew);
    let rec = trace::record(&spans, "collector", "otel-collector:sim").expect("a read");
    let mut sub = Substrate::new(MemBackend::default());
    for st in &rec.streams {
        sub.add_events(&st.uid, &rec.inst, &st.info, &st.events, &trace::tag)
            .expect("events");
    }
    let host: BTreeMap<&str, usize> = SERVICES.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut out = Run {
        by_host: [vec![0; SERVICES.len()], vec![0; SERVICES.len()]],
        violated: [0, 0],
        edges: 0,
        worst: [vec![0; SERVICES.len()], vec![0; SERVICES.len()]],
    };
    for tr in &sim {
        let m = trace::materialise(&sub, &tr.id).expect("materialised");
        let facts = |u: &smysl_core::Uid| {
            let c = &m.units[u];
            let p = c.payload.as_deref().expect("a payload");
            (
                uint_field(p, "tel:start_ms").expect("start"),
                uint_field(p, "tel:end_ms").expect("end"),
                host[text_field(p, "tel:service").expect("a service").as_str()],
            )
        };
        for e in &m.relations {
            let (ps, pe, ph) = facts(&e.from);
            let (cs, ce, ch) = facts(&e.to);
            out.edges += 1;
            let by_start = ps.saturating_sub(cs);
            let by_end = ce.saturating_sub(pe);
            for (arm, by) in [(0, by_start), (1, by_start.max(by_end))] {
                if by > 0 {
                    out.violated[arm] += 1;
                    out.by_host[arm][ph] += 1;
                    if ch != ph {
                        out.by_host[arm][ch] += 1;
                    }
                    for h in [ph, ch] {
                        out.worst[arm][h] = out.worst[arm][h].max(by);
                    }
                }
            }
        }
    }
    out
}

/// The one host touching the most violated edges, if there is one.
fn blamed(by_host: &[usize]) -> Option<usize> {
    let max = *by_host.iter().max()?;
    if max == 0 || by_host.iter().filter(|c| **c == max).count() > 1 {
        return None;
    }
    by_host.iter().position(|c| *c == max)
}

fn main() {
    let n = arg("--traces", 300) as usize;
    let seed = arg("--seed", 5);
    let started = std::time::Instant::now();
    println!("GE-M5 (simulated): does causal order catch clock skew?");
    println!(
        "  {} hosts, {n} traces a run, skews ±{SKEWS_MS:?} ms, seed {seed}",
        SERVICES.len()
    );

    // No skew: false alarms.
    let mut alarms = [0usize; 2];
    let mut loud = [0usize; 2];
    let mut base_viol = [0usize; 2];
    let mut base_edges = 0;
    for k in 0..20 {
        let r = run(seed * 1_000 + k, n, None);
        base_edges += r.edges;
        for arm in 0..2 {
            base_viol[arm] += r.violated[arm];
            if r.violated[arm] > 0 {
                alarms[arm] += 1;
            }
            if r.worst[arm].iter().any(|w| *w >= 2) {
                loud[arm] += 1;
            }
        }
    }
    println!(
        "  no skew           20 runs, {base_edges} edges: violated {} / {} (start / start+end); \
         false alarms {} / {} runs; violated by >= 2 ms in {} / {} runs",
        base_viol[0], base_viol[1], alarms[0], alarms[1], loud[0], loud[1]
    );

    // Skew, by magnitude: detections out of 2 × hosts runs.
    let mut at5 = [0usize; 2];
    let mut noticed5 = [0usize; 2];
    let mut at5_runs = 0usize;
    let mut unseen: BTreeMap<(usize, &str), usize> = BTreeMap::new();
    let mut misses: BTreeMap<(usize, &str), usize> = BTreeMap::new();
    println!(
        "  skew ms    detected (start)   detected (start+end)   noticed (start)   noticed (s+e)"
    );
    for (i, ms) in SKEWS_MS.iter().enumerate() {
        let mut det = [0usize; 2];
        let mut seen = [0usize; 2];
        let mut runs = 0;
        for h in 0..SERVICES.len() {
            for sign in [1i64, -1] {
                let r = run(
                    seed * 1_000_000 + (i as u64) * 1_000 + (h as u64) * 2 + (sign > 0) as u64,
                    n,
                    Some((h, sign * ms)),
                );
                runs += 1;
                for arm in 0..2 {
                    if r.worst[arm][h] >= 2 {
                        seen[arm] += 1;
                    } else if arm == 0 && *ms >= 5 {
                        *unseen
                            .entry((h, if sign > 0 { "ahead" } else { "behind" }))
                            .or_default() += 1;
                    }
                    if blamed(&r.by_host[arm]) == Some(h) {
                        det[arm] += 1;
                    } else if arm == 0 && *ms >= 5 {
                        *misses
                            .entry((h, if sign > 0 { "ahead" } else { "behind" }))
                            .or_default() += 1;
                    }
                }
            }
        }
        if *ms >= 5 {
            at5_runs += runs;
            at5[0] += det[0];
            at5[1] += det[1];
            noticed5[0] += seen[0];
            noticed5[1] += seen[1];
        }
        let p = |k: usize| 100.0 * k as f64 / runs as f64;
        println!(
            "  {ms:>7}    {:>3} / {runs} {:>5.1} %    {:>3} / {runs} {:>5.1} %        {:>5.1} %           \
             {:>5.1} %",
            det[0],
            p(det[0]),
            det[1],
            p(det[1]),
            p(seen[0]),
            p(seen[1])
        );
    }
    let rate = |d: usize| 100.0 * d as f64 / at5_runs as f64;
    println!(
        "  at >= 5 ms        detected: start {:.1} %, start+end {:.1} %; noticed (post hoc): \
         start {:.1} %, start+end {:.1} % ({at5_runs} runs each)",
        rate(at5[0]),
        rate(at5[1]),
        rate(noticed5[0]),
        rate(noticed5[1])
    );
    if !unseen.is_empty() {
        println!("  arm start leaves unnoticed at >= 5 ms:");
        for ((h, dir), k) in &unseen {
            println!("    {:<14} {dir:<7} {k} of 7", SERVICES[*h]);
        }
    }
    if !misses.is_empty() {
        println!("  arm start misses at >= 5 ms, by host and direction:");
        let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for ((h, dir), k) in &misses {
            by.entry(dir.to_string())
                .or_default()
                .push(format!("{} ({k})", SERVICES[*h]));
        }
        for (dir, hosts) in by {
            println!("    {dir:<7} {}", hosts.join(", "));
        }
    }
    let verdict = |d: usize| {
        if rate(d) >= 90.0 {
            "passes"
        } else {
            "fails"
        }
    };
    println!(
        "  verdict           arm start {} the 90 % bar; arm start+end {}",
        verdict(at5[0]),
        verdict(at5[1])
    );
    println!(
        "  run               {:.1} s; simulated data",
        started.elapsed().as_secs_f64()
    );
}
