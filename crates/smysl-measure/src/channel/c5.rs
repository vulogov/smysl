//! C5, compression across the channel (SMYSL-2.6 §3.9): how much the target's text says per
//! character beside how much its source's does.
//!
//! `T2(T)/T2(S)` and `T3(T)/T3(S)` as fixed nums, each side measured by this crate's own T2
//! and T3 under one recipe — the codec, the level and T3's `symbols` are C5's, so the two
//! sides cannot be measured differently — and each ratio taken **from the two stored
//! integers**, so it is the ratio of the figures a reader of the two T2 units would see. Read
//! beside C1: a target that dropped half its source compresses differently for a reason no
//! ratio of rates shows.
//!
//! **T3 is reported when both sides admit it.** Its `min:n` is 50,000 symbols in every text, its
//! own default, which a short channel does not reach; C5 then reports T2's ratio alone and is
//! flagged `t3-not-run`, rather than refused for a figure beside the one it has. C5's own
//! `min:n` is T2's, in characters.
//!
//! A source and target that share a part are refused (`SMY-E489`), as a channel's units are.

use crate::channel;
use crate::graph::ratio_q20;
use crate::text::{compress, entropy::corpus, lz_match};
use crate::{
    run_with, Corpus, Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value, T2, T3,
};

use smysl_core::diag::Code;

/// C5.
#[derive(Debug, Clone, Copy, Default)]
pub struct C5;

/// The flag a C5 result carries when T3 did not run on both sides.
pub const T3_NOT_RUN: &str = "t3-not-run";

/// The target's text, or the refusal.
fn target<'a>(recipe: &Recipe, input: &Input<'a>) -> Result<&'a Corpus, Refusal> {
    let t = input
        .target_text
        .ok_or_else(|| channel::no_target(recipe))?;
    let s = corpus(input)?;
    let shared = s.tids.intersection(&t.tids).count();
    if shared > 0 {
        return Err(Refusal::new(
            Code::E489,
            format!(
                "a channel's source and target share {shared} part(s); a text on both sides \
                 would be compared with itself"
            ),
        ));
    }
    Ok(t)
}

/// One side's T2 or T3 under C5's recipe: its headline's stored integer, or the refusal.
fn side(
    m: &dyn Measure,
    recipe: &Recipe,
    store: &smysl_graph::Store,
    scope: &crate::Resolved,
    text: &Corpus,
    min_n: Option<u64>,
) -> Result<i64, Refusal> {
    let mut base = recipe.for_measure("", 0);
    // The analyzer was stamped from the source's text, and the target may be in another
    // language: each side is read under its own, which a character count does not see.
    base.analyzer = None;
    base.channel = None;
    base.channel_digest = None;
    base.min_n = min_n;
    let mut input = Input::new(store, scope).with_text(text);
    input.target_text = None;
    let r = run_with(&[m], &[m.id()], &base, &input).remove(0);
    match r.outcome {
        Outcome::Value(v) => match v.headline.and_then(|h| v.values.get(&h.key).copied()) {
            Some(Num::Fixed(q)) => Ok(q),
            _ => Err(Refusal::new(Code::E489, format!("{} gave no rate", m.id()))),
        },
        Outcome::NotRun { why, code } => Err(Refusal::new(code.unwrap_or(Code::E489), why)),
    }
}

impl Measure for C5 {
    fn id(&self) -> &'static str {
        "C5"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::TEXT.union(Needs::CHANNEL)
    }

    fn complete(&self, recipe: &mut Recipe) {
        recipe.min_n.get_or_insert(compress::MIN_N);
        recipe
            .codec
            .get_or_insert_with(|| compress::CODEC.to_string());
        recipe
            .params
            .entry(lz_match::SYMBOLS.to_string())
            .or_insert(crate::Param::Text(lz_match::CHARS.to_string()));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        compress::codec(recipe)?;
        let t = target(recipe, input)?;
        let s = corpus(input)?;
        side(&T2, recipe, input.store, input.scope, s, recipe.min_n)?;
        side(&T2, recipe, input.store, input.scope, t, recipe.min_n)?;
        Ok(())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let (s, t) = match target(recipe, input).and_then(|t| Ok((corpus(input)?, t))) {
            Ok(x) => x,
            Err(r) => return Outcome::refused(r),
        };
        let at = |m: &dyn Measure, c, n| side(m, recipe, input.store, input.scope, c, n);
        let (s2, t2) =
            match at(&T2, s, recipe.min_n).and_then(|a| Ok((a, at(&T2, t, recipe.min_n)?))) {
                Ok(x) => x,
                Err(r) => return Outcome::refused(r),
            };
        let Some(r2) = ratio_q20(i128::from(t2), i128::from(s2)) else {
            return compress::refused_codec();
        };
        let mut v = Value::new()
            .with_n("source:chars", chars(s))
            .with_n("target:chars", chars(t))
            .with_value("T2:source", Num::Fixed(s2))
            .with_value("T2:target", Num::Fixed(t2))
            .with_value("T2:ratio", Num::Fixed(r2))
            .with_headline_in("T2:ratio", "brotli rate ratio", "");
        let t3 = at(&T3, s, None).and_then(|a| Ok((a, at(&T3, t, None)?)));
        match t3
            .ok()
            .and_then(|(a, b)| Some((a, b, ratio_q20(i128::from(b), i128::from(a))?)))
        {
            Some((s3, t3, r3)) => {
                v = v
                    .with_value("T3:source", Num::Fixed(s3))
                    .with_value("T3:target", Num::Fixed(t3))
                    .with_value("T3:ratio", Num::Fixed(r3));
            }
            None => v = v.with_flag(T3_NOT_RUN),
        }
        if t.lossy && !s.lossy {
            v = v.with_flag(crate::text::LOSSY_ADMITTED);
        }
        Outcome::Value(v)
    }
}

fn chars(c: &Corpus) -> u64 {
    c.docs
        .iter()
        .flat_map(|d| &d.segments)
        .map(|s| s.chars().count() as u64)
        .sum()
}
