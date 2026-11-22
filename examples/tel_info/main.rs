//! Information measures over telemetry (RFC SMYSL-5), on GE-M3's four series
//! (`fixtures/tel/detect/series.tsv`): four hours at one-minute steps, each with one thing in it
//! for a detector to find.
//!
//! | series | what happens | what a statistic should see |
//! |---|---|---|
//! | `cpu` | its level moves from 40 to 55 at minute 150 | three quarters of the second half above every reference quartile |
//! | `latency` | a five-minute spike at minute 160 | five points of 120: little |
//! | `errors` | a three-minute dip at minute 170 | less |
//! | `queue` | a constant 12 that becomes 30 at minute 180 | half the window at a level the reference never had; one ordinal pattern |
//!
//! It computes, by the code `smysl tel info` runs:
//!
//! - `jsd/1`: each series' last two hours against its first two, binned at the reference
//!   window's quartiles (`baseline:4`);
//! - `perm/1`: the entropy of each series' ordinal patterns of three over the four hours; noise
//!   is near `log₂ 6`, a constant has one pattern;
//! - `mi/1`: a lag scan of `cpu` against `latency`, -5 to 5 minutes, with 200 surrogates: two
//!   independent noises, so a peak no better than the surrogates' is what to expect.
//!
//! Then `tel-evidence` recomputes every unit from the substrate (RFC SMYSL-5 §7) and finds each
//! is itself; and a late sample in `cpu`'s window makes its units `SMY-E604`.
//!
//! ```text
//! cargo run --features tel --example tel_info
//! ```

use std::collections::BTreeMap;
use std::path::Path;

use smysl::schemas::tel;
use smysl::series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl::tel::info::{run_on, verify::verify, Computed, Spec, With};
use smysl::{canonical_uid, Decimal, Uid, UnitCore};

/// 2026-01-16T08:00:00Z, the fixture's first minute.
const T0: u64 = 1_768_550_400_000;
const MIN: u64 = 60_000;
const KEYS: [&str; 4] = ["cpu", "latency", "errors", "queue"];

fn key(name: &str) -> String {
    format!("ge_m3_{name}{{instance=\"sim:9100\",job=\"ge-m3\"}}")
}

/// The four series in a substrate of their own, read by one scraper.
fn substrate() -> (Substrate<MemBackend>, BTreeMap<&'static str, Uid>) {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/tel/detect/series.tsv");
    let text = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let inst = canonical_uid(&tel::instrument("scraper", "prom-scrape:ge-m3/sim:9100").unwrap());
    let mut sub = Substrate::new(MemBackend::default());
    let mut uids = BTreeMap::new();
    for name in KEYS {
        let k = key(name);
        let samples: Vec<Sample> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .map(|l| l.split('\t').collect::<Vec<_>>())
            .filter(|f| f[0] == k)
            .map(|f| {
                let v = Decimal::parse_token(f[2]).unwrap();
                Sample::new(f[1].parse().unwrap(), None, v, 0).unwrap()
            })
            .collect();
        let uid = canonical_uid(&tel::series("prom", &k).unwrap());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: k,
            class: "gauge".into(),
        };
        sub.add(&uid, &inst, &info, &samples).unwrap();
        uids.insert(name, uid);
    }
    (sub, uids)
}

fn one(
    spec: &Spec,
    sub: &Substrate<MemBackend>,
    s: Uid,
    with: Option<With>,
    a: u64,
    b: u64,
) -> Computed {
    let o = run_on(spec, sub, &[s], None, with, a, b).expect("the substrate reads");
    o.into_iter()
        .next()
        .unwrap()
        .result
        .unwrap_or_else(|r| panic!("{}: {r}", spec.id()))
}

fn main() {
    let (mut sub, uids) = substrate();
    let half = T0 + 120 * MIN;
    let end = T0 + 240 * MIN;
    let jsd = Spec::parse(&format!(
        r#"{{ stat: jsd/1, step: 60000, bins: "baseline:4", ref: "{T0}-{half}", min_n: 100 }}"#
    ))
    .unwrap();
    let perm = Spec::parse("{ stat: perm/1, step: 60000, m: 3 }").unwrap();
    let mi = Spec::parse(
        r#"{ stat: mi/1, step: 60000, bins: "rank:4", lag: "-5..5", null: 200, seed: 7 }"#,
    )
    .unwrap();

    println!(
        "GE-M3's four series (fixtures/tel/detect/), {} minutes each\n",
        240
    );
    println!(
        "{:<8} {:>22} {:>22}",
        "series", "jsd/1 last 2h vs first", "perm/1 m=3 over 4h"
    );
    let mut units: Vec<UnitCore> = Vec::new();
    for name in KEYS {
        let s = uids[name];
        let d = one(&jsd, &sub, s, None, half, end);
        let p = one(&perm, &sub, s, None, T0, end);
        println!(
            "{name:<8} {:>17} bits {:>12} bits, k {}",
            d.value.to_string(),
            p.value.to_string(),
            p.k
        );
        units.extend([d.unit, p.unit]);
    }
    let scan = one(
        &mi,
        &sub,
        uids["cpu"],
        Some(With {
            series: uids["latency"],
            instrument: None,
        }),
        T0,
        end,
    );
    let (ge, b) = scan.null.expect("surrogates were drawn");
    println!(
        "\nmi/1 cpu ~ latency over -5..5 min: {} bits at lag {}, bias term {}; {ge} of {b} \
         surrogates at least as high",
        scan.value,
        scan.lag.unwrap(),
        scan.bias
            .as_ref()
            .map_or("-".to_string(), |d| d.to_string())
    );
    units.push(scan.unit);

    // RFC SMYSL-5 §7: every unit recomputed from what it read.
    let clean = verify(&units, &[], &sub);
    println!(
        "\ntel-evidence: {} units recomputed, {} differ",
        units.len(),
        clean.len()
    );
    assert!(clean.is_empty(), "{clean:?}");

    // A sample arrives late, inside cpu's second half: its units no longer match the substrate.
    let cpu = uids["cpu"];
    let inst = sub.instruments(&cpu).unwrap()[0];
    let info = SeriesInfo {
        scheme: "prom".into(),
        key: key("cpu"),
        class: "gauge".into(),
    };
    let late = Sample::new(half + 30 * MIN + 30_000, None, Decimal::from_i64(90), 0).unwrap();
    sub.add(&cpu, &inst, &info, &[late]).unwrap();
    let after = verify(&units, &[], &sub);
    println!("a late cpu sample: {} unit(s) now SMY-E604", after.len());
    for d in &after {
        println!("  {d}");
    }
    assert!(!after.is_empty() && after.iter().all(|d| d.code == smysl::Code::E604));
}
