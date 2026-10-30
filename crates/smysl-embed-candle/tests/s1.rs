//! SMYSL-2.5 §5.7's S1 tests, on models small enough to make here.
//!
//! Each test writes a model directory of its own — `config.json`, a word-level
//! `tokenizer.json` and `model.safetensors` with random weights — for both architectures the
//! crate runs, BERT (`multilingual-e5-small`'s) and XLM-R (`-base`'s and `bge-m3`'s). Random
//! weights say nothing about meaning, and nothing here asks them to: these are the properties
//! that hold of *any* weights — determinism within a build, the model hash, the cache and its
//! invalidation, the seam. Meaning is `e5.rs`'s, on a real model.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device};
use candle_nn::{VarBuilder, VarMap};
use candle_transformers::models::{bert, xlm_roberta};
use smysl_core::diag::Code;
use smysl_core::{canonical_uid, KernelType, Record, Status, Uid, UnitCoreBuilder};
use smysl_embed_candle::{CandleSemantic, Encoder, Error, Pooling, TextRole, VectorCache};
use smysl_graph::Store;
use smysl_retrieve::{Query, Retriever};

const WORDS: &[&str] = &[
    "the",
    "river",
    "flooded",
    "village",
    "after",
    "three",
    "days",
    "of",
    "rain",
    "king",
    "forgave",
    "his",
    "brother",
    "and",
    "gave",
    "him",
    "land",
    "doctor",
    "said",
    "medicine",
    "causes",
    "headaches",
    "price",
    "bread",
    "rose",
    "in",
    "winter",
    "query",
    "passage",
    ":",
];

const GISTS: &[&str] = &[
    "the river flooded the village after three days of rain",
    "the king forgave his brother and gave him land",
    "the doctor said the medicine causes headaches",
    "the price of bread rose in winter",
];

#[derive(Clone, Copy, Debug)]
enum Arch {
    Bert,
    XlmRoberta,
}

/// A fresh directory under Cargo's per-target scratch space.
fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join("s1").join(name);
    if d.exists() {
        std::fs::remove_dir_all(&d).expect("clear the scratch model");
    }
    std::fs::create_dir_all(&d).expect("make the scratch model directory");
    d
}

/// Write a model of `arch` into `d`: 32 dimensions, two layers, random weights.
fn make_model(d: &Path, arch: Arch) {
    // Ids: [PAD] 0, [CLS] 1, [SEP] 2, [UNK] 3, then the words.
    let mut vocab = serde_json::Map::new();
    for (i, t) in ["[PAD]", "[CLS]", "[SEP]", "[UNK]"].iter().enumerate() {
        vocab.insert((*t).into(), i.into());
    }
    for (i, w) in WORDS.iter().enumerate() {
        vocab.insert((*w).into(), (i + 4).into());
    }
    let vocab_size = WORDS.len() + 4;
    let tokenizer = serde_json::json!({
        "version": "1.0",
        "truncation": null,
        "padding": null,
        "added_tokens": [
            {"id": 0, "content": "[PAD]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true},
            {"id": 1, "content": "[CLS]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true},
            {"id": 2, "content": "[SEP]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true},
            {"id": 3, "content": "[UNK]", "single_word": false, "lstrip": false, "rstrip": false, "normalized": false, "special": true}
        ],
        "normalizer": {"type": "Lowercase"},
        "pre_tokenizer": {"type": "Whitespace"},
        "post_processor": {"type": "BertProcessing", "sep": ["[SEP]", 2], "cls": ["[CLS]", 1]},
        "decoder": null,
        "model": {"type": "WordLevel", "vocab": vocab, "unk_token": "[UNK]"}
    });
    let config = match arch {
        Arch::Bert => serde_json::json!({
            "model_type": "bert", "vocab_size": vocab_size, "hidden_size": 32,
            "num_hidden_layers": 2, "num_attention_heads": 4, "intermediate_size": 64,
            "hidden_act": "gelu", "hidden_dropout_prob": 0.0, "attention_probs_dropout_prob": 0.0,
            "max_position_embeddings": 64, "type_vocab_size": 2, "initializer_range": 0.02,
            "layer_norm_eps": 1e-12, "pad_token_id": 0, "position_embedding_type": "absolute",
            "classifier_dropout": null
        }),
        Arch::XlmRoberta => serde_json::json!({
            "model_type": "xlm-roberta", "vocab_size": vocab_size, "hidden_size": 32,
            "num_hidden_layers": 2, "num_attention_heads": 4, "intermediate_size": 64,
            "hidden_act": "gelu", "hidden_dropout_prob": 0.0, "attention_probs_dropout_prob": 0.0,
            "max_position_embeddings": 66, "type_vocab_size": 1, "layer_norm_eps": 1e-5,
            "pad_token_id": 0, "position_embedding_type": "absolute"
        }),
    };
    let vm = VarMap::new();
    let vb = VarBuilder::from_varmap(&vm, DType::F32, &Device::Cpu);
    match arch {
        Arch::Bert => {
            let cfg: bert::Config = serde_json::from_value(config.clone()).expect("bert config");
            bert::BertModel::load(vb, &cfg).expect("random bert");
        }
        Arch::XlmRoberta => {
            let cfg: xlm_roberta::Config =
                serde_json::from_value(config.clone()).expect("xlm-r config");
            xlm_roberta::XLMRobertaModel::new(&cfg, vb).expect("random xlm-r");
        }
    }
    vm.save(d.join("model.safetensors")).expect("write weights");
    std::fs::write(d.join("config.json"), config.to_string()).expect("write config");
    std::fs::write(d.join("tokenizer.json"), tokenizer.to_string()).expect("write tokenizer");
}

fn model(name: &str, arch: Arch) -> PathBuf {
    let d = dir(name);
    make_model(&d, arch);
    d
}

fn store() -> (Store, Vec<Uid>) {
    let mut records = Vec::new();
    let mut uids = Vec::new();
    for g in GISTS {
        let core = UnitCoreBuilder::new(KernelType::Claim, *g, Status::Speculative)
            .build()
            .expect("fixture unit builds");
        uids.push(canonical_uid(&core));
        records.push(Record::Unit(core));
    }
    (Store::from_records(records), uids)
}

fn texts() -> Vec<String> {
    GISTS.iter().map(|g| g.to_string()).collect()
}

fn bits(v: &[Vec<f32>]) -> Vec<Vec<u32>> {
    v.iter()
        .map(|x| x.iter().map(|f| f.to_bits()).collect())
        .collect()
}

#[test]
fn both_architectures_load_and_return_unit_vectors() {
    for (arch, pooling) in [
        (Arch::Bert, Pooling::Mean),
        (Arch::XlmRoberta, Pooling::Cls),
    ] {
        let d = model(&format!("load-{arch:?}"), arch);
        let e = Encoder::from_dir(&d, pooling).expect("loads");
        assert_eq!(e.dim(), 32);
        for v in e.embed(&texts(), TextRole::Passage).expect("embeds") {
            assert_eq!(v.len(), 32);
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5, "{arch:?}: norm {norm}");
        }
    }
}

#[test]
fn two_runs_agree_bit_for_bit() {
    for arch in [Arch::Bert, Arch::XlmRoberta] {
        let d = model(&format!("det-{arch:?}"), arch);
        let a = Encoder::from_dir(&d, Pooling::Mean).expect("loads");
        let b = Encoder::from_dir(&d, Pooling::Mean).expect("loads again");
        let first = a.embed(&texts(), TextRole::Passage).expect("embeds");
        let again = a.embed(&texts(), TextRole::Passage).expect("embeds");
        let other = b.embed(&texts(), TextRole::Passage).expect("embeds");
        assert_eq!(
            bits(&first),
            bits(&again),
            "{arch:?}: one encoder, two runs"
        );
        assert_eq!(
            bits(&first),
            bits(&other),
            "{arch:?}: two loads of one directory"
        );

        // A text's vector does not depend on its neighbours: alone, or among others, in
        // either order, it is one vector.
        let reversed: Vec<String> = texts().into_iter().rev().collect();
        let mut back = a.embed(&reversed, TextRole::Passage).expect("embeds");
        back.reverse();
        assert_eq!(bits(&first), bits(&back), "{arch:?}: order");
        let alone = a.embed(&texts()[2..3], TextRole::Passage).expect("embeds");
        assert_eq!(bits(&first[2..3]), bits(&alone), "{arch:?}: alone");

        let (s, _) = store();
        let i = CandleSemantic::index(&s, a).expect("indexes");
        let j = CandleSemantic::index(&s, b).expect("indexes");
        let (_, uids) = store();
        for u in &uids {
            let (x, y) = (i.vector(u).unwrap(), j.vector(u).unwrap());
            assert!(x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits()));
        }
    }
}

#[test]
fn the_model_hash_covers_files_pooling_and_prefixes() {
    let d = model("hash", Arch::Bert);
    let mean = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    assert_eq!(
        mean.model_hash(),
        Encoder::from_dir(&d, Pooling::Mean).unwrap().model_hash()
    );
    let cls = Encoder::from_dir(&d, Pooling::Cls).unwrap();
    assert_ne!(mean.model_hash(), cls.model_hash(), "pooling");
    let h = mean.model_hash();
    let reprefixed = mean.with_prefixes("query: ", "passage: ");
    assert_eq!(
        reprefixed.model_hash(),
        h,
        "the defaults, restated, are the defaults"
    );
    let other = reprefixed.with_prefixes("q: ", "p: ");
    assert_ne!(other.model_hash(), h, "prefixes");

    // Other weights, in a directory otherwise the same.
    let d2 = model("hash-2", Arch::Bert);
    let fresh = Encoder::from_dir(&d2, Pooling::Mean).unwrap();
    assert_ne!(fresh.model_hash(), h, "weights");
}

#[test]
fn mean_prefixes_a_query_and_a_passage_differently_and_cls_does_not() {
    let d = model("roles", Arch::Bert);
    let mean = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    let q = mean.embed(&texts(), TextRole::Query).unwrap();
    let p = mean.embed(&texts(), TextRole::Passage).unwrap();
    assert_ne!(bits(&q), bits(&p));
    let cls = Encoder::from_dir(&d, Pooling::Cls).unwrap();
    let q = cls.embed(&texts(), TextRole::Query).unwrap();
    let p = cls.embed(&texts(), TextRole::Passage).unwrap();
    assert_eq!(bits(&q), bits(&p));
}

#[test]
fn a_gist_finds_itself_first() {
    // Under CLS with no prefixes a query and a passage of one text are one vector, so its
    // cosine is 1 and nothing outranks it — whatever the random weights.
    let d = model("seam", Arch::XlmRoberta);
    let (s, uids) = store();
    let idx = CandleSemantic::index(&s, Encoder::from_dir(&d, Pooling::Cls).unwrap()).unwrap();
    assert_eq!(idx.len(), 4);
    for (g, u) in GISTS.iter().zip(&uids) {
        let hits = idx.search(&Query::new(*g, 4));
        assert_eq!(hits.first().map(|h| h.uid), Some(*u), "{g}");
        assert!((hits[0].score - 1.0).abs() < 1e-5);
    }
    assert!(idx.search(&Query::new(GISTS[0], 0)).is_empty());
    let within = idx.search(&Query::new(GISTS[0], 4).within([uids[1], uids[2]]));
    assert!(within.iter().all(|h| h.uid == uids[1] || h.uid == uids[2]));
}

#[test]
fn a_long_gist_is_truncated_to_the_position_table() {
    // 64 positions in the BERT, 66 - 1 in the XLM-R: a gist of 500 words would index past
    // either without truncation.
    let long = vec!["rain"; 500].join(" ");
    for arch in [Arch::Bert, Arch::XlmRoberta] {
        let d = model(&format!("long-{arch:?}"), arch);
        let e = Encoder::from_dir(&d, Pooling::Mean).unwrap();
        let v = e
            .embed(std::slice::from_ref(&long), TextRole::Passage)
            .expect("truncated, not refused");
        assert_eq!(v[0].len(), 32);
    }
}

#[test]
fn the_cache_stands_in_for_the_encoder() {
    let d = model("cache", Arch::Bert);
    let (s, uids) = store();
    let side = VectorCache::sidecar(
        d.join("store.smysl"),
        &Encoder::from_dir(&d, Pooling::Mean).unwrap().model_hash(),
    );
    let name = side.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.starts_with("store.smysl.vec-") && name.len() == "store.smysl.vec-".len() + 12,
        "{name}"
    );

    let e = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    let (mut cache, diags) = VectorCache::open(&side, &e).expect("no file is an empty cache");
    assert!(cache.is_empty() && diags.is_empty());
    let fresh = CandleSemantic::index_cached(&s, e, &mut cache).unwrap();
    assert_eq!(cache.len(), 4);
    cache.save(&side).expect("saves");

    let e = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    let (mut back, diags) = VectorCache::open(&side, &e).expect("reads");
    assert!(diags.is_empty());
    assert_eq!(back, cache, "a saved cache reads back as itself");
    let cached = CandleSemantic::index_cached(&s, e, &mut back).unwrap();
    for u in &uids {
        let (x, y) = (fresh.vector(u).unwrap(), cached.vector(u).unwrap());
        assert!(
            x.iter().zip(y).all(|(p, q)| p.to_bits() == q.to_bits()),
            "bit for bit"
        );
    }

    // A unit gone from the store leaves the cache with it.
    let smaller = Store::from_records(
        GISTS[..2]
            .iter()
            .map(|g| {
                Record::Unit(
                    UnitCoreBuilder::new(KernelType::Claim, *g, Status::Speculative)
                        .build()
                        .unwrap(),
                )
            })
            .collect(),
    );
    let e = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    CandleSemantic::index_cached(&smaller, e, &mut back).unwrap();
    assert_eq!(back.len(), 2);
}

#[test]
fn a_cache_under_another_model_hash_is_ignored_with_w476() {
    let d = model("w476", Arch::Bert);
    let (s, _) = store();
    let side = d.join("store.smysl.vec-by-hand");
    let mean = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    let mut cache = VectorCache::new(&mean);
    CandleSemantic::index_cached(&s, mean, &mut cache).unwrap();
    cache.save(&side).unwrap();

    // The same weights pooled the other way: another model hash, so every vector goes.
    let cls = Encoder::from_dir(&d, Pooling::Cls).unwrap();
    let (mut other, diags) = VectorCache::open(&side, &cls).expect("reads");
    assert!(other.is_empty());
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, Code::W476);
    assert!(
        diags[0].message.starts_with("4 cached vector(s)"),
        "{}",
        diags[0].message
    );
    assert_eq!(other.model_hash(), cls.model_hash());
    CandleSemantic::index_cached(&s, cls, &mut other).unwrap();
    assert_eq!(other.len(), 4, "embedded again");

    // Handing an index a cache opened for another encoder is a mistake, not a W476.
    let mean = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    assert!(matches!(
        CandleSemantic::index_cached(&s, mean, &mut other),
        Err(Error::WrongCache)
    ));
}

#[test]
fn a_cache_that_does_not_decode_is_an_error() {
    let d = model("garbage", Arch::Bert);
    let side = d.join("store.smysl.vec-garbage");
    std::fs::write(&side, b"not cbor at all").unwrap();
    let e = Encoder::from_dir(&d, Pooling::Mean).unwrap();
    assert!(matches!(VectorCache::open(&side, &e), Err(Error::Cache(_))));
}

#[test]
fn what_is_not_a_model_is_refused_by_name() {
    let d = model("refuse", Arch::Bert);
    assert!(matches!(
        Encoder::from_dir(d.join("nowhere"), Pooling::Mean),
        Err(Error::NotADirectory(_))
    ));
    std::fs::write(d.join("config.json"), r#"{"model_type": "gpt2"}"#).unwrap();
    assert!(matches!(
        Encoder::from_dir(&d, Pooling::Mean),
        Err(Error::Architecture(t)) if t == "gpt2"
    ));
    std::fs::remove_file(d.join("tokenizer.json")).unwrap();
    assert!(matches!(
        Encoder::from_dir(&d, Pooling::Mean),
        Err(Error::Missing(n, _)) if n == "tokenizer.json"
    ));
}

#[test]
fn a_directory_declares_its_pooling_the_sentence_transformers_way() {
    let d = dir("pooling-file");
    assert_eq!(Pooling::from_dir(&d), None, "no file");
    std::fs::create_dir_all(d.join("1_Pooling")).unwrap();
    let write = |mean: bool, cls: bool| {
        std::fs::write(
            d.join("1_Pooling").join("config.json"),
            serde_json::json!({"pooling_mode_mean_tokens": mean, "pooling_mode_cls_token": cls})
                .to_string(),
        )
        .unwrap()
    };
    write(true, false);
    assert_eq!(Pooling::from_dir(&d), Some(Pooling::Mean));
    write(false, true);
    assert_eq!(Pooling::from_dir(&d), Some(Pooling::Cls));
    write(true, true);
    assert_eq!(Pooling::from_dir(&d), None, "both is not a declaration");
}
