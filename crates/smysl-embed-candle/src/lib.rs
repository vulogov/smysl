//! Transformer embeddings for a smysl store: S1 of RFC SMYSL-2.5 §3.14.
//!
//! S0 (`smysl-embed`) is a static table — a token maps to a vector. S1 runs an encoder: the
//! XLM-R family (`multilingual-e5-small`, `-base`) and `bge-m3`'s dense head, on candle, on the
//! CPU, loaded from a directory the operator supplies (`config.json`, `tokenizer.json`,
//! `model.safetensors`). No network, as for S0.
//!
//! **Nothing is built here yet.** TX-P10 step 1 verified the dependencies — versions, XLM-R
//! support, and a tree with no `-sys` crate and no `cc` — and this crate is where they are
//! declared, so that `xtask check-native` reads the tree the encoders will be built on rather
//! than a description of it. The encoders, pooling and the vector cache are step 3.
#![forbid(unsafe_code)]
