//! Text ingest: what a model is shown, and where its answer is anchored (SMYSL-2.4 §3.4).
//!
//! Two modules, and the division between them is the one the RFC draws. [`window`](crate::text::window) decides
//! **what text one call sees**: whole structure nodes at a chosen level, filled to a budget,
//! never splitting a node except at sentence boundaries when a node is larger than the window
//! itself (`SMY-W447`). [`attach`](mod@crate::text::attach) decides **where an answer landed**: it takes the quote a unit
//! attributed itself to and finds it in the window, in a fixed scope order, so the unit carries a
//! byte range into a part rather than a model's word for where it read.
//!
//! # Why a span and not a locator
//!
//! A locator is a name in a reading — `3:16`, `#/messages/4` — and names move: a reader upgraded
//! under a corpus renumbers them, and a unit naming one is then a claim about a place nobody can
//! find. A span is a pair of byte offsets into a part whose identity is the hash of its bytes, so
//! it either resolves or the part is not the part. That is why FC-3 put the span inside the uid
//! and left the locator informative (A-2.1 key 5, `SMY-W405`), and why attachment is a step of
//! its own rather than a field the prompt asks for: **the model is never asked for a span.** It
//! is asked for a quote, which is checkable, and the span is computed from it here.
//!
//! # What this module does not do
//!
//! It does not call a model, score an answer, or decide what to keep. The ledger digest, the
//! journal, the prompt templates and `ingest --text` itself are TX-P5 step 4; nothing here
//! persists anything or reaches a provider. Windowing and attachment are pure functions over a
//! reading and a batch, which is also what makes them testable without egress.

pub mod attach;
pub mod attribution;
pub mod digest;
pub mod journal;
#[cfg(feature = "model")]
pub mod run;
pub mod window;

pub use attach::{attach, Attached, Attachment, Opts};
pub use attribution::{attribution, quoted_spans, Attribution};
pub use digest::{digest, Digest};
pub use journal::Journal;
#[cfg(feature = "model")]
pub use run::{Target, TextIngest, TextReport};
pub use window::{windows, Pane, Placed, TextWindow};
