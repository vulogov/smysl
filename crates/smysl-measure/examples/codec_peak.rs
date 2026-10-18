//! One T2 compression, for measuring its peak memory (SMYSL-2.6 §6, TX-P8 M3's exit):
//!
//! ```text
//! cargo build --release -p smysl-measure --example codec_peak
//! /usr/bin/time -l target/release/examples/codec_peak 16   # macOS; `-v` on Linux
//! ```
//!
//! The argument is the input in MiB, at most 16: a text of words drawn from a seeded
//! vocabulary of 5,000, skewed towards the first, cut to the size and to brotli's window. Or a
//! file's path, whose first window is compressed. It prints the input and compressed lengths;
//! the peak is the `time` line.

use smysl_measure::rng::Rng;
use smysl_measure::text::compress::{compressed_len, WINDOW};

fn main() {
    let arg = std::env::args().nth(1).unwrap_or_else(|| "16".into());
    let Ok(mib) = arg.parse::<usize>() else {
        let mut data = std::fs::read(&arg).expect("a file");
        data.truncate(WINDOW);
        let c = compressed_len(&data).expect("the codec's own parameters");
        println!("input {} bytes, compressed {c} bytes", data.len());
        return;
    };
    let want = (mib << 20).min(WINDOW);
    let mut rng = Rng::new(&[3; 32], 0);
    let vocab: Vec<String> = (0..5000)
        .map(|_| {
            let len = 2 + rng.index(8);
            (0..len)
                .map(|_| char::from(b'a' + rng.index(26) as u8))
                .collect()
        })
        .collect();
    let mut text = String::with_capacity(want + 16);
    while text.len() < want {
        let top = rng.index(vocab.len()) + 1;
        let w = &vocab[rng.index(top)];
        text.push_str(w);
        text.push(if rng.index(12) == 0 { '\n' } else { ' ' });
    }
    text.truncate(want);
    let c = compressed_len(text.as_bytes()).expect("the codec's own parameters");
    println!("input {} bytes, compressed {c} bytes", text.len());
}
