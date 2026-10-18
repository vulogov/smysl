//! T2, compression rate (SMYSL-2.6 §3.7), and the codec T6 shares.
//!
//! `brotli` at quality 11 and `lgwin` 24 over each expression's NFC UTF-8, its segments joined
//! by `\n`: `bits/char = 8·ΣCᵢ / Σcharsᵢ` and `bits/word = 8·ΣCᵢ / words` over the scope's
//! expressions, which sums bits and characters rather than averaging rates (§3.6). An
//! expression longer than the window is compressed in window-sized pieces at segment
//! boundaries, the bits summed, and the result flagged `windowed`.
//!
//! # The codec
//!
//! [`CODEC`] names the encoder, its version and its build: `brotli-rs/8.0.4 no-std q11 w24`.
//! The build is in the name because it changes the output. With its `std` feature the encoder's
//! cost model reads `f32::log2` from the platform for every count past 255, and a cost one bit
//! off can choose another block split; without it the same function is a table and integer
//! shifts, and the encoder's floating point is IEEE arithmetic only, which every target this
//! crate builds for rounds alike. So the crate takes `brotli` without `std` and supplies the
//! allocator `std` would have brought ([`Heap`]): a boxed slice per cell, safe code.
//!
//! The input is handed to the encoder whole, in one `FINISH`, so the length cannot depend on
//! how a reader happened to chunk it.

use brotli::enc::encode::{
    BrotliEncoderOperation, BrotliEncoderParameter, BrotliEncoderStateStruct,
};
use brotli::enc::{Allocator, BrotliAlloc, SliceWrapper, SliceWrapperMut};

use crate::num::q20;
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::Recipe;
use crate::text::entropy::corpus;
use crate::{Input, Measure, Needs, Purity};

/// The encoder, its version and its build, and the two parameters: the recipe's `codec`.
pub const CODEC: &str = "brotli-rs/8.0.4 no-std q11 w24";

/// Brotli's quality.
pub const QUALITY: u32 = 11;

/// Brotli's window, as a base-2 logarithm.
pub const LGWIN: u32 = 24;

/// The window in bytes: brotli's `(1 << lgwin) − 16`. A compressor sees no further back.
pub const WINDOW: usize = (1 << LGWIN) - 16;

/// The flag a result carries when an expression was compressed in more than one piece.
pub const WINDOWED: &str = "windowed";

/// T2's `min:n`, in characters: §3.7 gives T2 none, and a rate over no characters is `0/0`.
pub const MIN_N: u64 = 1;

/// One allocation: a boxed slice.
#[derive(Debug)]
pub struct Cells<T>(Box<[T]>);

impl<T> Default for Cells<T> {
    fn default() -> Cells<T> {
        Cells(Box::new([]))
    }
}

impl<T> SliceWrapper<T> for Cells<T> {
    fn slice(&self) -> &[T] {
        &self.0
    }
}

impl<T> SliceWrapperMut<T> for Cells<T> {
    fn slice_mut(&mut self) -> &mut [T] {
        &mut self.0
    }
}

/// The allocator brotli's `std` feature would have brought: every cell zeroed by `Default`,
/// freed by drop.
#[derive(Debug, Clone, Copy, Default)]
pub struct Heap;

impl<T: Clone + Default> Allocator<T> for Heap {
    type AllocatedMemory = Cells<T>;

    fn alloc_cell(&mut self, len: usize) -> Cells<T> {
        Cells(vec![T::default(); len].into_boxed_slice())
    }

    fn free_cell(&mut self, _cell: Cells<T>) {}
}

impl BrotliAlloc for Heap {}

/// The brotli stream of `data` under [`CODEC`], written to `out` (cleared first); `None` if
/// the encoder refuses, which it does only for parameters it does not support.
pub fn compress_into(data: &[u8], out: &mut Vec<u8>) -> Option<()> {
    let mut s = BrotliEncoderStateStruct::new(Heap);
    if !s.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_QUALITY, QUALITY)
        || !s.set_parameter(BrotliEncoderParameter::BROTLI_PARAM_LGWIN, LGWIN)
    {
        return None;
    }
    out.clear();
    let mut buf = vec![0u8; 1 << 16];
    let mut available_in = data.len();
    let mut in_offset = 0;
    let mut total_out = Some(0);
    let mut nop =
        |_: &mut brotli::interface::PredictionModeContextMap<brotli::InputReferenceMut>,
         _: &mut [brotli::interface::StaticCommand],
         _: brotli::InputPair,
         _: &mut Heap| ();
    loop {
        let mut available_out = buf.len();
        let mut out_offset = 0;
        if !s.compress_stream(
            BrotliEncoderOperation::BROTLI_OPERATION_FINISH,
            &mut available_in,
            data,
            &mut in_offset,
            &mut available_out,
            &mut buf,
            &mut out_offset,
            &mut total_out,
            &mut nop,
        ) {
            return None;
        }
        out.extend_from_slice(&buf[..buf.len() - available_out]);
        if s.is_finished() {
            return Some(());
        }
    }
}

/// `C(data)`: the length in bytes of its brotli stream under [`CODEC`].
pub fn compressed_len(data: &[u8]) -> Option<u64> {
    let mut out = Vec::new();
    compress_into(data, &mut out)?;
    Some(out.len() as u64)
}

/// An expression's segments as the pieces a compressor reads: joined by `\n` while they fit
/// the window, a new piece at the segment boundary that would not, and a segment longer than
/// the window cut at the last character boundary that fits.
pub fn pieces(segments: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for seg in segments {
        let mut rest = seg.as_str();
        if !cur.is_empty() && cur.len() + 1 + rest.len() <= WINDOW {
            cur.push('\n');
            cur.push_str(rest);
            continue;
        }
        if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        while rest.len() > WINDOW {
            let mut cut = WINDOW;
            while !rest.is_char_boundary(cut) {
                cut -= 1;
            }
            out.push(rest[..cut].to_string());
            rest = &rest[cut..];
        }
        cur.push_str(rest);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// A recipe naming a codec this build does not have cannot be recomputed here.
pub(crate) fn codec(recipe: &Recipe) -> Result<(), Refusal> {
    match recipe.codec.as_deref() {
        Some(CODEC) => Ok(()),
        other => Err(Refusal::new(
            smysl_core::diag::Code::E489,
            format!(
                "{}'s codec is `{CODEC}` and the recipe names `{}`",
                recipe.id,
                other.unwrap_or("none")
            ),
        )),
    }
}

/// T2.
#[derive(Debug, Clone, Copy, Default)]
pub struct T2;

impl Measure for T2 {
    fn id(&self) -> &'static str {
        "T2"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::TEXT
    }

    fn complete(&self, recipe: &mut Recipe) {
        recipe.min_n.get_or_insert(MIN_N);
        recipe.codec.get_or_insert_with(|| CODEC.to_string());
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        codec(recipe)?;
        let c = corpus(input)?;
        let want = recipe.min_n.unwrap_or(0);
        let have: u64 = c
            .docs
            .iter()
            .flat_map(|d| &d.segments)
            .map(|s| s.chars().count() as u64)
            .sum();
        if have < want.max(1) {
            return Err(Refusal::new(
                smysl_core::diag::Code::W485,
                format!(
                    "T2 needs at least {} characters and `{}` has {have}",
                    want.max(1),
                    recipe.scope.trim()
                ),
            ));
        }
        Ok(())
    }

    fn run(&self, _recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let (mut bits, mut chars, mut bytes, mut count, mut segments) = (0u64, 0u64, 0u64, 0, 0);
        let mut windowed = false;
        let mut out = Vec::new();
        for d in &c.docs {
            segments += d.segments.len() as u64;
            let ps = pieces(&d.segments);
            windowed |= ps.len() > 1;
            for p in &ps {
                if compress_into(p.as_bytes(), &mut out).is_none() {
                    return refused_codec();
                }
                bits += 8 * out.len() as u64;
                chars += p.chars().count() as u64;
                bytes += p.len() as u64;
                count += 1;
            }
        }
        let words = c.tokens();
        let Some(per_char) = q20(bits as f64 / chars as f64) else {
            return refused_codec();
        };
        let mut value = Value::new()
            .with_n("chars", chars)
            .with_n("bytes", bytes)
            .with_n("words", words)
            .with_n("compressed", bits / 8)
            .with_n("texts", c.docs.len() as u64)
            .with_n("segments", segments)
            .with_n("pieces", count)
            .with_value("bits/char", Num::Fixed(per_char))
            .with_headline("bits/char", "brotli rate");
        if words > 0 {
            if let Some(per_word) = q20(bits as f64 / words as f64) {
                value = value.with_value("bits/word", Num::Fixed(per_word));
            }
        }
        if windowed {
            value = value.with_flag(WINDOWED);
        }
        Outcome::Value(value)
    }
}

pub(crate) fn refused_codec() -> Outcome {
    Outcome::NotRun {
        why: "the compressor refused its own parameters".into(),
        code: None,
    }
}
