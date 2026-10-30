//! The encoder: a transformer, its tokenizer and a pooling, loaded from a directory.

use std::path::Path;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::{bert, xlm_roberta};
use tokenizers::{Tokenizer, TruncationParams};

use crate::pool::Pooling;
use crate::Error;

/// The longest input either family was trained on, in tokens, special tokens included.
const MAX_TOKENS: usize = 512;

/// Which side of a search a text is on. `multilingual-e5-*` are trained with the two
/// prefixed differently and lose most of their quality without it; `bge-m3` prefixes
/// neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextRole {
    Query,
    Passage,
}

/// The two architectures the families of §3.14 come in. `multilingual-e5-small` is a BERT
/// with XLM-R's vocabulary (its `config.json` says `"model_type": "bert"`, whatever its
/// tokenizer class); `multilingual-e5-base` and `bge-m3` are XLM-R. They differ where it
/// matters here: XLM-R numbers positions from one past the padding id, BERT from zero.
enum Net {
    Bert(bert::BertModel),
    XlmRoberta(xlm_roberta::XLMRobertaModel),
}

/// A loaded encoder.
///
/// CPU and `f32` only. An accelerator is native code by definition (the manifest says why
/// none is compiled), and half precision would make a cached vector depend on which one
/// computed it.
pub struct Encoder {
    net: Net,
    tokenizer: Tokenizer,
    pooling: Pooling,
    query_prefix: String,
    passage_prefix: String,
    /// BLAKE3 over the three files, kept so a change of prefixes can rehash without reading
    /// them again.
    files: [u8; 32],
    hash: [u8; 32],
    dim: usize,
}

impl Encoder {
    /// Load from a directory holding `config.json`, `tokenizer.json` and `model.safetensors`.
    ///
    /// The prefixes are the model card's for the pooling given: `query: ` and `passage: `
    /// for [`Pooling::Mean`] (`multilingual-e5-*`), none for [`Pooling::Cls`] (`bge-m3`).
    /// [`Encoder::with_prefixes`] sets others.
    ///
    /// Every byte of the three files is read: the weights are hashed for the cache's key and
    /// handed to candle from memory, so nothing here maps a file (`from_mmaped_safetensors`
    /// is `unsafe`, and this crate forbids it).
    pub fn from_dir(path: impl AsRef<Path>, pooling: Pooling) -> Result<Encoder, Error> {
        let path = path.as_ref();
        if !path.is_dir() {
            return Err(Error::NotADirectory(path.display().to_string()));
        }
        let read = |name: &str| {
            std::fs::read(path.join(name))
                .map_err(|e| Error::Missing(name.to_string(), e.to_string()))
        };
        let config = read("config.json")?;
        let tokenizer = read("tokenizer.json")?;
        let weights = read("model.safetensors")?;
        Encoder::from_bytes(&config, &tokenizer, &weights, pooling)
    }

    /// Load from bytes the caller already holds — the three files' contents.
    pub fn from_bytes(
        config: &[u8],
        tokenizer: &[u8],
        weights: &[u8],
        pooling: Pooling,
    ) -> Result<Encoder, Error> {
        let files = files_digest(config, tokenizer, weights);
        let value: serde_json::Value =
            serde_json::from_slice(config).map_err(|e| Error::Config(e.to_string()))?;
        let model_type = value
            .get("model_type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_string();
        let vb = VarBuilder::from_slice_safetensors(weights, DType::F32, &Device::Cpu)
            .map_err(|e| Error::Model(e.to_string()))?;

        // Positions available to tokens, which bounds the input once the offset is paid.
        let (net, dim, positions) = match model_type.as_str() {
            "bert" => {
                let cfg: bert::Config =
                    serde_json::from_value(value).map_err(|e| Error::Config(e.to_string()))?;
                let net =
                    bert::BertModel::load(vb, &cfg).map_err(|e| Error::Model(e.to_string()))?;
                (Net::Bert(net), cfg.hidden_size, cfg.max_position_embeddings)
            }
            "xlm-roberta" => {
                let cfg: xlm_roberta::Config =
                    serde_json::from_value(value).map_err(|e| Error::Config(e.to_string()))?;
                // Some checkpoints keep the encoder under `roberta.`, as a masked-LM export
                // does; `bge-m3`'s and `e5-base`'s do not. Tried bare first.
                let net = xlm_roberta::XLMRobertaModel::new(&cfg, vb.clone())
                    .or_else(|_| xlm_roberta::XLMRobertaModel::new(&cfg, vb.pp("roberta")))
                    .map_err(|e| Error::Model(e.to_string()))?;
                let offset = cfg.pad_token_id as usize + 1;
                (
                    Net::XlmRoberta(net),
                    cfg.hidden_size,
                    cfg.max_position_embeddings.saturating_sub(offset),
                )
            }
            other => return Err(Error::Architecture(other.to_string())),
        };

        let mut tokenizer =
            Tokenizer::from_bytes(tokenizer).map_err(|e| Error::Tokenizer(e.to_string()))?;
        // Truncation is set here rather than taken from the file: a tokenizer.json that
        // asked for none would otherwise run a 600-token gist into a position table of 512.
        // Padding is off, because every text is encoded alone (see `embed`).
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS.min(positions),
                ..TruncationParams::default()
            }))
            .map_err(|e| Error::Tokenizer(e.to_string()))?;
        tokenizer.with_padding(None);

        let (query_prefix, passage_prefix) = match pooling {
            Pooling::Mean => ("query: ", "passage: "),
            Pooling::Cls => ("", ""),
        };
        let mut encoder = Encoder {
            net,
            tokenizer,
            pooling,
            query_prefix: query_prefix.to_string(),
            passage_prefix: passage_prefix.to_string(),
            files,
            hash: [0; 32],
            dim,
        };
        encoder.rehash();
        Ok(encoder)
    }

    /// Replace the prefixes put before a query and a passage. They change every vector, so
    /// they change the model hash.
    pub fn with_prefixes(
        mut self,
        query: impl Into<String>,
        passage: impl Into<String>,
    ) -> Encoder {
        self.query_prefix = query.into();
        self.passage_prefix = passage.into();
        self.rehash();
        self
    }

    /// The cache's key for this encoder: BLAKE3 over the three files, the pooling id and the
    /// two prefixes. Two encoders with one hash give one vector for one text.
    pub fn model_hash(&self) -> [u8; 32] {
        self.hash
    }

    /// The length of every vector this encoder returns.
    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn pooling(&self) -> Pooling {
        self.pooling
    }

    /// Embed each text, in order, as a query or as a passage. Every vector is L2-normalised.
    ///
    /// **One text a forward pass.** A batch pads its texts to one length, and a padded
    /// position, masked to zero weight, is still a term in every reduction a matmul makes —
    /// so a text's vector would depend on which texts it was batched with, and a cached
    /// vector would differ in its low bits from the one a re-embedding gives. One at a time,
    /// a vector is a function of its text and the model alone, which is what lets the cache
    /// stand in for the encoder.
    pub fn embed(&self, texts: &[String], role: TextRole) -> Result<Vec<Vec<f32>>, Error> {
        texts.iter().map(|t| self.embed_one(t, role)).collect()
    }

    pub(crate) fn embed_one(&self, text: &str, role: TextRole) -> Result<Vec<f32>, Error> {
        let prefix = match role {
            TextRole::Query => &self.query_prefix,
            TextRole::Passage => &self.passage_prefix,
        };
        let encoding = self
            .tokenizer
            .encode(format!("{prefix}{text}"), true)
            .map_err(|e| Error::Tokenizer(e.to_string()))?;
        let ids = encoding.get_ids();
        let mask = encoding.get_attention_mask();
        let model = |e: candle_core::Error| Error::Model(e.to_string());
        let input = Tensor::new(ids, &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(model)?;
        let types = input.zeros_like().map_err(model)?;
        let attention = Tensor::new(mask, &Device::Cpu)
            .and_then(|t| t.unsqueeze(0))
            .map_err(model)?;
        let states = match &self.net {
            Net::Bert(net) => net.forward(&input, &types, Some(&attention)),
            Net::XlmRoberta(net) => net.forward(&input, &attention, &types, None, None, None),
        }
        .and_then(|t| t.squeeze(0))
        .and_then(|t| t.to_dtype(DType::F32))
        .and_then(|t| t.to_vec2::<f32>())
        .map_err(model)?;
        Ok(self.pooling.pool(&states, mask))
    }

    fn rehash(&mut self) {
        let mut h = blake3::Hasher::new();
        h.update(b"smysl/s1-model/1\0");
        h.update(&self.files);
        for part in [self.pooling.id(), &self.query_prefix, &self.passage_prefix] {
            h.update(&(part.len() as u64).to_le_bytes());
            h.update(part.as_bytes());
        }
        self.hash = *h.finalize().as_bytes();
    }
}

/// BLAKE3 over the three files, each length-prefixed so no two splits of one byte string
/// hash alike.
fn files_digest(config: &[u8], tokenizer: &[u8], weights: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new();
    h.update(b"smysl/s1-files/1\0");
    for part in [config, tokenizer, weights] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }
    *h.finalize().as_bytes()
}
