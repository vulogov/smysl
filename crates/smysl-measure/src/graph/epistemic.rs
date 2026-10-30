//! A4, the epistemic profile (SMYSL-2.6 §3.8): how a scope's units spread over the six
//! statuses, and how much of what it holds is below `cited`.
//!
//! The status a unit is counted under is the one it has now: its core's, unless a retraction
//! has made it **unfounded** (`Store::is_unfounded`), which no unit is authored as (`SMY-E034`)
//! and every retracted one is. So a store that retracted half its claims reads half unfounded,
//! which is what it holds.
//!
//! `H(status) = −Σ pₛ log₂ pₛ` over the six, in ladder order, from 0 (one status) to log₂ 6.
//! The **hedging rate** is the ratio `[units below cited, units]`: what the scope asserts on
//! less than a citation.

use smysl_core::Status;

use crate::graph;
use crate::num::{log2, q20, sum};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A4.
#[derive(Debug, Clone, Copy, Default)]
pub struct A4;

/// The status a unit is counted under.
pub(crate) fn status_of(store: &smysl_graph::Store, uid: &smysl_core::Uid) -> Option<Status> {
    let unit = store.get(uid)?;
    Some(if store.is_unfounded(uid) {
        Status::Unfounded
    } else {
        unit.core.status
    })
}

/// The six counts, in ladder order.
pub(crate) fn counts(store: &smysl_graph::Store, uids: &[smysl_core::Uid]) -> [u64; 6] {
    let mut c = [0u64; 6];
    for u in uids {
        if let Some(s) = status_of(store, u) {
            c[s.as_u8() as usize] += 1;
        }
    }
    c
}

/// `−Σ p log₂ p` over counts, in their order; `None` for none.
pub(crate) fn entropy(counts: &[u64]) -> Option<f64> {
    let n: u64 = counts.iter().sum();
    if n == 0 {
        return None;
    }
    let nf = n as f64;
    Some(-sum(counts.iter().filter(|&&c| c > 0).map(|&c| {
        let p = c as f64 / nf;
        p * log2(p)
    })))
}

impl Measure for A4 {
    fn id(&self) -> &'static str {
        "A4"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS
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
        let c = counts(input.store, &uids);
        let units: u64 = c.iter().sum();
        let Some(h) = entropy(&c).and_then(q20) else {
            return Outcome::NotRun {
                why: "the status entropy was not a finite number".into(),
                code: None,
            };
        };
        let below = c[..Status::Cited.as_u8() as usize].iter().sum();
        let mut v = Value::new()
            .with_n("units", units)
            .with_value("H:status", Num::Fixed(h))
            .with_value("hedging", Num::Ratio(below, units))
            .with_headline_in("H:status", "status entropy", "bits");
        for s in Status::ALL {
            v = v.with_value(
                format!("status:{}", s.as_str()),
                Num::Count(c[s.as_u8() as usize]),
            );
        }
        Outcome::Value(v)
    }
}
