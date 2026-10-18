//! What a measure returns, and the payload a value becomes (SMYSL-2.6 §3.6).
//!
//! **No payload floats.** `smysl_core`'s payload encoding writes a float as binary32 quantised
//! to 1/1024, which would store a JSD of 0.0123 as 0.0127 and lose integers past 2¹⁴. A result
//! is therefore a [`Num`], one of three integer shapes, and the shape says what kind of number
//! it is:
//!
//! | shape | CBOR | for |
//! |---|---|---|
//! | count | an integer | sizes, tallies |
//! | ratio | `[num, den]`, never reduced | proportions of counts, stored exactly |
//! | fixed | `{q20: i}`, value `i · 2⁻²⁰` | entropies, exponents, divergences |
//!
//! **Not run is not a unit.** A refusal is a diagnostic and a line in a report; a unit asserts
//! a value, and a unit with no value would assert nothing (§3.1 deviation 2).

use std::collections::BTreeMap;
use std::fmt;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::surface::hjson::{HObject, HValue};
use smysl_core::surface::object_to_payload;

use crate::num::{render_q20, render_ratio};
use crate::recipe::{at, int, key, ratio, Recipe, RecipeError};

/// A payload number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Num {
    Count(u64),
    /// Numerator and denominator, as counted.
    Ratio(u64, u64),
    /// `i · 2⁻²⁰`; see [`crate::num::q20`].
    Fixed(i64),
}

impl Num {
    /// The CBOR shape of the table above.
    pub fn to_hvalue(&self) -> Result<HValue, RecipeError> {
        Ok(match *self {
            Num::Count(n) => HValue::Int(int(n, "a count")?),
            Num::Ratio(n, d) => ratio(n, d)?,
            Num::Fixed(i) => {
                let mut o = HObject::new();
                o.insert(key("q20"), at(HValue::Int(i)));
                HValue::Object(o)
            }
        })
    }

    /// The number as a gist writes it, from the integers: four places for a fixed or a ratio,
    /// and a ratio's counts beside it, since the counts are what was measured.
    pub fn render(&self) -> String {
        match *self {
            Num::Count(n) => n.to_string(),
            Num::Ratio(n, d) => format!(
                "{} ({n}/{d})",
                render_ratio(i128::from(n), i128::from(d), 4)
            ),
            Num::Fixed(i) => render_q20(i, 4),
        }
    }
}

impl fmt::Display for Num {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.render())
    }
}

/// A bootstrap interval around one result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ci {
    pub lo: Num,
    pub hi: Num,
    /// The coverage, as a ratio: `(95, 100)`.
    pub level: (u32, u32),
    /// Replicates drawn.
    pub b: u32,
}

/// A measure's value: what goes under `measure:n`, `measure:value`, `measure:ci`,
/// `measure:series` and `measure:flags`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Value {
    /// Sample sizes: `chars`, `words`, `segments`, `units`, `classes`.
    pub n: BTreeMap<String, u64>,
    /// Named results.
    pub values: BTreeMap<String, Num>,
    pub ci: BTreeMap<String, Ci>,
    /// Arrays: a curve, a per-segment top-k.
    pub series: BTreeMap<String, Vec<Num>>,
    /// `lossy-admitted`, `low-n`, `stem-level`. Written sorted, so the order a measure raised
    /// them in does not reach the uid.
    pub flags: Vec<String>,
    /// Which of `values` the gist names, and the label it is written with.
    pub headline: Option<(String, String)>,
}

impl Value {
    pub fn new() -> Value {
        Value::default()
    }

    pub fn with_n(mut self, k: impl Into<String>, n: u64) -> Value {
        self.n.insert(k.into(), n);
        self
    }

    pub fn with_value(mut self, k: impl Into<String>, v: Num) -> Value {
        self.values.insert(k.into(), v);
        self
    }

    pub fn with_ci(mut self, k: impl Into<String>, ci: Ci) -> Value {
        self.ci.insert(k.into(), ci);
        self
    }

    pub fn with_series(mut self, k: impl Into<String>, s: Vec<Num>) -> Value {
        self.series.insert(k.into(), s);
        self
    }

    pub fn with_flag(mut self, f: impl Into<String>) -> Value {
        self.flags.push(f.into());
        self
    }

    /// The result the gist names: `key` must be one of `values`, and `label` is the phrase in
    /// front of it, `brotli rate`.
    pub fn with_headline(mut self, key: impl Into<String>, label: impl Into<String>) -> Value {
        self.headline = Some((key.into(), label.into()));
        self
    }
}

/// A measure that declined before computing, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    pub message: String,
}

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Refusal {
        Refusal {
            code,
            message: message.into(),
        }
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(self.code).with_message(self.message.clone())
    }
}

/// What running a measure gave.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Value(Value),
    /// Reported, never persisted. `code` is the diagnostic when the reason has one.
    NotRun {
        why: String,
        code: Option<Code>,
    },
}

impl Outcome {
    pub fn refused(r: Refusal) -> Outcome {
        Outcome::NotRun {
            why: r.message,
            code: Some(r.code),
        }
    }
}

/// The payload of a measure unit: the six `measure:*` keys of §3.6, in canonical CBOR.
pub fn to_payload(recipe: &Recipe, value: &Value) -> Result<Vec<u8>, RecipeError> {
    let mut o = HObject::new();
    o.insert(key("measure:id"), at(HValue::Str(recipe.id.clone())));
    o.insert(key("measure:v"), at(HValue::Int(i64::from(recipe.v))));
    o.insert(
        key("measure:recipe"),
        at(HValue::Object(recipe.to_object()?)),
    );

    let mut n = HObject::new();
    for (k, v) in &value.n {
        n.insert(key(k), at(HValue::Int(int(*v, k)?)));
    }
    o.insert(key("measure:n"), at(HValue::Object(n)));

    let mut vals = HObject::new();
    for (k, v) in &value.values {
        vals.insert(key(k), at(v.to_hvalue()?));
    }
    o.insert(key("measure:value"), at(HValue::Object(vals)));

    if !value.ci.is_empty() {
        let mut ci = HObject::new();
        for (k, c) in &value.ci {
            let mut one = HObject::new();
            one.insert(key("lo"), at(c.lo.to_hvalue()?));
            one.insert(key("hi"), at(c.hi.to_hvalue()?));
            one.insert(
                key("level"),
                at(ratio(u64::from(c.level.0), u64::from(c.level.1))?),
            );
            one.insert(key("b"), at(HValue::Int(i64::from(c.b))));
            ci.insert(key(k), at(HValue::Object(one)));
        }
        o.insert(key("measure:ci"), at(HValue::Object(ci)));
    }
    if !value.series.is_empty() {
        let mut series = HObject::new();
        for (k, s) in &value.series {
            let items = s
                .iter()
                .map(|n| n.to_hvalue().map(at))
                .collect::<Result<Vec<_>, _>>()?;
            series.insert(key(k), at(HValue::Array(items)));
        }
        o.insert(key("measure:series"), at(HValue::Object(series)));
    }
    if !value.flags.is_empty() {
        let mut flags = value.flags.clone();
        flags.sort();
        flags.dedup();
        o.insert(
            key("measure:flags"),
            at(HValue::Array(
                flags.into_iter().map(|f| at(HValue::Str(f))).collect(),
            )),
        );
    }
    Ok(object_to_payload(&o).expect("measure:id is always written"))
}

/// The gist of a measure unit: the measure, its headline value rendered from the stored
/// integer, and the scope — in that order, so that a long scope is what the gist bound cuts
/// and the number is what survives.
pub fn gist(recipe: &Recipe, value: &Value) -> String {
    let head = value
        .headline
        .as_ref()
        .and_then(|(k, label)| value.values.get(k).map(|v| format!("{label} {v} {k}")));
    let scope = recipe
        .scope
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    match head {
        Some(h) => format!("{} {h}, over {scope}", recipe.id),
        None => format!("{} over {scope}", recipe.id),
    }
}
