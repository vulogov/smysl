//! S1 on a real model: `multilingual-e5-small`, TX-P10 step 3.
//!
//! ```text
//! SMYSL_EMBED_CANDLE_MODEL=/path/to/multilingual-e5-small cargo test -p smysl-embed-candle \
//!     --release --test e5 -- --nocapture
//! ```
//!
//! Skips without the variable, and `required` turns a skip into a failure, as S0's tests do.
//! Taken at revision `614241f622f53c4eeff9890bdc4f31cfecc418b3` of `intfloat/multilingual-e5-small`
//! (`model.safetensors` 470,641,600 bytes): a BERT with XLM-R's vocabulary, mean pooling, the
//! `query: `/`passage: ` prefixes.
//!
//! **What the reference vectors are, exactly.** The first eight components of three texts'
//! vectors, computed by this crate once (2026-10-27, aarch64-apple-darwin) and stored here,
//! compared within 1e-4. They are a baseline, not an independent reference: no PyTorch was at
//! hand to compute the model card's own numbers, so what they catch is this crate changing its
//! mind — a tokenizer, pooling or prefix change — and not a mistake it made the first time. The
//! cross-language search below is what says the vectors carry meaning; the tolerance is wide
//! because a different CPU may reduce a matmul in another order (§3.14 claims no cross-machine
//! agreement).

use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_embed_candle::{CandleSemantic, Encoder, Pooling, TextRole};
use smysl_graph::Store;
use smysl_retrieve::{Query, Retriever};

const LANGS: [&str; 5] = ["de", "en", "es", "fr", "ru"];

/// `CLAIMS[c][l]` is claim `c` in `LANGS[l]` — S0's step 2 set, so the two are comparable.
const CLAIMS: [[&str; 5]; 4] = [
    [
        "der Fluss überschwemmte das Dorf nach drei Tagen Regen",
        "the river flooded the village after three days of rain",
        "el río inundó el pueblo después de tres días de lluvia",
        "la rivière a inondé le village après trois jours de pluie",
        "река затопила деревню после трёх дней дождя",
    ],
    [
        "der König vergab seinem Bruder und gab ihm Land",
        "the king forgave his brother and gave him land",
        "el rey perdonó a su hermano y le dio tierras",
        "le roi a pardonné à son frère et lui a donné des terres",
        "король простил своего брата и дал ему землю",
    ],
    [
        "der Arzt sagte, das Medikament verursache Kopfschmerzen",
        "the doctor said the medicine causes headaches",
        "el médico dijo que el medicamento causa dolores de cabeza",
        "le médecin a dit que le médicament provoque des maux de tête",
        "врач сказал, что лекарство вызывает головную боль",
    ],
    [
        "der Brotpreis stieg im Winter stark an",
        "the price of bread rose sharply in the winter",
        "el precio del pan subió mucho en invierno",
        "le prix du pain a fortement augmenté en hiver",
        "цена хлеба резко выросла зимой",
    ],
];

const REFERENCE_TEXTS: [&str; 3] = [
    "the river flooded the village after three days of rain",
    "король простил своего брата и дал ему землю",
    "",
];

/// `REFERENCE[i]`: the first eight components of `REFERENCE_TEXTS[i]` as a passage, then as
/// a query.
#[rustfmt::skip]
const REFERENCE: [[[f32; 8]; 2]; 3] = [
    [
        [0.027747309, 0.03726549, 0.014456686, -0.11381744, 0.09723684, -0.02317435, -0.04692768, 0.009523887],
        [0.050077464, -0.01119523, 0.0046621375, -0.07040222, 0.07873804, -0.03848733, 0.006488592, 0.034313563],
    ],
    [
        [0.049108393, 0.014434983, 0.022714479, -0.041513283, 0.058513924, -0.00767031, -0.011641577, -0.003170144],
        [0.059442032, -0.017301925, 0.02393025, -0.033849157, 0.046199616, -0.036040965, 0.008681181, 0.03794429],
    ],
    [
        [0.060428876, 0.004255813, -0.010002949, -0.025809951, 0.07339259, -0.015065503, 0.02267903, 0.022733063],
        [0.040028308, 0.011368814, -0.08232319, -0.048613627, 0.07085877, 0.005006015, 0.032389145, 0.01826396],
    ],
];

const TOLERANCE: f32 = 1e-4;

fn encoder() -> Option<Encoder> {
    let path = std::env::var("SMYSL_EMBED_CANDLE_MODEL").ok()?;
    if path == "required" {
        panic!("SMYSL_EMBED_CANDLE_MODEL=required needs a path as well");
    }
    match Encoder::from_dir(&path, Pooling::Mean) {
        Ok(e) => Some(e),
        Err(e) => panic!("SMYSL_EMBED_CANDLE_MODEL is {path} and the model would not load: {e}"),
    }
}

#[test]
fn e5_small_reproduces_its_reference_vectors() {
    let Some(e) = encoder() else {
        eprintln!("skipped: set SMYSL_EMBED_CANDLE_MODEL to a multilingual-e5-small directory");
        return;
    };
    assert_eq!(e.dim(), 384);
    let texts: Vec<String> = REFERENCE_TEXTS.iter().map(|t| t.to_string()).collect();
    let t = std::time::Instant::now();
    let passages = e.embed(&texts, TextRole::Passage).unwrap();
    let queries = e.embed(&texts, TextRole::Query).unwrap();
    println!("6 texts in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let again = e.embed(&texts, TextRole::Passage).unwrap();
    assert_eq!(passages, again, "two runs, one encoder");
    for (i, (p, q)) in passages.iter().zip(&queries).enumerate() {
        println!("{:?}: passage {:?}", REFERENCE_TEXTS[i], &p[..8]);
        println!("{:?}: query   {:?}", REFERENCE_TEXTS[i], &q[..8]);
    }
    for (i, (p, q)) in passages.iter().zip(&queries).enumerate() {
        for (got, want) in [(p, REFERENCE[i][0]), (q, REFERENCE[i][1])] {
            for (k, (g, w)) in got.iter().zip(want).enumerate() {
                assert!(
                    (g - w).abs() <= TOLERANCE,
                    "{:?} component {k}: {g} against the stored {w}",
                    REFERENCE_TEXTS[i]
                );
            }
        }
    }
}

#[test]
fn e5_small_finds_a_claim_across_languages() {
    let Some(e) = encoder() else {
        eprintln!("skipped: set SMYSL_EMBED_CANDLE_MODEL to a multilingual-e5-small directory");
        return;
    };
    let mut records = Vec::new();
    let mut uid = [[Uid::from_bytes([0; 32]); 5]; 4];
    for (c, claim) in CLAIMS.iter().enumerate() {
        for (l, gist) in claim.iter().enumerate() {
            let core = UnitCoreBuilder::new(KernelType::Claim, *gist, Status::Speculative)
                .build()
                .expect("fixture unit builds");
            uid[c][l] = canonical_uid(&core);
            records.push(Record::Unit(core));
        }
    }
    let store = Store::from_records(records);
    let t = std::time::Instant::now();
    let idx = CandleSemantic::index(&store, e).unwrap();
    println!(
        "20 units indexed in {:.0} ms",
        t.elapsed().as_secs_f64() * 1e3
    );
    assert_eq!(idx.len(), 20, "every unit embedded");

    // The bar is step 2's, set before that run: every claim finds itself in its own language,
    // and the right claim first in at least half of the 80 cross-language searches.
    let mut right = [[0usize; 5]; 5];
    for (c, claim) in CLAIMS.iter().enumerate() {
        for (q, text) in claim.iter().enumerate() {
            for (t, row) in right[q].iter_mut().enumerate() {
                let within = (0..4).map(|k| uid[k][t]);
                let hits = idx.search(&Query::new(*text, 1).within(within));
                if hits.first().map(|h| h.uid) == Some(uid[c][t]) {
                    *row += 1;
                }
            }
        }
    }
    println!("query \\ target  {}", LANGS.join("  "));
    for (q, row) in right.iter().enumerate() {
        let cells: Vec<String> = row.iter().map(|n| format!("{n}/4")).collect();
        println!("{:<15} {}", LANGS[q], cells.join(" "));
    }
    for (q, row) in right.iter().enumerate() {
        assert_eq!(
            row[q], 4,
            "a claim asked in {} did not find itself",
            LANGS[q]
        );
    }
    let across: usize = (0..5)
        .flat_map(|q| (0..5).filter(move |&t| t != q).map(move |t| (q, t)))
        .map(|(q, t)| right[q][t])
        .sum();
    let rate = across as f64 / 80.0;
    println!("across languages: {across}/80 = {rate:.3} (chance 0.250, bar 0.500)");
    assert!(
        rate >= 0.5,
        "cross-language top-1 {rate:.3} below the bar of 0.5"
    );
}
