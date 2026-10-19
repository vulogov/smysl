//! A8, rate–distortion (SMYSL-2.6 §3.8): how much of a scope a pack loses at each fraction of
//! what carrying all of it costs — and **`full_tokens` and `floor_tokens`**, moved here from
//! `smysl-eval` (§4.3), which now re-exports them.
//!
//! - **Budgets** `bₖ = round(k/steps · full_tokens(scope))` for `k = 1 … steps`, rounded half
//!   to even in integers; `params.steps` is 20 by default, which is draft 3's fractions 0.05,
//!   0.10, …, 1.00 exactly.
//! - **Packs.** Each budget is one greedy `pack()` over the scope with the recipe's estimator
//!   and the default salience request, as E1's chain packs. A budget below the mandatory floor
//!   is a `PackError`, and the point is counted **refused** rather than recorded as distortion
//!   1, as `run_smysl_arm` records a refused hop: nothing was shipped because shipping it
//!   would have been one-sided.
//! - **Distortion** `D(b) = D_sal + λ·D_stat`: `D_sal` the salience mass the pack left out
//!   over the scope's, `D_stat` the status weight it left out over the scope's, the weight of a
//!   unit being its status's place on the ladder (speculative 1 … measured 5; an unfounded unit
//!   weighs 0, as `Store::is_unfounded` reads it). `λ = params.lambda`, ½ by default. A unit is
//!   kept when the pack selects it at any level.
//! - **Signature**: the curve, `D` at each admitted budget (`series.D`, with `series.budget`
//!   beside it), and its area by the trapezoid rule over consecutive admitted points against
//!   the fraction.
//!
//! # The curve is the least loss within a budget, not the last pack's
//!
//! §5.2 asks that D be non-increasing in budget, and **the greedy pack's own loss is not**: on
//! 12 of 534 curves over halves of `fixtures/corpus` a larger budget kept fewer of the scope's
//! units. The pack is greedy by value per token, and value counts detail: at 67 tokens it
//! ships a claim, its ground and its rebuttal at `L0`; at 68 the ground at `L1` is the densest
//! item, costs 68, and nothing else fits. That is the pack doing what it optimises, and a
//! distortion that counts units it left out is not what it optimises.
//!
//! A rate–distortion function is the least distortion at a rate **at most** R, and a pack
//! within a smaller budget is within a larger one, so `series.D` is the running minimum of the
//! packs' losses — non-increasing by definition, as the function is — and `series.D:packed`
//! is each budget's own pack, with `n.rises` the budgets where it lost more than an earlier
//! one. The area is over `series.D`. On a whole fixture the two series agree.
//!
//! # A scope cut from its rebuttals does not end at 0
//!
//! `full_tokens(scope)` is the scope's own units at their richest, and rule R will not ship a
//! claim without its rebuttal (`ClosureCost`). A scope that holds a claim and not the unit that
//! rebuts it cannot be carried whole for its own full cost, so the last point is above 0 —
//! a measure of how open the scope is, which is what a truncated pack would hide.
//!
//! **Exact.** The scores are `salience()`'s, quantised to 1/1024 (as A6 reads them), and the
//! weights are integers, so every `D(b)` is a ratio of integers over one denominator the whole
//! curve shares, and so is the area. Each is rounded once into the payload.
//!
//! **Refused on a `mul` scope with a byte estimator** (`SMY-E407`, draft 3 Appendix D). A scope
//! is `mul` when a part its units are drawn from is tagged `mul`, or when the parts are in more
//! than one language by primary subtag; a unit drawn from no part, or from a part tagged `und`,
//! says nothing either way. `smysl/utf8-div4` counts bytes, and a byte is a quarter of a token
//! in English and an eighth of one in Russian, so a budget over both would spend a different
//! number of tokens on each and call the difference distortion. Cross-lingual A8 waits for
//! FC-6's script-aware estimator (SMYSL-2.1).

use std::collections::BTreeSet;

use smysl_core::diag::Code;
use smysl_core::{Lod, Status, Uid};
use smysl_graph::salience::{salience, SalienceRequest};
use smysl_graph::Store;
use smysl_pack::{pack, Estimator, PackRequest};

use crate::graph;
use crate::graph::concentration::quantised;
use crate::recipe::Param;
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A8.
#[derive(Debug, Clone, Copy, Default)]
pub struct A8;

/// `params.steps`' default: twenty budgets, 5% apart.
pub const STEPS: i64 = 20;

/// `params.lambda`'s default: status loss weighs half what salience loss does.
pub const LAMBDA: (u32, u32) = (1, 2);

/// The cheapest way to carry every unit: all of them at `L0`, gist only.
///
/// This is the line below which units *have* to be dropped. Above it, a budget can be met
/// by shedding detail alone, so a unit going missing means something other than arithmetic
/// decided it - which is what makes this worth exposing rather than inlining.
pub fn floor_tokens(store: &Store, uids: &BTreeSet<Uid>, est: &Estimator) -> u64 {
    uids.iter()
        .filter_map(|u| store.get(u))
        .map(|u| est.unit(&u.core, Lod::L0))
        .sum()
}

/// The input's cost with nothing left behind: every unit at its richest available level.
/// This is the denominator E1 is a fraction of - what handing the whole graph over costs.
pub fn full_tokens(store: &Store, uids: &BTreeSet<Uid>, est: &Estimator) -> u64 {
    uids.iter()
        .filter_map(|u| store.get(u))
        .map(|u| {
            let level = smysl_pack::available_levels(&u.core)
                .last()
                .copied()
                .unwrap_or(Lod::L0);
            est.unit(&u.core, level)
        })
        .sum()
}

/// Whether an estimator counts bytes, and so cannot budget two scripts alike.
fn counts_bytes(est: &Estimator) -> bool {
    matches!(est, Estimator::Utf8Div4)
}

fn estimator(recipe: &Recipe) -> Result<Estimator, Refusal> {
    let id = recipe
        .estimator
        .as_deref()
        .unwrap_or(Estimator::Utf8Div4.id());
    Estimator::parse(id).ok_or_else(|| {
        let known: Vec<&str> = Estimator::ALL.iter().map(Estimator::id).collect();
        Refusal::new(
            Code::E489,
            format!("no estimator `{id}`; this build has {}", known.join(", ")),
        )
    })
}

fn steps(recipe: &Recipe) -> Result<u64, Refusal> {
    match recipe.params.get("steps") {
        Some(Param::Int(k)) if (1..=1000).contains(k) => Ok(*k as u64),
        _ => Err(Refusal::new(
            Code::E489,
            "A8's `steps` is an integer from 1 to 1000",
        )),
    }
}

fn lambda(recipe: &Recipe) -> Result<(u32, u32), Refusal> {
    match recipe.params.get("lambda") {
        Some(Param::Ratio(a, b)) if *b > 0 => Ok((*a, *b)),
        _ => Err(Refusal::new(
            Code::E489,
            "A8's `lambda` is a ratio with a positive denominator",
        )),
    }
}

/// The languages of the parts `uids` are drawn from, by primary subtag, `und` left out.
pub(crate) fn languages(store: &Store, uids: &[Uid]) -> BTreeSet<String> {
    let mut lang = std::collections::BTreeMap::new();
    for (mid, m) in store.manifests() {
        if store.is_superseded(mid) {
            continue;
        }
        for p in &m.parts {
            lang.entry(p.tid)
                .or_insert_with(|| p.lang.clone().unwrap_or_else(|| m.lang.clone()));
        }
    }
    uids.iter()
        .filter_map(|u| graph::part_of(store, u))
        .filter_map(|t| lang.get(&t))
        .map(|l| {
            let s = l.as_str();
            s.split('-').next().unwrap_or(s).to_ascii_lowercase()
        })
        .filter(|l| l != "und")
        .collect()
}

/// `SMY-E407` when a byte estimator would budget a scope in more than one language.
fn monolingual(store: &Store, uids: &[Uid], est: &Estimator) -> Result<(), Refusal> {
    let langs = languages(store, uids);
    if counts_bytes(est) && (langs.len() > 1 || langs.contains("mul")) {
        return Err(Refusal::new(
            Code::E407,
            format!(
                "A8 budgets in `{}`, which counts bytes, and the scope's units are drawn from \
                 text in {}; a byte is not the same share of a token in each, so the budgets \
                 would not be the same budget",
                est.id(),
                langs.into_iter().collect::<Vec<_>>().join(", ")
            ),
        ));
    }
    Ok(())
}

/// `num / den` with a zero denominator read as nothing to lose.
fn part(num: u64, den: u64) -> (i128, i128) {
    if den == 0 {
        (0, 1)
    } else {
        (i128::from(num), i128::from(den))
    }
}

/// One budget's outcome: the budget, and the numerator of its distortion over the curve's
/// shared denominator, or `None` when the pack refused it.
type Point = (u64, Option<i128>);

/// The whole curve, its shared denominator, and the scope's two token counts.
struct Curve {
    points: Vec<Point>,
    den: i128,
    full: u64,
    floor: u64,
}

fn curve(recipe: &Recipe, store: &Store, uids: &[Uid], est: &Estimator) -> Result<Curve, Refusal> {
    let steps = steps(recipe)?;
    let (la, lb) = lambda(recipe)?;
    let scope: BTreeSet<Uid> = uids.iter().copied().collect();
    let full = full_tokens(store, &scope, est);
    let floor = floor_tokens(store, &scope, est);
    let report = salience(store, &SalienceRequest::default());

    let mut q = Vec::with_capacity(uids.len());
    let mut w = Vec::with_capacity(uids.len());
    for u in uids {
        q.push(quantised(&report, u).ok_or_else(|| {
            Refusal::new(
                Code::E489,
                format!("a salience score of {u} is not on the 1/1024 grid"),
            )
        })?);
        w.push(u64::from(
            graph::epistemic::status_of(store, u).map_or(0, Status::as_u8),
        ));
    }
    let (s_total, t_total): (u64, u64) = (q.iter().sum(), w.iter().sum());
    let ((_, ds), (_, dt)) = (part(0, s_total), part(0, t_total));
    // D = ls/S + (a/b)·lt/T over the one denominator S·b·T.
    let den = ds * i128::from(lb) * dt;

    let mut points = Vec::with_capacity(steps as usize);
    for k in 1..=steps {
        let budget = half_even(u128::from(full) * u128::from(k), u128::from(steps));
        let mut req = PackRequest::default();
        req.budget = budget;
        req.estimator = est.clone();
        req.scope = scope.clone();
        let Ok(p) = pack(store, &report, &req) else {
            points.push((budget, None));
            continue;
        };
        let (mut ls, mut lt) = (0u64, 0u64);
        for (i, u) in uids.iter().enumerate() {
            if !p.selection.contains_key(u) {
                ls += q[i];
                lt += w[i];
            }
        }
        let (ns, _) = part(ls, s_total);
        let (nt, _) = part(lt, t_total);
        let num = ns * i128::from(lb) * dt + i128::from(la) * nt * ds;
        points.push((budget, Some(num)));
    }
    Ok(Curve {
        points,
        den,
        full,
        floor,
    })
}

/// `num / den` rounded half to even, in integers. `den` positive.
fn half_even(num: u128, den: u128) -> u64 {
    let (q, r) = (num / den, num % den);
    let q = if 2 * r > den || (2 * r == den && q % 2 != 0) {
        q + 1
    } else {
        q
    };
    u64::try_from(q).unwrap_or(u64::MAX)
}

impl Measure for A8 {
    fn id(&self) -> &'static str {
        "A8"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::SALIENCE).union(Needs::PACK)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
        recipe
            .estimator
            .get_or_insert_with(|| Estimator::Utf8Div4.id().to_string());
        recipe
            .params
            .entry("steps".to_string())
            .or_insert(Param::Int(STEPS));
        recipe
            .params
            .entry("lambda".to_string())
            .or_insert(Param::Ratio(LAMBDA.0, LAMBDA.1));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        let est = estimator(recipe)?;
        steps(recipe)?;
        lambda(recipe)?;
        let uids = graph::admitted(recipe, input)?;
        monolingual(input.store, &uids, &est)
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let run = || -> Result<Value, Refusal> {
            let est = estimator(recipe)?;
            let uids = graph::admitted(recipe, input)?;
            monolingual(input.store, &uids, &est)?;
            let c = curve(recipe, input.store, &uids, &est)?;
            let steps = c.points.len() as i128;
            let fixed = |num: i128| {
                graph::ratio_q20(num, c.den).map(Num::Fixed).ok_or_else(|| {
                    Refusal::new(Code::E489, "a distortion does not fit a payload number")
                })
            };
            // The curve is the rate–distortion function, the least loss of any pack within a
            // budget, and a pack within a smaller budget is within a larger one: the running
            // minimum of what each budget's pack lost. The packs themselves are beside it.
            let mut best: Option<i128> = None;
            let curve: Vec<Option<i128>> = c
                .points
                .iter()
                .map(|(_, num)| {
                    num.map(|n| {
                        let m = best.map_or(n, |b| b.min(n));
                        best = Some(m);
                        m
                    })
                })
                .collect();
            let (mut d, mut packed, mut budgets) = (Vec::new(), Vec::new(), Vec::new());
            let (mut refused, mut rises) = (0u64, 0u64);
            for ((b, num), env) in c.points.iter().zip(&curve) {
                match (num, env) {
                    (Some(n), Some(e)) => {
                        d.push(fixed(*e)?);
                        packed.push(fixed(*n)?);
                        budgets.push(Num::Count(*b));
                        rises += u64::from(n > e);
                    }
                    _ => refused += 1,
                }
            }
            // The trapezoids between consecutive admitted points, each 1/steps wide: the
            // sum of (Dₖ + Dₖ₊₁) over 2·steps·den.
            let mut area = None;
            for pair in curve.windows(2) {
                if let (Some(a), Some(b)) = (pair[0], pair[1]) {
                    *area.get_or_insert(0i128) += a + b;
                }
            }
            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("full:tokens", c.full)
                .with_n("floor:tokens", c.floor)
                .with_n("points", c.points.len() as u64)
                .with_n("refused", refused)
                .with_n("rises", rises)
                .with_series("D", d)
                .with_series("D:packed", packed)
                .with_series("budget", budgets);
            match area {
                Some(a) => {
                    let x = graph::ratio_q20(a, 2 * steps * c.den).ok_or_else(|| {
                        Refusal::new(Code::E489, "the area does not fit a payload number")
                    })?;
                    v = v.with_value("area", Num::Fixed(x)).with_headline_in(
                        "area",
                        "rate–distortion area",
                        "",
                    );
                }
                None => {
                    return Err(Refusal::new(
                        Code::W485,
                        format!(
                            "A8 needs two budgets the pack admits to draw a curve, and `{}` \
                             gave {}",
                            recipe.scope.trim(),
                            c.points.len() as u64 - refused
                        ),
                    ));
                }
            }
            Ok(v)
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
