//! A10, dating (SMYSL-2.6 §3.8): how well a scope is placed in time on the recipe's axis.
//!
//! Every unit falls in exactly one of five shares, by what rule E (`smysl_text::time`) says
//! set its effective interval:
//!
//! - **contested**: the constraints over it cannot all hold, so nothing was chosen;
//! - **undated**: neither bound is set;
//! - **dating**: a documentary dating (record 17) or its offset set a bound;
//! - **constraint**: a free constraint set a bound and no dating did;
//! - **recorded**: the bounds are the unit's own `published` or `observed`.
//!
//! The order is the order of precedence: a unit a dating moved is dated by documentary
//! evidence even where its other bound is its own record. Each share is `[k, units]`.
//!
//! **Widths** are over units with both bounds set, `hi − lo` in milliseconds, written in days
//! as a fixed-point number rounded once: the median (the lower one) and the nearest-rank 90th
//! percentile. An instant is width 0, a year about 365.
//!
//! Per text as well as across the scope, as A2 and A7 are.

use smysl_core::Uid;
use smysl_text::time::engine::{effective, Cause, Effective};
use smysl_text::time::{Bound, Subject};

use crate::graph::{self, novelty};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A10.
#[derive(Debug, Clone, Copy, Default)]
pub struct A10;

/// Milliseconds in a day.
pub const DAY_MS: u64 = 86_400_000;

/// The five shares, in report order.
pub const SHARES: [&str; 5] = ["contested", "undated", "dating", "constraint", "recorded"];

/// Which share a unit is in, as an index into [`SHARES`].
pub fn share(eff: &Effective, uid: &Uid) -> usize {
    let Some(d) = eff.of(&Subject::Unit(*uid)) else {
        return 1;
    };
    if d.contested {
        return 0;
    }
    if d.interval.is_undated() {
        return 1;
    }
    let causes = [d.why_lo, d.why_hi];
    let by = |f: fn(&Cause) -> bool| causes.iter().flatten().any(|w| f(&w.cause));
    if by(|c| matches!(c, Cause::Dating(_) | Cause::Offset(_))) {
        2
    } else if by(|c| matches!(c, Cause::Free(_))) {
        3
    } else {
        4
    }
}

/// `hi − lo` in milliseconds, for a unit with both bounds.
fn width(eff: &Effective, uid: &Uid) -> Option<u64> {
    let iv = eff.interval(&Subject::Unit(*uid));
    match (iv.lo, iv.hi) {
        (Bound::At(lo), Bound::At(hi)) if hi.ms() >= lo.ms() => Some(hi.ms().abs_diff(lo.ms())),
        _ => None,
    }
}

fn days(ms: u64) -> Option<Num> {
    graph::scaled(ms, 1, DAY_MS).map(Num::Fixed)
}

fn tally(eff: &Effective, uids: &[Uid], prefix: &str, mut v: Value) -> Value {
    let mut counts = [0u64; 5];
    let mut widths: Vec<u64> = Vec::new();
    for u in uids {
        counts[share(eff, u)] += 1;
        widths.extend(width(eff, u));
    }
    let units = uids.len() as u64;
    for (k, name) in SHARES.iter().enumerate() {
        v = v.with_value(format!("{name}{prefix}"), Num::Ratio(counts[k], units));
    }
    widths.sort_unstable();
    if let Some(x) = graph::median(&widths).and_then(days) {
        v = v.with_value(format!("width:median{prefix}"), x);
    }
    if let Some(x) = graph::nearest_rank(&widths, 9, 10).and_then(days) {
        v = v.with_value(format!("width:p90{prefix}"), x);
    }
    v
}

impl Measure for A10 {
    fn id(&self) -> &'static str {
        "A10"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::TIME)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let uids = match graph::admitted(recipe, input) {
            Ok(u) => u,
            Err(r) => return Outcome::refused(r),
        };
        let store = input.store;
        let eff = match novelty::axis(recipe) {
            Ok(a) => effective(store, a),
            Err(r) => return Outcome::refused(r),
        };
        let mut v = Value::new()
            .with_n("units", uids.len() as u64)
            .with_headline_in("undated", "undated share", "");
        v = tally(&eff, &uids, "", v);
        for (alias, members) in graph::by_text(store, &uids) {
            v = tally(&eff, &members, &format!(":{alias}"), v);
        }
        Outcome::Value(v)
    }
}
