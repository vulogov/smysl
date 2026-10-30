//! A multilingual model, loaded and used across languages: TX-P10 step 2's smoke test.
//!
//! ```text
//! SMYSL_EMBED_MODEL_MULTILINGUAL=/path/to/potion-multilingual-128M cargo test -p smysl-embed \
//!     --release --test multilingual -- --nocapture
//! ```
//!
//! A variable of its own rather than `SMYSL_EMBED_MODEL`: `semantic.rs` asserts English
//! rankings taken with `potion-base-8M`, and a model trained for a hundred languages is not
//! obliged to reproduce them. Skips without one, as `semantic.rs` does, and `required` turns a
//! skip into a failure.
//!
//! The corpus is four claims on unrelated subjects, each in the five tier-1 languages, written
//! for this test. A query is a claim in one language and is searched within one language's four
//! units. The bar was set before the first run: every unit embedded, every query finding its
//! own claim in its own language, and the right claim first in at least half of the
//! cross-language searches, where chance is a quarter. It is a smoke test — that the model
//! loads and that its vectors carry meaning across a language boundary — and not a measure of
//! cross-lingual retrieval, which is GE-T5's cross-lingual arm (TX-P10 step 4).

use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_embed::{Model, Semantic};
use smysl_graph::Store;
use smysl_retrieve::{Query, Retriever};

const LANGS: [&str; 5] = ["de", "en", "es", "fr", "ru"];

/// `CLAIMS[c][l]` is claim `c` in `LANGS[l]`.
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

fn model() -> Option<Model> {
    let path = std::env::var("SMYSL_EMBED_MODEL_MULTILINGUAL").ok()?;
    if path == "required" {
        panic!("SMYSL_EMBED_MODEL_MULTILINGUAL=required needs a path as well");
    }
    match Model::from_dir(&path) {
        Ok(m) => Some(m),
        Err(e) => {
            panic!("SMYSL_EMBED_MODEL_MULTILINGUAL is {path} and the model would not load: {e}")
        }
    }
}

#[test]
fn a_multilingual_model_finds_a_claim_across_languages() {
    let Some(m) = model() else {
        eprintln!("skipped: set SMYSL_EMBED_MODEL_MULTILINGUAL to a model directory");
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
    let idx = Semantic::index(&store, m);
    assert_eq!(idx.len(), 20, "every unit embedded");

    // right[q][t]: of the four claims asked in language q, how many find themselves first among
    // language t's four units.
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
