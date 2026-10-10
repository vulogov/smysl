//! The check passes of §17, one module each.
//!
//! Every pass takes the store and a `Report` and appends to it. None of them return a
//! result, because none of them may stop the pipeline.

pub mod closure;
pub mod commitment;
pub mod epistemics;
pub mod extension;
pub mod granularity;
pub mod integrity;
pub mod library;
pub mod shape;
/// Pass 13, behind the `text` feature: rule E's engine lives in `smysl-text`.
#[cfg(feature = "text")]
pub mod time;
pub mod trust;
