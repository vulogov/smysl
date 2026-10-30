//! Writes T3's calibration table (SMYSL-2.6 §3.7): both estimators on i.i.d. sources of 1 to 8
//! bits, 10³ to 10⁷ symbols, seed 0, so that a reader can see the bias at their n.
//!
//! ```text
//! cargo run --release -p smysl-measure --example t3_calibration > fixtures/measure/t3-calibration.tsv
//! ```
//!
//! `tests/lz_match.rs` recomputes every row bit for bit: up to 10⁵ symbols by default, and
//! the rest with `--ignored`. With `seeds <k>` it prints instead each row's mean and spread over
//! seeds `0..k`, which is what §2.9's probe is compared against; with `<bits> <n>`, the one row,
//! which under `/usr/bin/time -l` is T3's peak memory at that n; and with `<bits> <n> corpus`,
//! the same text built and refused before T3 runs, which is the part of that peak that is the
//! harness's corpus and not T3.

#[path = "../tests/support/calibration.rs"]
mod calibration;

use calibration::{row, t3, BITS, LENGTHS};
use smysl_measure::{Num, Outcome};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [] => {}
        [what, k] if what == "seeds" => return spread(k.parse().expect("a count of seeds")),
        [bits, n] => {
            let (bits, n) = (bits.parse().expect("bits"), n.parse().expect("a length"));
            return println!("{}", row(bits, n, 0));
        }
        [bits, n, what] if what == "corpus" => {
            let (bits, n) = (bits.parse().expect("bits"), n.parse().expect("a length"));
            let r = t3(calibration::iid(bits, n, 0), u64::MAX);
            return println!("{:?}", matches!(r.outcome, Outcome::NotRun { .. }));
        }
        _ => panic!("usage: t3_calibration [seeds <k> | <bits> <n> [corpus]]"),
    }
    println!("# T3 calibration (SMYSL-2.6 §3.7): i.i.d. uniform over 2^bits symbols, seed 0.");
    println!("# Written by `examples/t3_calibration`; checked by `tests/lz_match.rs`.");
    println!("# bits\tn\tseed\tq20(H)\tq20(H:mr)\tH\tH:mr\tblake3(payload)");
    for bits in BITS {
        for n in LENGTHS {
            println!("{}", row(bits, n, 0));
        }
    }
}

fn spread(k: u64) {
    println!("# bits\tn\tmean(H)\tsd(H)\tmean(H:mr)\tsd(H:mr)");
    for bits in BITS {
        for n in LENGTHS.into_iter().filter(|&n| n <= 100_000) {
            let (mut h, mut mr) = (Vec::new(), Vec::new());
            for seed in 0..k {
                let r = t3(calibration::iid(bits, n, seed), 2);
                let Outcome::Value(v) = &r.outcome else {
                    panic!("not run")
                };
                let f = |key: &str| match v.values[key] {
                    Num::Fixed(i) => smysl_measure::num::from_q20(i),
                    _ => unreachable!(),
                };
                h.push(f("H"));
                mr.push(f("H:mr"));
            }
            let (mh, sh) = stats(&h);
            let (mm, sm) = stats(&mr);
            println!("{bits}\t{n}\t{mh:.4}\t{sh:.4}\t{mm:.4}\t{sm:.4}");
        }
    }
}

fn stats(xs: &[f64]) -> (f64, f64) {
    let n = xs.len() as f64;
    let m = xs.iter().sum::<f64>() / n;
    let v = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (n - 1.0).max(1.0);
    (m, v.sqrt())
}
