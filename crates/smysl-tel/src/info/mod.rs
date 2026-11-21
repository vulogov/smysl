//! Information measures over telemetry (RFC SMYSL-5): entropies, mutual information and
//! divergences of a symbolised sample set, each stated as an `x.tel/info` unit.
//!
//! A series becomes symbols in exact decimal arithmetic (§5: [`grid`], then [`symbols`]);
//! floating point enters once, in the sum over integer counts, through [`num`]; and the result
//! is stored rounded to six places (§6.0), so that a quote of it can be verified by rule TQ.
//!
//! The statistics of one series are `ent/1` and `perm/1` ([`entropy`]); of two windows of one
//! series, `jsd/1` ([`jsd`]); of two series, `mi/1` with its lag scan and surrogates ([`mi`]);
//! and of a log stream's templates, `tent/1` ([`logs`]), which reads units and not samples.
//!
//! **A statistic that cannot be given is not a unit** (§4): a series whose class does not admit
//! it (`SMY-E701`), a recipe that is not admitted (`SMY-E702`), or a window with too few symbols
//! or too little coverage (`SMY-W703`) gives a [`Refusal`] and nothing else.

use std::cmp::Ordering;

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::diag::Code;
use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, Decimal, RelKind, Relation, SchemaId, SourceKind, SourceRef, Status, Uid,
    UnitCore, UnitCoreBuilder,
};
use smysl_series::{Backend, Sample, Substrate};

use crate::detect::Recipe;
use crate::exact::{numeric_cmp, BigDec};

pub mod entropy;
pub mod grid;
pub mod jsd;
pub mod logs;
pub mod mi;
pub mod num;
pub mod symbols;

pub use grid::Of;
pub use mi::Lag;
pub use symbols::Bins;

/// `ent/1`: the plug-in entropy of a series' binned values (§6.1).
pub const ENT: &str = "ent/1";
/// `perm/1`: the plug-in entropy of a series' ordinal patterns (§6.1).
pub const PERM: &str = "perm/1";
/// `mi/1`: the plug-in mutual information between two series at a lag (§6.3).
pub const MI: &str = "mi/1";
/// `jsd/1`: the Jensen–Shannon divergence between a window and a reference window (§6.2).
pub const JSD: &str = "jsd/1";
/// `tent/1`: the plug-in entropy of a log stream's templates (§6.4).
pub const TENT: &str = "tent/1";
/// What a refusal of an unknown statistic lists.
const BUILT: &str = "ent/1, perm/1, mi/1, jsd/1 or tent/1";

/// Why a statistic was not given (§9): a diagnostic code and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    pub message: String,
}

impl Refusal {
    fn new(code: Code, message: impl Into<String>) -> Refusal {
        Refusal {
            code,
            message: message.into(),
        }
    }

    fn recipe(message: impl Into<String>) -> Refusal {
        Refusal::new(Code::E702, message)
    }

    fn not_run(message: impl Into<String>) -> Refusal {
        Refusal::new(Code::W703, message)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

/// Which statistic, with the parameters only it has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat {
    /// `ent/1`, over `bins`; `reference` is `baseline`'s window, `[from, to)`.
    Ent {
        bins: Bins,
        reference: Option<(u64, u64)>,
    },
    /// `perm/1`: patterns of `m` values `tau` steps apart.
    Perm { m: usize, tau: usize },
    /// `mi/1`, over `bins` (`width`, `rank` or `diff`), at `lag`, with `null` surrogates drawn
    /// from `seed` and offset at least `guard` of the grid.
    Mi {
        bins: Bins,
        lag: Lag,
        null: u32,
        seed: u64,
        guard: (u64, u64),
    },
    /// `jsd/1`, over `bins` (`width`, `baseline` or `diff`), against `reference`, `[from, to)`.
    Jsd { bins: Bins, reference: (u64, u64) },
    /// `tent/1`, over the templates of the Drain recipe whose text hashes to `templates`
    /// (lowercase hex of `BLAKE3`).
    Tent { templates: String },
}

/// A recipe (§4): a statistic and every parameter it was computed under.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub stat: Stat,
    /// The grid step, ms. No default (D-I5).
    pub step: u64,
    /// How long a sample stays in force, ms.
    pub hold: u64,
    pub of: Of,
    /// The least share of grid points that must give a symbol, `(a, b)` in lowest terms.
    pub cover: (u64, u64),
    /// The least number of symbols, or of pairs.
    pub min_n: u64,
}

impl Spec {
    /// Parse a recipe file, HJSON:
    ///
    /// ```hjson
    /// { stat: ent/1, step: 60000, bins: "width:0.5" }
    /// { stat: ent/1, step: 60000, bins: "baseline:4", ref: "1768550400000-1768554000000" }
    /// { stat: perm/1, step: 60000, m: 4, tau: 2, of: increase }
    /// { stat: mi/1, step: 60000, bins: "rank:4", lag: "-10..10", null: 200, seed: 7 }
    /// { stat: jsd/1, step: 60000, bins: diff, ref: "1768550400000-1768554000000" }
    /// ```
    ///
    /// A parameter left out takes its default, and the recipe text writes it anyway. An unknown
    /// parameter, a value out of range or a missing `step`, `bins` or `ref` is `SMY-E702`.
    pub fn parse(text: &str) -> Result<Spec, Refusal> {
        let o = smysl_core::surface::parse_object(text, 0)
            .map_err(|e| Refusal::recipe(e.to_string()))?
            .value;
        let get = |k: &str| o.get(k).map(|v| &v.value);
        let id = match get("stat") {
            Some(HValue::Str(s)) => s.as_str(),
            _ => {
                return Err(Refusal::recipe(format!(
                    "a recipe names its `stat`: {BUILT}"
                )))
            }
        };
        let known: &[&str] = match id {
            ENT => &["step", "hold", "of", "bins", "cover", "min_n", "ref"],
            PERM => &["step", "hold", "of", "m", "tau", "cover", "min_n"],
            MI => &[
                "step", "hold", "of", "bins", "lag", "cover", "min_n", "null", "seed", "guard",
            ],
            JSD => &["step", "hold", "of", "bins", "ref", "cover", "min_n"],
            TENT => &["templates", "min_n"],
            "te/1" => {
                return Err(Refusal::recipe(
                    "`te/1` is reserved behind GE-I5 (RFC SMYSL-5 §10)",
                ))
            }
            other => {
                return Err(Refusal::recipe(format!(
                    "`{other}` is not a statistic: {BUILT}"
                )))
            }
        };
        for (k, _) in o.iter() {
            if k.value != "stat" && !known.contains(&k.value.as_str()) {
                return Err(Refusal::recipe(format!(
                    "`{}` is not a parameter of {id}",
                    k.value
                )));
            }
        }
        let int = |k: &str| -> Result<Option<u64>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Int(n)) if *n > 0 => Ok(Some(*n as u64)),
                Some(_) => Err(Refusal::recipe(format!("`{k}` is a positive integer"))),
            }
        };
        let count = |k: &str| -> Result<Option<u64>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Int(n)) if *n >= 0 => Ok(Some(*n as u64)),
                Some(_) => Err(Refusal::recipe(format!("`{k}` is a non-negative integer"))),
            }
        };
        let text = |k: &str| -> Result<Option<&str>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Str(s)) => Ok(Some(s.as_str())),
                Some(_) => Err(Refusal::recipe(format!("`{k}` is a string"))),
            }
        };
        let reference = || -> Result<Option<(u64, u64)>, Refusal> {
            text("ref")?
                .map(|r| {
                    window(r).ok_or_else(|| {
                        Refusal::recipe(format!("`ref: {r}` is FROM-TO, epoch ms, FROM < TO"))
                    })
                })
                .transpose()
        };
        let bins = |refuse_rank: Option<&str>| -> Result<Bins, Refusal> {
            let b = text("bins")?.ok_or_else(|| {
                Refusal::recipe(format!("{id} states its `bins`: {}", bins_for(id)))
            })?;
            let b = Bins::parse(b).map_err(Refusal::recipe)?;
            if let (Bins::Rank(k), Some(why)) = (&b, refuse_rank) {
                return Err(Refusal::recipe(format!(
                    "`rank:{k}` {why}; {id} takes {}",
                    bins_for(id)
                )));
            }
            Ok(b)
        };
        if id == TENT {
            let templates = match text("templates")? {
                None => logs::shipped(),
                Some(h)
                    if h.len() == 64
                        && h.bytes()
                            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) =>
                {
                    h.to_string()
                }
                Some(h) => {
                    return Err(Refusal::recipe(format!(
                        "`templates: {h}` is the lowercase hex of BLAKE3 over a Drain recipe's \
                         text, 64 digits"
                    )))
                }
            };
            // A grid's parameters mean nothing to templates: tent/1 states only its own.
            return Ok(Spec {
                stat: Stat::Tent { templates },
                step: 0,
                hold: 0,
                of: Of::Value,
                cover: (1, 1),
                min_n: int("min_n")?.unwrap_or(100),
            });
        }
        let step = int("step")?.ok_or_else(|| {
            Refusal::recipe("a recipe states its `step`, the grid step in ms: it has no default")
        })?;
        let hold = int("hold")?.unwrap_or(step);
        let of = match text("of")? {
            None | Some("value") => Of::Value,
            Some("increase") => Of::Increase,
            Some(s) => return Err(Refusal::recipe(format!("`of: {s}` is value or increase"))),
        };
        let cover = match text("cover")? {
            None => (4, 5),
            Some(s) => ratio(s)
                .ok_or_else(|| Refusal::recipe(format!("`cover: {s}` is A/B with 0 < A ≤ B")))?,
        };
        let stat = match id {
            ENT => {
                let bins = bins(Some("makes every entropy log2 k whatever the series does"))?;
                let reference = match (reference()?, &bins) {
                    (Some(r), Bins::Baseline(_)) => Some(r),
                    (None, Bins::Baseline(_)) => {
                        return Err(Refusal::recipe(
                            "baseline bins take their edges from `ref`, FROM-TO",
                        ))
                    }
                    (Some(_), _) => {
                        return Err(Refusal::recipe("`ref` is read only with baseline bins"))
                    }
                    (None, _) => None,
                };
                Stat::Ent { bins, reference }
            }
            PERM => {
                let m = int("m")?.unwrap_or(3);
                if !(2..=7).contains(&m) {
                    return Err(Refusal::recipe("`m`, the pattern length, is 2 to 7"));
                }
                Stat::Perm {
                    m: m as usize,
                    tau: int("tau")?.unwrap_or(1) as usize,
                }
            }
            MI => {
                let bins = bins(None)?;
                if let Bins::Baseline(_) = bins {
                    // R3-1: `x.tel/info` has one `ref_digest`, and baseline bins over two
                    // series read two reference windows, one of which no field could carry.
                    return Err(Refusal::recipe(
                        "mi/1 reads two series, and baseline bins would read a reference \
                         window of each, which one unit cannot carry the digests of; \
                         mi/1 takes rank, width or diff",
                    ));
                }
                let lag = match get("lag") {
                    None => Lag::One(0),
                    Some(HValue::Int(l)) => Lag::One(*l),
                    Some(HValue::Str(s)) => Lag::parse_scan(s).ok_or_else(|| {
                        Refusal::recipe(format!(
                            "`lag: {s}` is one lag in steps, or -L..L with 1 ≤ L ≤ {}",
                            mi::MAX_SCAN
                        ))
                    })?,
                    Some(_) => {
                        return Err(Refusal::recipe(
                            "`lag` is one lag in steps, or a scan \"-L..L\"",
                        ))
                    }
                };
                let null = count("null")?.unwrap_or(0);
                if null > u64::from(mi::MAX_NULL) {
                    return Err(Refusal::recipe(format!(
                        "`null`, the surrogates drawn, is 0 to {}",
                        mi::MAX_NULL
                    )));
                }
                let guard = match text("guard")? {
                    None => (1, 10),
                    Some(s) => ratio(s).filter(|(a, b)| 2 * a <= *b).ok_or_else(|| {
                        Refusal::recipe(format!("`guard: {s}` is A/B with 0 < A/B ≤ 1/2"))
                    })?,
                };
                Stat::Mi {
                    bins,
                    lag,
                    null: null as u32,
                    seed: count("seed")?.unwrap_or(0),
                    guard,
                }
            }
            _ => {
                let bins = bins(Some(
                    "ranks each window within itself, which makes the two distributions equal",
                ))?;
                let reference = reference()?.ok_or_else(|| {
                    Refusal::recipe("jsd/1 compares the window with `ref`, FROM-TO: it is required")
                })?;
                Stat::Jsd { bins, reference }
            }
        };
        let min_n = match int("min_n")? {
            Some(n) => n,
            None => match &stat {
                Stat::Ent { .. } | Stat::Jsd { .. } => 200,
                Stat::Perm { m, .. } => 10 * symbols::factorial(*m),
                Stat::Mi {
                    bins: Bins::Rank(k),
                    ..
                } => 5 * u64::from(*k) * u64::from(*k),
                Stat::Mi { .. } => 500,
                Stat::Tent { .. } => 100,
            },
        };
        Ok(Spec {
            stat,
            step,
            hold,
            of,
            cover,
            min_n,
        })
    }

    /// The recipe id.
    pub fn id(&self) -> &'static str {
        match self.stat {
            Stat::Ent { .. } => ENT,
            Stat::Perm { .. } => PERM,
            Stat::Mi { .. } => MI,
            Stat::Jsd { .. } => JSD,
            Stat::Tent { .. } => TENT,
        }
    }

    /// The `tel:stat` value its units carry.
    pub fn stat_name(&self) -> &'static str {
        match self.stat {
            Stat::Ent { .. } => "entropy",
            Stat::Perm { .. } => "perm-entropy",
            Stat::Mi { .. } => "mi",
            Stat::Jsd { .. } => "jsd",
            Stat::Tent { .. } => "template-entropy",
        }
    }

    /// Whether the statistic reads a log stream's templates (`tent/1`), through [`logs::run_on`].
    pub fn templates(&self) -> bool {
        matches!(self.stat, Stat::Tent { .. })
    }

    /// Whether the statistic reads a second series, `with` (`mi/1`).
    pub fn pairs(&self) -> bool {
        matches!(self.stat, Stat::Mi { .. })
    }

    /// The reference window the recipe reads, `[from, to)`: `jsd/1`'s, or `baseline`'s.
    pub fn reference(&self) -> Option<(u64, u64)> {
        match self.stat {
            Stat::Ent { reference, .. } => reference,
            Stat::Jsd { reference, .. } => Some(reference),
            _ => None,
        }
    }

    /// The recipe, every parameter written, defaults included (§4): the unit's `tel:recipe`.
    pub fn recipe(&self) -> Recipe {
        if let Stat::Tent { templates } = &self.stat {
            return Recipe::new(TENT)
                .with("min_n", self.min_n)
                .with("templates", templates);
        }
        let r = Recipe::new(self.id())
            .with("step", self.step)
            .with("hold", self.hold)
            .with("of", self.of.as_str())
            .with("cover", format!("{}/{}", self.cover.0, self.cover.1))
            .with("min_n", self.min_n);
        let window = |r: Recipe, (a, b): (u64, u64)| r.with("ref", format!("{a}-{b}"));
        match &self.stat {
            Stat::Ent { bins, reference } => {
                let r = r.with("bins", bins.text());
                match reference {
                    Some(w) => window(r, *w),
                    None => r,
                }
            }
            Stat::Perm { m, tau } => r.with("m", m).with("tau", tau),
            Stat::Mi {
                bins,
                lag,
                null,
                seed,
                guard,
            } => r
                .with("bins", bins.text())
                .with("lag", lag.text())
                .with("null", null)
                .with("seed", seed)
                .with("guard", format!("{}/{}", guard.0, guard.1)),
            Stat::Jsd { bins, reference } => window(r.with("bins", bins.text()), *reference),
            Stat::Tent { .. } => unreachable!("written above"),
        }
    }
}

/// The binnings a statistic takes, for its refusals.
fn bins_for(id: &str) -> &'static str {
    match id {
        MI => "rank:K, width:W or diff",
        _ => "width:W, baseline:K or diff",
    }
}

fn ratio(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('/')?;
    let (a, b) = (a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?);
    if a == 0 || a > b {
        return None;
    }
    let g = gcd(a, b);
    Some((a / g, b / g))
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

fn window(s: &str) -> Option<(u64, u64)> {
    let (a, b) = s.split_once('-')?;
    let (a, b) = (a.parse::<u64>().ok()?, b.parse::<u64>().ok()?);
    (a < b).then_some((a, b))
}

/// One (series, instrument) stream, as a statistic reads it.
#[derive(Debug, Clone, Copy)]
pub struct Stream<'a> {
    pub series: Uid,
    pub scheme: &'a str,
    pub key: &'a str,
    /// The leaf class the substrate recorded for the series (`SeriesRow::class`).
    pub class: &'a str,
    pub instrument: Uid,
}

/// Samples of one window, in sample order, and their §9.1 digest.
#[derive(Debug, Clone, Copy)]
pub struct Read<'a> {
    pub from_ms: u64,
    pub to_ms: u64,
    pub samples: &'a [Sample],
    pub digest: [u8; 32],
}

/// A statistic given: its unit, the unit's `x.tel/of-series` edges, and what the report prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Computed {
    pub unit: UnitCore,
    /// One `x.tel/of-series` edge to each series the unit grounds on, in uid order.
    pub relations: Vec<Relation>,
    /// Symbols, or pairs at `lag`, counted.
    pub n: u64,
    /// Distinct symbols of the series (at `lag`, for `mi/1`).
    pub k: u64,
    pub value: Decimal,
    /// The Miller–Madow value: `ent/1` and `perm/1`.
    pub mm: Option<Decimal>,
    /// `mi/1`'s bias term at `lag`.
    pub bias: Option<Decimal>,
    /// `mi/1`: distinct symbols of `with` at `lag`.
    pub k_with: Option<u64>,
    /// `mi/1`: the lag `value` belongs to.
    pub lag: Option<i64>,
    /// A scan's curve: each lag and its stored value, ascending.
    pub values: Vec<(i64, Decimal)>,
    /// Surrogates `(null_ge, null_b)`, when the recipe draws any.
    pub null: Option<(u64, u64)>,
    /// Points that gave no symbol (pairs that could have formed and did not, for `mi/1`).
    pub missing: u64,
}

/// Whether the series' class admits `of` (§5.2, R2-1): by the leaf class the substrate records.
/// `gauge`, or no class (a series first seen as a stale marker), as `value` or `increase`;
/// `counter` as `increase` only; nothing else.
pub fn admits(class: &str, of: Of) -> Result<(), Refusal> {
    match (class, of) {
        ("gauge" | "", _) | ("counter", Of::Increase) => Ok(()),
        ("counter", Of::Value) => Err(Refusal::new(
            Code::E701,
            "a counter's level is the time since it started; read it with `of: increase`",
        )),
        (other, _) => Err(Refusal::new(
            Code::E701,
            format!("a series of class `{other}` is not read by a statistic: gauge or counter"),
        )),
    }
}

/// Whether `n` of `possible` points meet `cover`.
fn covered(n: u64, possible: u64, (a, b): (u64, u64)) -> bool {
    possible > 0 && u128::from(n) * u128::from(b) >= u128::from(a) * u128::from(possible)
}

/// A window's grid and the sequence `of` reads on it, refusing a window with no grid point.
fn sequence(spec: &Spec, read: &Read<'_>) -> Result<(u64, Vec<Option<BigDec>>), Refusal> {
    let grid = grid::points(read.from_ms, read.to_ms, spec.step);
    if grid.is_empty() {
        return Err(Refusal::not_run(format!(
            "[{}, {}) holds no multiple of step {}",
            read.from_ms, read.to_ms, spec.step
        )));
    }
    let seq = grid::sequence(read.samples, &grid, spec.hold, spec.step, spec.of);
    Ok((grid.len() as u64, seq))
}

/// `baseline:k`'s edges from the reference window's sequence, refusing one with no value.
fn baseline_edges(
    spec: &Spec,
    bins: &Bins,
    reference: Option<&Read<'_>>,
) -> Result<Vec<BigDec>, Refusal> {
    let Bins::Baseline(k) = bins else {
        return Ok(Vec::new());
    };
    let r = reference
        .ok_or_else(|| Refusal::recipe("baseline bins need the reference window's samples"))?;
    let (_, rseq) = sequence(spec, r)?;
    let e = symbols::edges(&rseq, *k);
    if e.is_empty() {
        return Err(Refusal::not_run(format!(
            "the reference window [{}, {}) gives no value to take edges from",
            r.from_ms, r.to_ms
        )));
    }
    Ok(e)
}

/// Refuse `n` symbols of `g` points under `cover` or `min_n`; `what` names the window or lag.
fn enough(spec: &Spec, n: u64, g: u64, what: &str) -> Result<(), Refusal> {
    let (a, b) = spec.cover;
    if !covered(n, g, spec.cover) {
        return Err(Refusal::not_run(format!(
            "{what}{n} of {g} points give a symbol, under cover {a}/{b}"
        )));
    }
    if n < spec.min_n {
        return Err(Refusal::not_run(format!(
            "{what}{n} symbols, under min_n {}",
            spec.min_n
        )));
    }
    Ok(())
}

/// Store a statistic by §6.0, refusing one that is not finite.
fn store(v: f64, statistic: bool, what: &str) -> Result<Decimal, Refusal> {
    let d = if statistic {
        num::stored_statistic(v)
    } else {
        num::stored(v)
    };
    d.ok_or_else(|| Refusal::not_run(format!("{what} is not finite")))
}

/// Compute `spec` over one stream's window: with the reference window when the recipe reads one
/// (`jsd/1`, `baseline` bins), and with the second stream's window when it reads two (`mi/1`).
pub fn compute(
    spec: &Spec,
    stream: &Stream<'_>,
    read: &Read<'_>,
    reference: Option<&Read<'_>>,
    with: Option<(&Stream<'_>, &Read<'_>)>,
) -> Result<Computed, Refusal> {
    admits(stream.class, spec.of)?;
    match &spec.stat {
        Stat::Mi { .. } => {
            let (ws, wr) = with.ok_or_else(|| {
                Refusal::recipe("mi/1 reads two series: name the second with --with")
            })?;
            admits(ws.class, spec.of)?;
            compute_mi(spec, stream, read, ws, wr)
        }
        Stat::Jsd { bins, .. } => {
            let r = reference
                .ok_or_else(|| Refusal::recipe("jsd/1 needs the reference window's samples"))?;
            compute_jsd(spec, bins, stream, read, r)
        }
        Stat::Tent { .. } => Err(Refusal::recipe(
            "tent/1 reads a log stream's templates, not samples: info::logs::run_on runs it",
        )),
        _ => compute_one(spec, stream, read, reference),
    }
}

/// `ent/1` and `perm/1`.
fn compute_one(
    spec: &Spec,
    stream: &Stream<'_>,
    read: &Read<'_>,
    reference: Option<&Read<'_>>,
) -> Result<Computed, Refusal> {
    let (g, seq) = sequence(spec, read)?;
    let (syms, reference) = match &spec.stat {
        Stat::Ent { bins, .. } => {
            let edges = baseline_edges(spec, bins, reference)?;
            let reference = matches!(bins, Bins::Baseline(_))
                .then_some(reference)
                .flatten();
            (symbols::symbols(&seq, bins, &edges), reference)
        }
        Stat::Perm { m, tau } => (symbols::ordinal(&seq, *m, *tau), None),
        _ => unreachable!("compute dispatches mi/1 and jsd/1 elsewhere"),
    };
    let n = syms.iter().flatten().count() as u64;
    enough(spec, n, g, "")?;
    let p =
        entropy::plugin(&entropy::counts(&syms)).ok_or_else(|| Refusal::not_run("no symbol"))?;
    let value = store(p.value, true, "the statistic")?;
    let mm = store(p.mm, false, "mm")?;
    let missing = g - n;
    let mut f = Fields::header(spec, stream, read, p.n, &value);
    f.uint("tel:k", p.k);
    f.dec("tel:mm", &mm);
    f.missing(missing);
    if let Some(r) = reference {
        f.reference(r);
    }
    let (unit, relations) = f.unit(spec, stream, read, None);
    Ok(Computed {
        unit,
        relations,
        n: p.n,
        k: p.k,
        value,
        mm: Some(mm),
        bias: None,
        k_with: None,
        lag: None,
        values: Vec::new(),
        null: None,
        missing,
    })
}

/// `jsd/1` (§6.2): both windows under one binning, `baseline` edges from the reference.
fn compute_jsd(
    spec: &Spec,
    bins: &Bins,
    stream: &Stream<'_>,
    read: &Read<'_>,
    reference: &Read<'_>,
) -> Result<Computed, Refusal> {
    let edges = baseline_edges(spec, bins, Some(reference))?;
    let (g, seq) = sequence(spec, read)?;
    let (rg, rseq) = sequence(spec, reference)?;
    let p = entropy::counts(&symbols::symbols(&seq, bins, &edges));
    let q = entropy::counts(&symbols::symbols(&rseq, bins, &edges));
    let (np, nq): (u64, u64) = (p.values().sum(), q.values().sum());
    enough(spec, np, g, "the window: ")?;
    enough(spec, nq, rg, "the reference window: ")?;
    let d = jsd::divergence(&p, &q)
        .ok_or_else(|| Refusal::not_run("the windows hold too many symbols to weigh"))?;
    let value = store(d, true, "the statistic")?;
    let n = np.min(nq);
    let k = p.len() as u64;
    let missing = g - np;
    let mut f = Fields::header(spec, stream, read, n, &value);
    f.uint("tel:k", k);
    f.missing(missing);
    f.reference(reference);
    let (unit, relations) = f.unit(spec, stream, read, None);
    Ok(Computed {
        unit,
        relations,
        n,
        k,
        value,
        mm: None,
        bias: None,
        k_with: None,
        lag: None,
        values: Vec::new(),
        null: None,
        missing,
    })
}

/// One lag of `mi/1`, as computed and stored.
struct AtLag {
    lag: i64,
    plugin: mi::Plugin,
    value: Decimal,
    /// Pairs that could have formed at this lag: the grid less `|lag|`.
    possible: u64,
}

/// The keys a series pairs by: its symbols, or for `rank:k` its dense value places.
fn keys(bins: &Bins, seq: &[Option<BigDec>]) -> Vec<Option<i64>> {
    match bins {
        Bins::Rank(_) => symbols::dense(seq),
        b => symbols::symbols(seq, b, &[]),
    }
}

/// `mi/1` (§6.3): one lag or a scan, then the surrogates.
fn compute_mi(
    spec: &Spec,
    stream: &Stream<'_>,
    read: &Read<'_>,
    ws: &Stream<'_>,
    wr: &Read<'_>,
) -> Result<Computed, Refusal> {
    let Stat::Mi {
        bins,
        lag,
        null,
        seed,
        guard,
    } = &spec.stat
    else {
        unreachable!("compute dispatches mi/1 here")
    };
    let (g, a_seq) = sequence(spec, read)?;
    let (_, b_seq) = sequence(spec, wr)?;
    let (a, b) = (keys(bins, &a_seq), keys(bins, &b_seq));
    let rank = match bins {
        Bins::Rank(k) => Some(*k),
        _ => None,
    };
    let scan = matches!(lag, Lag::Scan(_));
    let mut curve = Vec::new();
    for l in lag.lags() {
        let counts = mi::pairs(&a, &b, l, rank);
        let n: u64 = counts.values().sum();
        let possible = g.saturating_sub(l.unsigned_abs());
        let what = if scan {
            format!("at lag {l}: ")
        } else {
            String::new()
        };
        let (ca, cb) = spec.cover;
        if !covered(n, possible, spec.cover) {
            return Err(Refusal::not_run(format!(
                "{what}{n} of {possible} pairs form, under cover {ca}/{cb}"
            )));
        }
        if n < spec.min_n {
            return Err(Refusal::not_run(format!(
                "{what}{n} pairs, under min_n {}",
                spec.min_n
            )));
        }
        let p = mi::plugin(&counts).ok_or_else(|| Refusal::not_run("no pair"))?;
        let value = store(p.value, true, "the statistic")?;
        curve.push(AtLag {
            lag: l,
            plugin: p,
            value,
            possible,
        });
    }
    // The maximum of the stored values; at a tie, the smallest |lag|, the negative one first.
    let best = curve
        .iter()
        .min_by(|x, y| {
            numeric_cmp(&y.value, &x.value)
                .then(x.lag.unsigned_abs().cmp(&y.lag.unsigned_abs()))
                .then(x.lag.cmp(&y.lag))
        })
        .expect("a lag is read");
    let recipe = spec.recipe().text();
    let null = if *null > 0 {
        let range = mi::offset_range(g, *guard).ok_or_else(|| {
            Refusal::not_run(format!(
                "a grid of {g} points leaves no offset guard {}/{} admits",
                guard.0, guard.1
            ))
        })?;
        let mut ge = 0u64;
        for o in mi::offsets(&recipe, *seed, range, *null) {
            let rb = mi::rotate(&b, o);
            let mut top: Option<Decimal> = None;
            let mut short = false;
            for l in lag.lags() {
                let counts = mi::pairs(&a, &rb, l, rank);
                let n: u64 = counts.values().sum();
                let Some(p) = mi::plugin(&counts).filter(|_| n >= spec.min_n) else {
                    short = true;
                    break;
                };
                let v = store(p.value, true, "a surrogate")?;
                if top
                    .as_ref()
                    .is_none_or(|t| numeric_cmp(&v, t) == Ordering::Greater)
                {
                    top = Some(v);
                }
            }
            // A rotation whose pairs fall below min_n counts toward null_b and not null_ge.
            if !short
                && top
                    .as_ref()
                    .is_some_and(|t| numeric_cmp(t, &best.value) != Ordering::Less)
            {
                ge += 1;
            }
        }
        Some((ge, u64::from(*null)))
    } else {
        None
    };
    let p = &best.plugin;
    let bias = store(p.bias, false, "bias")?;
    let missing = best.possible - p.n;
    let mut f = Fields::header(spec, stream, read, p.n, &best.value);
    f.uint("tel:k", p.k_a);
    f.uint("tel:k_with", p.k_b);
    f.text("tel:with", &ws.series.canonical());
    f.text("tel:with_instrument", &ws.instrument.canonical());
    f.bytes("tel:with_digest", &wr.digest);
    f.dec("tel:bias", &bias);
    f.int("tel:lag", best.lag);
    if scan {
        f.ints("tel:lags", curve.iter().map(|c| c.lag));
        f.decs("tel:values", curve.iter().map(|c| &c.value));
    }
    if let Some((ge, b)) = null {
        f.uint("tel:null_b", b);
        f.uint("tel:null_ge", ge);
    }
    f.missing(missing);
    let (unit, relations) = f.unit(spec, stream, read, Some(ws));
    Ok(Computed {
        unit,
        relations,
        n: p.n,
        k: p.k_a,
        value: best.value,
        mm: None,
        bias: Some(bias),
        k_with: Some(p.k_b),
        lag: Some(best.lag),
        values: if scan {
            curve.iter().map(|c| (c.lag, c.value)).collect()
        } else {
            Vec::new()
        },
        null,
        missing,
    })
}

/// A payload under construction: CBOR values by key, written in the kernel's key order.
#[derive(Default)]
struct Fields(Vec<(Vec<u8>, Vec<u8>)>);

impl Fields {
    /// The fields every info unit carries (§3).
    fn header(spec: &Spec, stream: &Stream<'_>, read: &Read<'_>, n: u64, value: &Decimal) -> Self {
        let mut f = Fields::default();
        f.text("tel:stat", spec.stat_name());
        f.text("tel:series", &stream.series.canonical());
        f.text("tel:instrument", &stream.instrument.canonical());
        f.uint("tel:from_ms", read.from_ms);
        f.uint("tel:to_ms", read.to_ms);
        f.text("tel:recipe", &spec.recipe().text());
        f.uint("tel:n", n);
        f.dec("tel:value", value);
        f.bytes("tel:digest", &read.digest);
        f
    }
    fn put(&mut self, k: &str, v: Vec<u8>) {
        let mut e = Enc::new();
        e.text(k);
        self.0.push((e.into_bytes(), v));
    }
    fn text(&mut self, k: &str, s: &str) {
        let mut e = Enc::new();
        e.text(s);
        self.put(k, e.into_bytes());
    }
    fn uint(&mut self, k: &str, n: u64) {
        let mut e = Enc::new();
        e.uint(n);
        self.put(k, e.into_bytes());
    }
    fn int(&mut self, k: &str, n: i64) {
        let mut e = Enc::new();
        e.int(n);
        self.put(k, e.into_bytes());
    }
    fn ints(&mut self, k: &str, ns: impl ExactSizeIterator<Item = i64>) {
        let mut e = Enc::new();
        e.array_head(ns.len());
        for n in ns {
            e.int(n);
        }
        self.put(k, e.into_bytes());
    }
    fn dec(&mut self, k: &str, d: &Decimal) {
        self.put(k, d.to_cbor());
    }
    fn decs<'d>(&mut self, k: &str, ds: impl ExactSizeIterator<Item = &'d Decimal>) {
        let mut e = Enc::new();
        e.array_head(ds.len());
        for d in ds {
            e.raw(&d.to_cbor());
        }
        self.put(k, e.into_bytes());
    }
    fn bytes(&mut self, k: &str, b: &[u8; 32]) {
        let mut e = Enc::new();
        e.bytes(b);
        self.put(k, e.into_bytes());
    }
    fn missing(&mut self, missing: u64) {
        if missing > 0 {
            self.uint("tel:missing", missing);
        }
    }
    fn reference(&mut self, r: &Read<'_>) {
        self.uint("tel:ref_from_ms", r.from_ms);
        self.uint("tel:ref_to_ms", r.to_ms);
        self.bytes("tel:ref_digest", &r.digest);
    }
    fn payload(mut self) -> Vec<u8> {
        self.0.sort_by(|a, b| a.0.cmp(&b.0));
        let mut e = Enc::new();
        e.head(major::MAP, self.0.len() as u64);
        for (k, v) in self.0 {
            e.raw(&k);
            e.raw(&v);
        }
        e.into_bytes()
    }

    /// The `x.tel/info` unit (§3) and its edges: `derived`, grounded on its series (both, in
    /// uid order, for `mi/1`), its source the series observed at the window's end.
    fn unit(
        self,
        spec: &Spec,
        stream: &Stream<'_>,
        read: &Read<'_>,
        with: Option<&Stream<'_>>,
    ) -> (UnitCore, Vec<Relation>) {
        let mut grounds = vec![stream.series];
        let mut gist = stream.key.to_string();
        if let Some(w) = with {
            grounds.push(w.series);
            gist = format!("{gist} ~ {}", w.key);
        }
        grounds.sort();
        grounds.dedup();
        let unit = UnitCoreBuilder::new(
            SchemaId::Extension(tel::INFO.to_string()),
            tel::gist_of(&format!(
                "{} {gist} [{}, {}) {}",
                spec.stat_name(),
                read.from_ms,
                read.to_ms,
                spec.id()
            )),
            Status::Derived,
        )
        .source(
            SourceRef::new(
                SourceKind::Metric,
                format!("{}{}:{}", tel::SERIES_REF, stream.scheme, stream.key),
            )
            .observed_at(read.to_ms),
        )
        .grounds(grounds.clone())
        .payload(self.payload())
        .build()
        .expect("an info unit is well formed by construction");
        let uid = canonical_uid(&unit);
        let relations = grounds
            .into_iter()
            .map(|s| Relation::new(RelKind::Extension(tel::OF_SERIES.to_string()), uid, s))
            .collect();
        (unit, relations)
    }
}

/// The second series of a pair statistic, and the instrument to read it from when it has more
/// than one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct With {
    pub series: Uid,
    pub instrument: Option<Uid>,
}

/// One stream's outcome: a unit, or why there is none.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub series: Uid,
    pub key: String,
    pub instrument: Uid,
    /// `mi/1`: the second series' key and the instrument it was read from.
    pub with: Option<(String, Uid)>,
    pub result: Result<Computed, Refusal>,
}

/// Run `spec` over each of `series` in `[from_ms, to_ms)`, once per instrument it has samples
/// from, or for `instrument` alone when it is given; for `mi/1`, against `with`, once per
/// instrument of it in turn. An error is the substrate's, a series it does not hold, or `with`
/// given to a statistic of one series or missing from one of two; a refusal is an outcome.
pub fn run_on<B: Backend>(
    spec: &Spec,
    substrate: &Substrate<B>,
    series: &[Uid],
    instrument: Option<Uid>,
    with: Option<With>,
    from_ms: u64,
    to_ms: u64,
) -> Result<Vec<Outcome>, String> {
    if spec.templates() {
        return Err("tent/1 reads templates: info::logs::run_on runs it".to_string());
    }
    let with = match (spec.pairs(), with) {
        (true, Some(w)) => Some(w),
        (true, None) => {
            return Err(format!(
                "{} reads two series: name the second with --with",
                spec.id()
            ))
        }
        (false, Some(_)) => {
            return Err(format!("{} reads one series; --with is mi/1's", spec.id()))
        }
        (false, None) => None,
    };
    let row = |s: &Uid| {
        substrate
            .row(s)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("the substrate holds no sample of series {}", s.canonical()))
    };
    let instruments = |s: &Uid, given: Option<Uid>| match given {
        Some(i) => Ok(vec![i]),
        None => substrate.instruments(s).map_err(|e| e.to_string()),
    };
    let window =
        |s: &Uid, i: &Uid, a: u64, b: u64| substrate.window(s, i, a, b).map_err(|e| e.to_string());
    let wrow = with.as_ref().map(|w| row(&w.series)).transpose()?;
    let wins = match &with {
        Some(w) => instruments(&w.series, w.instrument)?,
        None => vec![],
    };
    let wwins = match &with {
        Some(w) => wins
            .iter()
            .map(|i| window(&w.series, i, from_ms, to_ms))
            .collect::<Result<Vec<_>, _>>()?,
        None => vec![],
    };
    let reference = spec.reference();
    let mut out = Vec::new();
    for s in series {
        let row = row(s)?;
        for inst in instruments(s, instrument)? {
            let w = window(s, &inst, from_ms, to_ms)?;
            let r = reference.map(|(a, b)| window(s, &inst, a, b)).transpose()?;
            let stream = Stream {
                series: *s,
                scheme: &row.scheme,
                key: &row.key,
                class: &row.class,
                instrument: inst,
            };
            let read = Read {
                from_ms,
                to_ms,
                samples: &w.samples,
                digest: w.digest,
            };
            let rread = r.as_ref().zip(reference).map(|(r, (a, b))| Read {
                from_ms: a,
                to_ms: b,
                samples: &r.samples,
                digest: r.digest,
            });
            match (&with, &wrow) {
                (Some(wi), Some(wrow)) => {
                    for (winst, ww) in wins.iter().zip(&wwins) {
                        let ws = Stream {
                            series: wi.series,
                            scheme: &wrow.scheme,
                            key: &wrow.key,
                            class: &wrow.class,
                            instrument: *winst,
                        };
                        let wread = Read {
                            from_ms,
                            to_ms,
                            samples: &ww.samples,
                            digest: ww.digest,
                        };
                        out.push(Outcome {
                            series: *s,
                            key: row.key.clone(),
                            instrument: inst,
                            with: Some((wrow.key.clone(), *winst)),
                            result: compute(spec, &stream, &read, None, Some((&ws, &wread))),
                        });
                    }
                }
                _ => out.push(Outcome {
                    series: *s,
                    key: row.key.clone(),
                    instrument: inst,
                    with: None,
                    result: compute(spec, &stream, &read, rread.as_ref(), None),
                }),
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recipe_writes_every_parameter_and_its_defaults() {
        let s = Spec::parse(r#"{ stat: ent/1, step: 60000, bins: "width:0.50" }"#).unwrap();
        assert_eq!(
            s.recipe().text(),
            "ent/1?bins=width:0.5&cover=4/5&hold=60000&min_n=200&of=value&step=60000"
        );
        let s = Spec::parse(r#"{ stat: perm/1, step: 1000, m: 4, cover: "8/10" }"#).unwrap();
        assert_eq!(
            s.recipe().text(),
            "perm/1?cover=4/5&hold=1000&m=4&min_n=240&of=value&step=1000&tau=1"
        );
        let s = Spec::parse(
            r#"{ stat: ent/1, step: 60000, bins: "baseline:4", ref: "0-3600000", of: increase }"#,
        )
        .unwrap();
        assert_eq!(
            s.recipe().text(),
            "ent/1?bins=baseline:4&cover=4/5&hold=60000&min_n=200&of=increase&ref=0-3600000\
             &step=60000"
        );
    }

    #[test]
    fn pair_and_window_recipes_write_every_parameter() {
        let s = Spec::parse(r#"{ stat: mi/1, step: 60000, bins: "rank:4" }"#).unwrap();
        assert_eq!(
            s.recipe().text(),
            "mi/1?bins=rank:4&cover=4/5&guard=1/10&hold=60000&lag=0&min_n=80&null=0&of=value\
             &seed=0&step=60000"
        );
        let s = Spec::parse(
            r#"{ stat: mi/1, step: 60000, bins: "width:2", lag: "-10..10", null: 200, seed: 7,
                 guard: "2/10" }"#,
        )
        .unwrap();
        assert_eq!(
            s.recipe().text(),
            "mi/1?bins=width:2&cover=4/5&guard=1/5&hold=60000&lag=-10..10&min_n=500&null=200\
             &of=value&seed=7&step=60000"
        );
        let s = Spec::parse(r#"{ stat: mi/1, step: 60000, bins: diff, lag: -3 }"#).unwrap();
        assert!(s.recipe().text().contains("&lag=-3&"));
        assert!(s.pairs());
        let s =
            Spec::parse(r#"{ stat: jsd/1, step: 60000, bins: diff, ref: "0-3600000" }"#).unwrap();
        assert_eq!(
            s.recipe().text(),
            "jsd/1?bins=diff&cover=4/5&hold=60000&min_n=200&of=value&ref=0-3600000&step=60000"
        );
        assert_eq!(s.reference(), Some((0, 3_600_000)));
    }

    #[test]
    fn a_recipe_that_is_not_admitted_is_e702() {
        for bad in [
            r#"{ stat: ent/1, bins: diff }"#,
            r#"{ stat: ent/1, step: 1000 }"#,
            r#"{ stat: ent/1, step: 1000, bins: "rank:4" }"#,
            r#"{ stat: ent/1, step: 1000, bins: "baseline:4" }"#,
            r#"{ stat: ent/1, step: 1000, bins: diff, ref: "0-10" }"#,
            r#"{ stat: ent/1, step: 1000, bins: diff, lag: 1 }"#,
            r#"{ stat: ent/1, step: 0, bins: diff }"#,
            r#"{ stat: ent/1, step: 1000, bins: diff, cover: "6/5" }"#,
            r#"{ stat: ent/1, step: 1000, bins: diff, of: level }"#,
            r#"{ stat: perm/1, step: 1000, m: 8 }"#,
            r#"{ stat: mi/1, step: 1000 }"#,
            r#"{ stat: mi/1, step: 1000, bins: "baseline:4", ref: "0-10" }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", ref: "0-10" }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", lag: "-65..65" }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", lag: "-3..4" }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", null: 1001 }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", null: -1 }"#,
            r#"{ stat: mi/1, step: 1000, bins: "rank:4", guard: "3/5" }"#,
            r#"{ stat: jsd/1, step: 1000, bins: diff }"#,
            r#"{ stat: jsd/1, step: 1000, bins: "rank:4", ref: "0-10" }"#,
            r#"{ stat: jsd/1, step: 1000, bins: diff, ref: "0-10", m: 3 }"#,
            r#"{ stat: tent/1, step: 1000 }"#,
            r#"{ stat: tent/1, templates: "ABC" }"#,
            r#"{ stat: tent/1, min_n: 0 }"#,
            r#"{ stat: te/1, step: 1000 }"#,
            r#"{ stat: ent/2, step: 1000 }"#,
            r#"{ step: 1000 }"#,
        ] {
            let r = Spec::parse(bad).expect_err(bad);
            assert_eq!(r.code, Code::E702, "{bad}: {r}");
        }
    }

    #[test]
    fn admission_is_by_the_substrates_class() {
        assert!(admits("gauge", Of::Value).is_ok());
        assert!(admits("", Of::Increase).is_ok());
        assert!(admits("counter", Of::Increase).is_ok());
        for (class, of) in [
            ("counter", Of::Value),
            ("histogram", Of::Value),
            ("histogram", Of::Increase),
            ("config", Of::Value),
            ("log", Of::Value),
        ] {
            assert_eq!(admits(class, of).unwrap_err().code, Code::E701, "{class}");
        }
    }
}
