//! Information measures over telemetry (RFC SMYSL-5): entropies, mutual information and
//! divergences of a symbolised sample set, each stated as an `x.tel/info` unit.
//!
//! A series becomes symbols in exact decimal arithmetic (§5: [`grid`], then [`symbols`]);
//! floating point enters once, in the sum over integer counts, through [`num`]; and the result
//! is stored rounded to six places (§6.0), so that a quote of it can be verified by rule TQ.
//!
//! TI-P1 builds the grid, `of`, the binnings and ordinal patterns, and the two statistics of
//! one series: `ent/1` and `perm/1` ([`entropy`]). `mi/1` and `jsd/1` are TI-P2's, `tent/1`
//! TI-P3's; a recipe naming one is refused until then.
//!
//! **A statistic that cannot be given is not a unit** (§4): a series whose class does not admit
//! it (`SMY-E701`), a recipe that is not admitted (`SMY-E702`), or a window with too few symbols
//! or too little coverage (`SMY-W703`) gives a [`Refusal`] and nothing else.

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

pub mod entropy;
pub mod grid;
pub mod num;
pub mod symbols;

pub use grid::Of;
pub use symbols::Bins;

/// `ent/1`: the plug-in entropy of a series' binned values (§6.1).
pub const ENT: &str = "ent/1";
/// `perm/1`: the plug-in entropy of a series' ordinal patterns (§6.1).
pub const PERM: &str = "perm/1";
/// The recipes later phases build, refused by name until they do.
const LATER: &[(&str, &str)] = &[("mi/1", "TI-P2"), ("jsd/1", "TI-P2"), ("tent/1", "TI-P3")];

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
    /// The least number of symbols.
    pub min_n: u64,
}

impl Spec {
    /// Parse a recipe file, HJSON:
    ///
    /// ```hjson
    /// { stat: ent/1, step: 60000, bins: "width:0.5" }
    /// { stat: ent/1, step: 60000, bins: "baseline:4", ref: "1768550400000-1768554000000" }
    /// { stat: perm/1, step: 60000, m: 4, tau: 2, of: increase }
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
                return Err(Refusal::recipe(
                    "a recipe names its `stat`: ent/1 or perm/1",
                ))
            }
        };
        if let Some((_, phase)) = LATER.iter().find(|(r, _)| *r == id) {
            return Err(Refusal::recipe(format!(
                "`{id}` is RFC SMYSL-5's and arrives in {phase}; this build has ent/1 and perm/1"
            )));
        }
        let known: &[&str] = match id {
            ENT => &["step", "hold", "of", "bins", "cover", "min_n", "ref"],
            PERM => &["step", "hold", "of", "m", "tau", "cover", "min_n"],
            "te/1" => {
                return Err(Refusal::recipe(
                    "`te/1` is reserved behind GE-I5 (RFC SMYSL-5 §10)",
                ))
            }
            other => {
                return Err(Refusal::recipe(format!(
                    "`{other}` is not a statistic: ent/1 or perm/1"
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
        let text = |k: &str| -> Result<Option<&str>, Refusal> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Str(s)) => Ok(Some(s.as_str())),
                Some(_) => Err(Refusal::recipe(format!("`{k}` is a string"))),
            }
        };
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
                let bins = text("bins")?.ok_or_else(|| {
                    Refusal::recipe("ent/1 states its `bins`: width:W, baseline:K or diff")
                })?;
                let bins = Bins::parse(bins).map_err(Refusal::recipe)?;
                if let Bins::Rank(k) = bins {
                    return Err(Refusal::recipe(format!(
                        "`rank:{k}` makes every entropy log2 {k} whatever the series does; \
                         ent/1 takes width, baseline or diff"
                    )));
                }
                let reference = match (text("ref")?, &bins) {
                    (Some(r), Bins::Baseline(_)) => Some(window(r).ok_or_else(|| {
                        Refusal::recipe(format!("`ref: {r}` is FROM-TO, epoch ms, FROM < TO"))
                    })?),
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
            _ => {
                let m = int("m")?.unwrap_or(3);
                if !(2..=7).contains(&m) {
                    return Err(Refusal::recipe("`m`, the pattern length, is 2 to 7"));
                }
                Stat::Perm {
                    m: m as usize,
                    tau: int("tau")?.unwrap_or(1) as usize,
                }
            }
        };
        let min_n = match int("min_n")? {
            Some(n) => n,
            None => match &stat {
                Stat::Ent { .. } => 200,
                Stat::Perm { m, .. } => 10 * symbols::factorial(*m),
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
        }
    }

    /// The `tel:stat` value its units carry.
    pub fn stat_name(&self) -> &'static str {
        match self.stat {
            Stat::Ent { .. } => "entropy",
            Stat::Perm { .. } => "perm-entropy",
        }
    }

    /// The recipe, every parameter written, defaults included (§4): the unit's `tel:recipe`.
    pub fn recipe(&self) -> Recipe {
        let r = Recipe::new(self.id())
            .with("step", self.step)
            .with("hold", self.hold)
            .with("of", self.of.as_str())
            .with("cover", format!("{}/{}", self.cover.0, self.cover.1))
            .with("min_n", self.min_n);
        match &self.stat {
            Stat::Ent { bins, reference } => {
                let r = r.with("bins", bins.text());
                match reference {
                    Some((a, b)) => r.with("ref", format!("{a}-{b}")),
                    None => r,
                }
            }
            Stat::Perm { m, tau } => r.with("m", m).with("tau", tau),
        }
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

/// A statistic given: its unit, the unit's `x.tel/of-series` edge, and what the report prints.
#[derive(Debug, Clone, PartialEq)]
pub struct Computed {
    pub unit: UnitCore,
    pub relation: Relation,
    pub n: u64,
    pub k: u64,
    pub value: Decimal,
    pub mm: Decimal,
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

/// Compute `spec` over one stream's window, and its reference window when the recipe has one.
pub fn compute(
    spec: &Spec,
    stream: &Stream<'_>,
    read: &Read<'_>,
    reference: Option<&Read<'_>>,
) -> Result<Computed, Refusal> {
    admits(stream.class, spec.of)?;
    let not_run = |m: String| Refusal::new(Code::W703, m);
    let grid = grid::points(read.from_ms, read.to_ms, spec.step);
    if grid.is_empty() {
        return Err(not_run(format!(
            "[{}, {}) holds no multiple of step {}",
            read.from_ms, read.to_ms, spec.step
        )));
    }
    let seq = grid::sequence(read.samples, &grid, spec.hold, spec.step, spec.of);
    let syms = match &spec.stat {
        Stat::Ent { bins, .. } => {
            let edges = match bins {
                Bins::Baseline(k) => {
                    let r = reference.ok_or_else(|| {
                        Refusal::recipe("baseline bins need the reference window's samples")
                    })?;
                    let rgrid = grid::points(r.from_ms, r.to_ms, spec.step);
                    let rseq = grid::sequence(r.samples, &rgrid, spec.hold, spec.step, spec.of);
                    let e = symbols::edges(&rseq, *k);
                    if e.is_empty() {
                        return Err(not_run(format!(
                            "the reference window [{}, {}) gives no value to take edges from",
                            r.from_ms, r.to_ms
                        )));
                    }
                    e
                }
                _ => Vec::new(),
            };
            symbols::symbols(&seq, bins, &edges)
        }
        Stat::Perm { m, tau } => symbols::ordinal(&seq, *m, *tau),
    };
    let g = grid.len() as u64;
    let n = syms.iter().flatten().count() as u64;
    let (a, b) = spec.cover;
    if u128::from(n) * u128::from(b) < u128::from(a) * u128::from(g) {
        return Err(not_run(format!(
            "{n} of {g} grid points give a symbol, under cover {a}/{b}"
        )));
    }
    if n < spec.min_n {
        return Err(not_run(format!("{n} symbols, under min_n {}", spec.min_n)));
    }
    let p =
        entropy::plugin(&entropy::counts(&syms)).ok_or_else(|| not_run("no symbol".to_string()))?;
    let value = num::stored_statistic(p.value)
        .ok_or_else(|| not_run("the statistic is not finite".to_string()))?;
    let mm = num::stored(p.mm).ok_or_else(|| not_run("mm is not finite".to_string()))?;
    let missing = g - n;
    let reference = match spec.stat {
        Stat::Ent {
            bins: Bins::Baseline(_),
            ..
        } => reference,
        _ => None,
    };
    let unit = unit(spec, stream, read, reference, &p, &value, &mm, missing);
    let relation = Relation::new(
        RelKind::Extension(tel::OF_SERIES.to_string()),
        canonical_uid(&unit),
        stream.series,
    );
    Ok(Computed {
        unit,
        relation,
        n: p.n,
        k: p.k,
        value,
        mm,
        missing,
    })
}

/// The `x.tel/info` unit (§3): `derived`, grounded on its series, its source the series
/// observed at the window's end.
#[allow(clippy::too_many_arguments)]
fn unit(
    spec: &Spec,
    stream: &Stream<'_>,
    read: &Read<'_>,
    reference: Option<&Read<'_>>,
    p: &entropy::Plugin,
    value: &Decimal,
    mm: &Decimal,
    missing: u64,
) -> UnitCore {
    let recipe = spec.recipe().text();
    let mut f = Fields::default();
    f.text("tel:stat", spec.stat_name());
    f.text("tel:series", &stream.series.canonical());
    f.text("tel:instrument", &stream.instrument.canonical());
    f.uint("tel:from_ms", read.from_ms);
    f.uint("tel:to_ms", read.to_ms);
    f.text("tel:recipe", &recipe);
    f.uint("tel:n", p.n);
    f.dec("tel:value", value);
    f.bytes("tel:digest", &read.digest);
    f.uint("tel:k", p.k);
    f.dec("tel:mm", mm);
    if missing > 0 {
        f.uint("tel:missing", missing);
    }
    if let Some(r) = reference {
        f.uint("tel:ref_from_ms", r.from_ms);
        f.uint("tel:ref_to_ms", r.to_ms);
        f.bytes("tel:ref_digest", &r.digest);
    }
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::INFO.to_string()),
        tel::gist_of(&format!(
            "{} {} [{}, {}) {}",
            spec.stat_name(),
            stream.key,
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
    .grounds(vec![stream.series])
    .payload(f.payload())
    .build()
    .expect("an info unit is well formed by construction")
}

/// A payload under construction: CBOR values by key, written in the kernel's key order.
#[derive(Default)]
struct Fields(Vec<(Vec<u8>, Vec<u8>)>);

impl Fields {
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
    fn dec(&mut self, k: &str, d: &Decimal) {
        self.put(k, d.to_cbor());
    }
    fn bytes(&mut self, k: &str, b: &[u8; 32]) {
        let mut e = Enc::new();
        e.bytes(b);
        self.put(k, e.into_bytes());
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
}

/// One stream's outcome: a unit, or why there is none.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub series: Uid,
    pub key: String,
    pub instrument: Uid,
    pub result: Result<Computed, Refusal>,
}

/// Run `spec` over each of `series` in `[from_ms, to_ms)`, once per instrument it has samples
/// from, or for `instrument` alone when it is given. An error is the substrate's, or a series
/// it does not hold; a refusal is an outcome.
pub fn run_on<B: Backend>(
    spec: &Spec,
    substrate: &Substrate<B>,
    series: &[Uid],
    instrument: Option<Uid>,
    from_ms: u64,
    to_ms: u64,
) -> Result<Vec<Outcome>, String> {
    let mut out = Vec::new();
    let reference = match spec.stat {
        Stat::Ent {
            reference: Some(w), ..
        } => Some(w),
        _ => None,
    };
    for s in series {
        let row = substrate
            .row(s)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("the substrate holds no sample of series {}", s.canonical()))?;
        let instruments = match instrument {
            Some(i) => vec![i],
            None => substrate.instruments(s).map_err(|e| e.to_string())?,
        };
        for inst in instruments {
            let w = substrate
                .window(s, &inst, from_ms, to_ms)
                .map_err(|e| e.to_string())?;
            let r = match reference {
                Some((a, b)) => Some(
                    substrate
                        .window(s, &inst, a, b)
                        .map_err(|e| e.to_string())?,
                ),
                None => None,
            };
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
            out.push(Outcome {
                series: *s,
                key: row.key.clone(),
                instrument: inst,
                result: compute(spec, &stream, &read, rread.as_ref()),
            });
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
