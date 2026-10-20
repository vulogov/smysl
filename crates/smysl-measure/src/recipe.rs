//! A measure's recipe (SMYSL-2.6 §3.4): everything its value depends on, as a canonical map.
//!
//! The map is the payload's `measure:recipe`, and its hash is what an attestation carries in
//! its `recipe` slot, so "which runs used the same recipe" is a comparison of 32 bytes. Two
//! rules make that comparison mean something:
//!
//! - **Every field with a value is written**, defaults included. A recipe that left `seed: 0`
//!   out because it is the default would hash the same as one written before the default
//!   changed, and the default is exactly what a later version might change.
//! - **Absent is absent.** An optional field with no value — no class policy for a measure
//!   that needs none — is not written as a null, so a recipe gaining an optional field later
//!   does not change the hash of every recipe that does not use it.
//!
//! The encoding is `smysl_core::surface::object_to_payload`: deterministic CBOR, keys sorted by
//! their encoded bytes, text NFC-normalised. It holds no floats — a ratio is two integers — so
//! nothing in a recipe is quantised.

use std::collections::BTreeMap;
use std::fmt;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::object_to_payload;
use smysl_core::{hash_bytes, Span, Uid};

/// The default unit filter: what a text asserts in its own voice (§3.4).
pub const DEFAULT_FILTER: &str = "holder = text and mode = asserted";

/// The default pack estimator (§3.4), for the measures that use one.
pub const DEFAULT_ESTIMATOR: &str = "smysl/utf8-div4";

/// The segment a measure counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Level {
    #[default]
    Part,
    Chapter,
    Verse,
    Message,
    /// A fixed window of this many symbols.
    Window(u32),
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Level::Part => f.write_str("part"),
            Level::Chapter => f.write_str("chapter"),
            Level::Verse => f.write_str("verse"),
            Level::Message => f.write_str("message"),
            Level::Window(n) => write!(f, "window:{n}"),
        }
    }
}

/// The time axis a time-ordered measure orders by (SMYSL-2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Axis {
    #[default]
    Said,
    Composed,
    About,
    Known,
}

impl fmt::Display for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Axis::Said => "said",
            Axis::Composed => "composed",
            Axis::About => "about",
            Axis::Known => "known",
        })
    }
}

/// How classes are formed, for a measure over classes (SMYSL-2.4 §3.6).
///
/// The recipe's own copy rather than `smysl_graph::proposition::Policy`, because a recipe is a
/// format and its spellings are fixed here; the measures that read classes map it onto the
/// graph's type when they ask for them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ClassPolicy {
    Strict,
    Component,
    /// `strict` over edges with at least this many distinct attesting agents.
    Attested(u32),
}

impl ClassPolicy {
    /// `strict`, `component` or `attested:<n>` with `n` at least 1, as [`fmt::Display`] writes
    /// it; `None` for anything else.
    pub fn parse(s: &str) -> Option<ClassPolicy> {
        match s {
            "strict" => Some(ClassPolicy::Strict),
            "component" => Some(ClassPolicy::Component),
            other => other
                .strip_prefix("attested:")
                .and_then(|n| n.parse::<u32>().ok())
                .filter(|n| *n > 0)
                .map(ClassPolicy::Attested),
        }
    }
}

impl fmt::Display for ClassPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClassPolicy::Strict => f.write_str("strict"),
            ClassPolicy::Component => f.write_str("component"),
            ClassPolicy::Attested(n) => write!(f, "attested:{n}"),
        }
    }
}

/// A measure-specific parameter: an n-gram order, a budget, a smoothing weight as a ratio.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Param {
    Int(i64),
    Text(String),
    Bool(bool),
    Ratio(u32, u32),
}

/// A recipe could not be written: a number outside what a payload integer holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeError(pub String);

impl fmt::Display for RecipeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RecipeError {}

impl RecipeError {
    /// As the diagnostic a caller reports: a recipe that cannot be written cannot be compared.
    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(Code::E489).with_message(self.0.clone())
    }
}

/// What a measure's value depends on (SMYSL-2.6 §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Recipe {
    /// The measure id, `T2`, and its version.
    pub id: String,
    pub v: u16,
    /// The sq scope as written, `using` pins included (SMYSL-2.5).
    pub scope: String,
    /// [`crate::scope::digest`] of what the scope resolved to.
    pub scope_digest: [u8; 32],
    /// A channel measure's target scope (§3.9), as written: the scope is the source. `None`
    /// for every other measure, and then not written, so no other recipe's hash moved when
    /// channels arrived (TX-P9 N4).
    pub channel: Option<String>,
    /// The target's digest, as `scope_digest` is the source's.
    pub channel_digest: Option<[u8; 32]>,
    /// The lens chain the scope resolved under. `None` until TX-P12 applies one.
    pub lenses: Option<[u8; 32]>,
    /// Analyzer chain and stream, `en/1:surface`.
    pub analyzer: Option<String>,
    pub estimator: Option<String>,
    /// Granularity profile, for A1.
    pub profile: Option<String>,
    pub policy: Option<ClassPolicy>,
    pub level: Level,
    pub axis: Axis,
    /// An sq `where` predicate over the scope's units; empty for none, and then not written.
    pub filter: String,
    /// The largest undated share a time-ordered measure accepts, as a ratio.
    pub undated_max: (u32, u32),
    /// Admit lossy texts (`SMY-W408` when it does, `SMY-E480` when it does not).
    pub lossy: bool,
    /// The smallest sample the estimator accepts (`SMY-W485`).
    pub min_n: Option<u64>,
    pub seed: u64,
    /// Bootstrap replicates.
    pub boot_b: u32,
    /// Encoder and version, for the measures that compress: `brotli-rs/8.0.4 q11 w24`.
    pub codec: Option<String>,
    pub params: BTreeMap<String, Param>,
}

impl Recipe {
    /// A recipe for `id` at version `v` over `scope`, every other field at its §3.4 default.
    pub fn new(id: impl Into<String>, v: u16, scope: impl Into<String>) -> Recipe {
        Recipe {
            id: id.into(),
            v,
            scope: scope.into(),
            scope_digest: [0; 32],
            channel: None,
            channel_digest: None,
            lenses: None,
            analyzer: None,
            estimator: None,
            profile: None,
            policy: None,
            level: Level::default(),
            axis: Axis::default(),
            filter: DEFAULT_FILTER.to_string(),
            undated_max: (1, 10),
            lossy: false,
            min_n: None,
            seed: 0,
            boot_b: 1000,
            codec: None,
            params: BTreeMap::new(),
        }
    }

    /// The same recipe for another measure: what [`crate::run`] does to a base recipe for each
    /// id it is asked for.
    pub fn for_measure(&self, id: impl Into<String>, v: u16) -> Recipe {
        Recipe {
            id: id.into(),
            v,
            ..self.clone()
        }
    }

    /// The recipe as the `measure:recipe` map.
    pub fn to_object(&self) -> Result<HObject, RecipeError> {
        let mut o = HObject::new();
        let put = |o: &mut HObject, k: &str, v: HValue| o.insert(key(k), at(v));
        put(&mut o, "id", HValue::Str(self.id.clone()));
        put(&mut o, "v", HValue::Int(i64::from(self.v)));
        put(&mut o, "scope", HValue::Str(self.scope.clone()));
        put(&mut o, "scope:digest", HValue::Str(b3(&self.scope_digest)));
        if let Some(c) = &self.channel {
            put(&mut o, "channel", HValue::Str(c.clone()));
        }
        if let Some(d) = &self.channel_digest {
            put(&mut o, "channel:digest", HValue::Str(b3(d)));
        }
        if let Some(l) = &self.lenses {
            put(&mut o, "lenses", HValue::Str(b3(l)));
        }
        if let Some(a) = &self.analyzer {
            put(&mut o, "analyzer", HValue::Str(a.clone()));
        }
        if let Some(e) = &self.estimator {
            put(&mut o, "estimator", HValue::Str(e.clone()));
        }
        if let Some(p) = &self.profile {
            put(&mut o, "profile", HValue::Str(p.clone()));
        }
        if let Some(p) = &self.policy {
            put(&mut o, "policy", HValue::Str(p.to_string()));
        }
        put(&mut o, "level", HValue::Str(self.level.to_string()));
        put(&mut o, "axis", HValue::Str(self.axis.to_string()));
        // An empty filter is no filter, and absent is absent: a unit measure's default (TX-P9
        // N1), which leaves every text measure's recipe, and so its hash, as it was.
        if !self.filter.is_empty() {
            put(&mut o, "filter", HValue::Str(self.filter.clone()));
        }
        put(
            &mut o,
            "undated:max",
            ratio(u64::from(self.undated_max.0), u64::from(self.undated_max.1))?,
        );
        put(&mut o, "lossy", HValue::Bool(self.lossy));
        if let Some(n) = self.min_n {
            put(&mut o, "min:n", HValue::Int(int(n, "min:n")?));
        }
        put(&mut o, "seed", HValue::Int(int(self.seed, "seed")?));
        put(&mut o, "boot:b", HValue::Int(i64::from(self.boot_b)));
        if let Some(c) = &self.codec {
            put(&mut o, "codec", HValue::Str(c.clone()));
        }
        if !self.params.is_empty() {
            let mut p = HObject::new();
            for (k, v) in &self.params {
                let v = match v {
                    Param::Int(i) => HValue::Int(*i),
                    Param::Text(t) => HValue::Str(t.clone()),
                    Param::Bool(b) => HValue::Bool(*b),
                    Param::Ratio(n, d) => ratio(u64::from(*n), u64::from(*d))?,
                };
                p.insert(key(k), at(v));
            }
            put(&mut o, "params", HValue::Object(p));
        }
        Ok(o)
    }

    /// The canonical CBOR of [`Recipe::to_object`].
    pub fn to_cbor(&self) -> Result<Vec<u8>, RecipeError> {
        let o = self.to_object()?;
        Ok(object_to_payload(&o).expect("a recipe always has an id, so it is never empty"))
    }

    /// `hash_bytes` of [`Recipe::to_cbor`]: the attestation's `recipe` slot.
    pub fn hash(&self) -> Result<[u8; 32], RecipeError> {
        Ok(hash_bytes(&self.to_cbor()?))
    }
}

pub(crate) fn key(k: &str) -> Spanned<String> {
    Spanned::new(k.to_string(), Span::new(0, 0))
}

pub(crate) fn at(v: HValue) -> Spanned<HValue> {
    Spanned::new(v, Span::new(0, 0))
}

/// A 32-byte hash in the `b3:` text form a uid uses.
pub(crate) fn b3(h: &[u8; 32]) -> String {
    Uid::from_bytes(*h).canonical()
}

/// A count as a payload integer, or the reason it is not one.
pub(crate) fn int(n: u64, what: &str) -> Result<i64, RecipeError> {
    i64::try_from(n).map_err(|_| RecipeError(format!("`{what}` is {n}, past a payload integer")))
}

/// `[num, den]`, never reduced: `2/4` and `1/2` are different counts of different things.
pub(crate) fn ratio(num: u64, den: u64) -> Result<HValue, RecipeError> {
    Ok(HValue::Array(vec![
        at(HValue::Int(int(num, "a ratio's numerator")?)),
        at(HValue::Int(int(den, "a ratio's denominator")?)),
    ]))
}
