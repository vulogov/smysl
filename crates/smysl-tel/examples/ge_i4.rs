//! GE-I4 (RFC SMYSL-5 §15, TI-P3): is template entropy a property of the logs, or of Drain?
//!
//! Drain builds its templates from the messages it is given, in order, so the templates of a
//! window, and with them `tent/1`, could depend on where the window starts as much as on what
//! the service is doing. This asks how much.
//!
//! **What this runs.** One simulated log stream, and the report says so: seven days of a
//! service writing about ten messages a minute at random instants, from eight message shapes
//! with variable fields (addresses, ids, durations, hex keys). Every six hours a fault of 90
//! minutes, alternating two kinds:
//!
//! - **errors**: three error shapes join the mix, a fifth of the messages;
//! - **mix**: no new shape, but cache misses and retries rise from a tenth to half of it.
//!
//! Windows are 30 minutes on a 30-minute grid. Each is computed as it is and with both
//! boundaries shifted by a tenth of its length, earlier and later, by
//! `smysl_tel::drain::run` and `smysl_tel::info::logs::compute`, the code that writes units.
//! A window is **normal** or **faulty** only when it and both its shifts lie inside one regime,
//! so a shift never crosses into the other: what moves the value is the boundary alone.
//!
//! **A control.** The same plug-in over the shapes the simulator wrote each message from, which
//! no template miner can make or break, under the same shifts: its move is what sampling alone
//! does, and `tent/1`'s move less it is Drain's share.
//!
//! **The stop condition** (§15): "if shifting a boundary moves the value more than the median
//! difference between normal and faulty windows, `tent/1` does not ship and TI-P3 closes with
//! that result." Read here as fixed before the run: the **median** move of a shift, over every
//! normal and faulty window and both shifts, against the difference between the faulty windows'
//! median value and the normal windows', per fault kind. The 95th percentile and the largest
//! move are reported beside it, and judged in a second line that is not the stop condition.
//!
//! ```text
//! cargo run --release -p smysl-tel --example ge_i4 [-- --days 7 --seed 7]
//! ```

// The simulator draws its instants and shapes with ChaCha20 and plain arithmetic; nothing the
// estimator stores comes from a transcendental of its own.

use std::collections::BTreeMap;

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use smysl_core::Uid;
use smysl_tel::drain;
use smysl_tel::info::logs::{compute, Templates};
use smysl_tel::info::{Spec, Stream};

const MIN: u64 = 60_000;
const WINDOW: u64 = 30 * MIN;
const SHIFT: u64 = WINDOW / 10;
const FAULT_EVERY: u64 = 6 * 60 * MIN;
const FAULT_LEN: u64 = 90 * MIN;

struct Sim(ChaCha20Rng);

impl Sim {
    fn below(&mut self, n: u64) -> u64 {
        self.0.next_u64() % n
    }

    fn hex(&mut self) -> String {
        format!("{:08x}", self.0.next_u64() as u32)
    }

    /// A message of `regime`, with the index of the shape it was written from.
    fn message(&mut self, regime: Regime) -> (u8, String) {
        let p = self.below(100);
        let (errors, misses) = match regime {
            Regime::Normal => (0, 10),
            Regime::Errors => (20, 10),
            Regime::Mix => (0, 50),
        };
        if p < errors {
            return match self.below(3) {
                0 => (
                    0,
                    format!(
                        "upstream db-{} timed out after {} ms",
                        self.below(4),
                        3000 + self.below(500)
                    ),
                ),
                1 => (
                    1,
                    format!(
                        "payment {} declined: gateway returned {}",
                        self.below(100_000),
                        500 + self.below(4)
                    ),
                ),
                _ => (
                    2,
                    format!("worker {} restarted after panic", self.below(16)),
                ),
            };
        }
        if p < errors + misses {
            return if self.below(2) == 0 {
                (3, format!("cache miss key={}", self.hex()))
            } else {
                (
                    4,
                    format!(
                        "retrying request {} attempt {}",
                        self.hex(),
                        1 + self.below(3)
                    ),
                )
            };
        }
        match self.below(6) {
            0 | 1 => (
                5,
                format!(
                    "connection from 10.0.{}.{} closed after {} ms",
                    self.below(4),
                    self.below(250),
                    self.below(90)
                ),
            ),
            2 => (
                6,
                format!(
                    "GET /api/items/{} 200 in {} ms",
                    self.below(1000),
                    self.below(40)
                ),
            ),
            3 => (
                7,
                format!(
                    "order {} placed by user {}",
                    self.below(100_000),
                    self.below(5000)
                ),
            ),
            4 => (8, format!("session {} refreshed", self.hex())),
            _ => (
                9,
                format!(
                    "inventory sync for warehouse {} took {} ms",
                    self.below(8),
                    self.below(900)
                ),
            ),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Regime {
    Normal,
    Errors,
    Mix,
}

fn regime(t: u64) -> Regime {
    let k = t / FAULT_EVERY;
    if t % FAULT_EVERY < FAULT_EVERY - FAULT_LEN {
        Regime::Normal
    } else if k % 2 == 0 {
        Regime::Errors
    } else {
        Regime::Mix
    }
}

/// The regime `[a, b)` lies wholly inside, if one.
fn inside(a: u64, b: u64) -> Option<Regime> {
    let r = regime(a);
    // Regimes change only on minute boundaries, so checking each minute is exact.
    (a..b)
        .step_by(MIN as usize)
        .all(|t| regime(t) == r)
        .then_some(r)
}

/// An event: its instant, the shape it was written from, and its text.
type Ev = (u64, u8, String);

/// `tent/1` of `[a, b)`, by Drain; and the same plug-in over the true shapes, which no
/// template miner can make or break.
fn tent(spec: &Spec, stream: &Stream<'_>, events: &[Ev], a: u64, b: u64) -> (f64, f64) {
    let lo = events.partition_point(|(t, _, _)| *t < a);
    let hi = events.partition_point(|(t, _, _)| *t < b);
    let w = &events[lo..hi];
    let by_drain = value(
        spec,
        stream,
        a,
        b,
        drain::run(w.iter().map(|(_, _, m)| m.as_str())),
    );
    let mut shapes: BTreeMap<String, u64> = BTreeMap::new();
    for (_, s, _) in w {
        *shapes.entry(format!("shape {s:02}")).or_insert(0) += 1;
    }
    (by_drain, value(spec, stream, a, b, shapes))
}

fn value(spec: &Spec, stream: &Stream<'_>, a: u64, b: u64, counts: BTreeMap<String, u64>) -> f64 {
    let t = Templates {
        instrument: stream.instrument,
        digest: [0; 32],
        counts,
    };
    let c = compute(spec, stream, a, b, &t).expect("a window holds enough events");
    c.value.to_string().parse().expect("a decimal")
}

fn median(xs: &mut [f64]) -> f64 {
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    }
}

fn quantile(xs: &mut [f64], q: f64) -> f64 {
    xs.sort_by(f64::total_cmp);
    xs[((xs.len() - 1) as f64 * q).round() as usize]
}

fn arg(args: &[String], name: &str, default: u64) -> u64 {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let days = arg(&args, "--days", 7);
    let seed = arg(&args, "--seed", 7);
    let mut sim = Sim(ChaCha20Rng::seed_from_u64(seed));
    let end = days * 24 * 60 * MIN;

    // About ten messages a minute, each at a uniform instant: 600 per hour.
    let mut instants: Vec<u64> = (0..end / 6_000).map(|_| sim.below(end)).collect();
    instants.sort_unstable();
    let events: Vec<Ev> = instants
        .into_iter()
        .map(|t| {
            let (shape, text) = sim.message(regime(t));
            (t, shape, text)
        })
        .collect();

    let spec = Spec::parse("{ stat: tent/1 }").expect("the shipped recipe");
    let stream = Stream {
        series: Uid::from_bytes([1; 32]),
        scheme: "journald",
        key: "ge-i4",
        class: "log",
        instrument: Uid::from_bytes([2; 32]),
    };

    let mut values: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    let mut moves: Vec<f64> = Vec::new();
    let mut moves_by: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    // The same moves of the true-shape entropy: what a shift moves by sampling alone.
    let mut shape_moves: Vec<f64> = Vec::new();
    let mut a = WINDOW;
    while a + WINDOW + SHIFT <= end {
        let b = a + WINDOW;
        if let Some(r) = inside(a - SHIFT, b + SHIFT) {
            let name = match r {
                Regime::Normal => "normal",
                Regime::Errors => "errors",
                Regime::Mix => "mix",
            };
            let (v, sv) = tent(&spec, &stream, &events, a, b);
            values.entry(name).or_default().push(v);
            for (sa, sb) in [(a - SHIFT, b - SHIFT), (a + SHIFT, b + SHIFT)] {
                let (shifted, sshifted) = tent(&spec, &stream, &events, sa, sb);
                let m = (shifted - v).abs();
                moves.push(m);
                moves_by.entry(name).or_default().push(m);
                shape_moves.push((sshifted - sv).abs());
            }
        }
        a += WINDOW;
    }

    println!(
        "GE-I4, RFC SMYSL-5 §15. One simulated log stream, {days} days, about 10 messages a \
         minute, seed {seed};"
    );
    println!(
        "30-minute windows, each also shifted by 3 minutes both ways; faults of 90 minutes \
         every 6 hours.\n"
    );
    println!(
        "{:<8} {:>8} {:>12} {:>14} {:>14}",
        "regime", "windows", "median bits", "median move", "p95 move"
    );
    for (name, vs) in &values {
        let mut v = vs.clone();
        let mut m = moves_by[name].clone();
        println!(
            "{name:<8} {:>8} {:>12.4} {:>14.4} {:>14.4}",
            v.len(),
            median(&mut v),
            median(&mut m),
            quantile(&mut m, 0.95)
        );
    }
    let mut all = moves.clone();
    let (med, p95, max) = (
        median(&mut all),
        quantile(&mut all, 0.95),
        quantile(&mut all, 1.0),
    );
    println!(
        "\nevery shift: median move {med:.4} bits, p95 {p95:.4}, largest {max:.4} ({} shifts)",
        moves.len()
    );
    let mut sm = shape_moves.clone();
    let (smed, sp95) = (median(&mut sm), quantile(&mut sm, 0.95));
    println!(
        "the true shapes' entropy, same shifts: median move {smed:.4}, p95 {sp95:.4}: what \
         sampling moves; the rest is Drain's"
    );

    let normal = median(&mut values["normal"].clone());
    println!("\nverdict (stop condition: the median move against each fault's median difference)");
    let mut failed = Vec::new();
    for kind in ["errors", "mix"] {
        let diff = (median(&mut values[kind].clone()) - normal).abs();
        let ok = med < diff;
        if !ok {
            failed.push(kind);
        }
        println!(
            "  {kind:<7} median difference {diff:.4} bits: median move {med:.4} is {}; \
             p95 {p95:.4} is {}",
            if ok { "below it" } else { "NOT below it" },
            if p95 < diff { "below it too" } else { "not" }
        );
    }
    println!("  Drain's share of the median move: {:.4} bits", med - smed);
    println!(
        "\nGE-I4 at seed {seed}: the stop condition {}. RFC SMYSL-5 §15 records the decision \
         over 20 seeds.",
        if failed.is_empty() {
            "holds for both kinds".to_string()
        } else {
            format!("fails for {}", failed.join(" and "))
        }
    );
}
