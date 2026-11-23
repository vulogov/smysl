//! Detectors (RFC SMYSL-3 §21, TL-P5): derived units from the substrate's samples.
//!
//! **Every output records its recipe** (§21): the detector and its version, its parameters, the
//! window and instrument it ran over. The recipe's text is the unit's `tel:recipe`, and its hash
//! the attestation's, so two runs of one recipe over one sample set give one set of uids, and a
//! change of parameter is a different recipe rather than a different answer to the same one.
//!
//! **Replayable by construction** (§17 H-7, GE-M3). A detector's output is a `derived` unit, and
//! GE-M3 requires the same bytes on x86_64 and aarch64, twice each. So a detector here:
//! - reads samples in sample order, and reduces in that order;
//! - uses only `+ − × ÷` and `sqrt`, which IEEE 754 rounds correctly everywhere, and never `ln`,
//!   `exp` or `pow`, which come from a platform's libm;
//! - writes each number by §5.3 (the shortest decimal that round-trips);
//! - draws no randomness (none of the detectors shipped needs any).
//!
//! **Status** (§21's table): every detector output is `derived`, grounded on what it was
//! derived from: a segment or anomaly on its series, an episode on its members, a ranking on the
//! anomalies it ranked. Its attestation is `Op::Transformed` at `Rung::Computed`.

use std::collections::BTreeMap;

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, Decimal, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_series::Sample;

use crate::evidence::counted;
use crate::scrape::{ms, payload};
use crate::sketch::to_f64;

pub mod baro;
pub mod calibrate;
pub mod cusum;
pub mod episode;
pub mod mad;
pub mod pelt;

/// A detector's recipe (§21): which detector, its parameters, and what it ran over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// `cusum/1`, `pelt/1`, …: the detector and its version.
    pub detector: &'static str,
    /// Parameters as their canonical text, by name.
    pub params: BTreeMap<String, String>,
}

impl Recipe {
    pub fn new(detector: &'static str) -> Recipe {
        Recipe {
            detector,
            params: BTreeMap::new(),
        }
    }

    /// A parameter, by name; a `dec` is written by §5.3, so `0.50` and `0.5` are one recipe.
    pub fn with(mut self, name: &str, value: impl ToString) -> Recipe {
        self.params.insert(name.to_string(), value.to_string());
        self
    }

    /// `detector?name=value&…`, names in order: the unit's `tel:recipe`.
    pub fn text(&self) -> String {
        if self.params.is_empty() {
            return self.detector.to_string();
        }
        let p: Vec<String> = self
            .params
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!("{}?{}", self.detector, p.join("&"))
    }

    /// The hash an output's attestation records (§21).
    pub fn hash(&self) -> [u8; 32] {
        smysl_core::hash_bytes(self.text().as_bytes())
    }
}

/// One series, one instrument, as a detector reads it.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    pub series: Uid,
    pub scheme: &'a str,
    pub key: &'a str,
    pub instrument: Uid,
    /// In sample order (a window read).
    pub samples: &'a [Sample],
}

impl Input<'_> {
    /// The counted samples as `(ts_ms, value)`, in sample order. A stale or no-value sample is
    /// no reading, so no detector sees it.
    pub fn points(&self) -> Vec<(u64, f64)> {
        self.samples
            .iter()
            .filter(|s| counted(s))
            .map(|s| (s.ts_ms(), to_f64(s.value_dec())))
            .collect()
    }

    fn source(&self, observed: u64) -> SourceRef {
        SourceRef::new(
            SourceKind::Metric,
            format!("{}{}:{}", tel::SERIES_REF, self.scheme, self.key),
        )
        .observed_at(observed)
    }
}

/// A finite binary64 as a `dec` (§5.3); a non-finite one is not a value, and has no field.
pub fn dec(x: f64) -> Option<Decimal> {
    Decimal::from_f64(x).ok()
}

fn dec_field(d: &Decimal) -> Option<HValue> {
    d.to_payload_value()
}

/// An `x.tel/segment` (§4.1): a regime of `input` over `[from_ms, to_ms)`, at `level`.
pub fn segment(
    input: &Input<'_>,
    from_ms: u64,
    to_ms: u64,
    recipe: &Recipe,
    level: Option<f64>,
    slope: Option<f64>,
) -> UnitCore {
    let mut fields = vec![
        ("tel:series", HValue::Str(input.series.canonical())),
        ("tel:from_ms", ms(from_ms)),
        ("tel:to_ms", ms(to_ms)),
        ("tel:recipe", HValue::Str(recipe.text())),
    ];
    if let Some(v) = level.and_then(dec).as_ref().and_then(dec_field) {
        fields.push(("tel:level", v));
    }
    if let Some(v) = slope.and_then(dec).as_ref().and_then(dec_field) {
        fields.push(("tel:slope", v));
    }
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::SEGMENT.to_string()),
        tel::gist_of(&format!("segment {} [{from_ms}, {to_ms})", input.key)),
        Status::Derived,
    )
    .source(input.source(to_ms))
    .grounds(vec![input.series])
    .payload(payload(&fields))
    .build()
    .expect("a segment unit is well formed by construction")
}

/// Which way an anomaly departs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Up => "up",
            Direction::Down => "down",
        }
    }
}

/// An `x.tel/anomaly` (§4.1): `input` over `[from_ms, to_ms)` judged anomalous, with `score`.
pub fn anomaly(
    input: &Input<'_>,
    from_ms: u64,
    to_ms: u64,
    recipe: &Recipe,
    score: f64,
    direction: Option<Direction>,
) -> Option<UnitCore> {
    let score = dec(score)?.to_payload_value()?;
    let mut fields = vec![
        ("tel:series", HValue::Str(input.series.canonical())),
        ("tel:from_ms", ms(from_ms)),
        ("tel:to_ms", ms(to_ms)),
        ("tel:recipe", HValue::Str(recipe.text())),
        ("tel:score", score),
    ];
    if let Some(d) = direction {
        fields.push(("tel:direction", HValue::Str(d.as_str().into())));
    }
    Some(
        UnitCoreBuilder::new(
            SchemaId::Extension(tel::ANOMALY.to_string()),
            tel::gist_of(&format!("anomaly {} [{from_ms}, {to_ms})", input.key)),
            Status::Derived,
        )
        .source(input.source(to_ms))
        .grounds(vec![input.series])
        .payload(payload(&fields))
        .build()
        .expect("an anomaly unit is well formed by construction"),
    )
}

/// The uids of `units`, for comparing runs.
pub fn uids(units: &[UnitCore]) -> Vec<Uid> {
    units.iter().map(canonical_uid).collect()
}

/// A detector recipe file (RFC SMYSL-3 §21, §23): which detector, and its parameters, as HJSON.
///
/// ```hjson
/// { detector: cusum/1, k: 0.5, h: 5, baseline: 10 }
/// { detector: pelt/1, penalty: 10, min_size: 2 }
/// { detector: mad/1, threshold: 3.5, baseline: "1768550400000-1768554000000" }
/// { detector: episode/1, gap: 300000, min: 2 }
/// { detector: baro/1, at: 1768557600000, pre: 3600000, post: 600000, top: 5 }
/// ```
///
/// A parameter left out takes its default. A decimal parameter may be a string, so that `0.10`
/// is read as written rather than through a binary64; either way its §5.3 spelling is what the
/// recipe records. An unknown key is refused, because a parameter nobody reads is a parameter
/// somebody believes is in force.
#[derive(Debug, Clone, PartialEq)]
pub enum Spec {
    Cusum(cusum::Params),
    Pelt(pelt::Params),
    Mad(mad::Params),
    Episode(episode::Params),
    Baro(baro::Params),
}

impl Spec {
    pub fn parse(text: &str) -> Result<Spec, String> {
        let o = smysl_core::surface::parse_object(text, 0)
            .map_err(|e| e.to_string())?
            .value;
        let get = |k: &str| o.get(k).map(|v| &v.value);
        let detector = match get("detector") {
            Some(HValue::Str(s)) => s.as_str(),
            _ => return Err("a recipe names its `detector`".into()),
        };
        let known: &[&str] = match detector {
            cusum::ID => &["k", "h", "baseline"],
            pelt::ID => &["penalty", "min_size"],
            mad::ID => &["threshold", "baseline"],
            episode::ID => &["gap", "min"],
            baro::ID => &["at", "pre", "post", "top"],
            other => {
                return Err(format!(
                    "`{other}` is not a detector: {}, {}, {}, {} or {}",
                    cusum::ID,
                    pelt::ID,
                    mad::ID,
                    episode::ID,
                    baro::ID
                ))
            }
        };
        for (k, _) in o.iter() {
            if k.value != "detector" && !known.contains(&k.value.as_str()) {
                return Err(format!("`{}` is not a parameter of {detector}", k.value));
            }
        }
        let decimal = |k: &str| -> Result<Option<Decimal>, String> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Str(s)) => Decimal::parse_token(s)
                    .map(Some)
                    .map_err(|e| format!("`{k}`: {e}")),
                Some(HValue::Int(n)) => Ok(Some(Decimal::from_i64(*n))),
                Some(HValue::Float(x)) => Decimal::from_f64(*x)
                    .map(Some)
                    .map_err(|e| format!("`{k}`: {e}")),
                Some(_) => Err(format!("`{k}` is a number")),
            }
        };
        let count = |k: &str| -> Result<Option<u64>, String> {
            match get(k) {
                None => Ok(None),
                Some(HValue::Int(n)) if *n >= 0 => Ok(Some(*n as u64)),
                Some(_) => Err(format!("`{k}` is a non-negative integer")),
            }
        };
        let positive = |k: &str, d: Option<Decimal>| -> Result<Option<Decimal>, String> {
            match d {
                Some(v) if v.mantissa() <= 0 => Err(format!("`{k}` is positive")),
                other => Ok(other),
            }
        };
        Ok(match detector {
            cusum::ID => {
                let d = cusum::Params::default();
                Spec::Cusum(cusum::Params {
                    k: decimal("k")?.unwrap_or(d.k),
                    h: positive("h", decimal("h")?)?.unwrap_or(d.h),
                    baseline: count("baseline")?.map_or(d.baseline, |n| n as usize),
                })
            }
            pelt::ID => {
                let d = pelt::Params::default();
                Spec::Pelt(pelt::Params {
                    penalty: positive("penalty", decimal("penalty")?)?.unwrap_or(d.penalty),
                    min_size: count("min_size")?.map_or(d.min_size, |n| n as usize),
                })
            }
            mad::ID => {
                let (from, to) = match get("baseline") {
                    Some(HValue::Str(s)) => s
                        .split_once('-')
                        .and_then(|(a, b)| Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?)))
                        .filter(|(a, b)| a < b)
                        .ok_or_else(|| format!("`baseline` is \"FROM-TO\", epoch ms, not {s:?}"))?,
                    _ => return Err("mad/1 needs `baseline: \"FROM-TO\"`, epoch ms".into()),
                };
                let mut p = mad::Params::new(from, to);
                if let Some(t) = positive("threshold", decimal("threshold")?)? {
                    p.threshold = t;
                }
                Spec::Mad(p)
            }
            episode::ID => {
                let d = episode::Params::default();
                Spec::Episode(episode::Params {
                    gap_ms: count("gap")?.unwrap_or(d.gap_ms),
                    min_members: count("min")?.map_or(d.min_members, |n| n as usize),
                })
            }
            _ => {
                let at =
                    count("at")?.ok_or("baro/1 needs `at`, the anomaly instant in epoch ms")?;
                let d = baro::Params::new(at);
                Spec::Baro(baro::Params {
                    at_ms: at,
                    pre_ms: count("pre")?.unwrap_or(d.pre_ms),
                    post_ms: count("post")?.unwrap_or(d.post_ms),
                    top: count("top")?.map_or(d.top, |n| n as usize),
                })
            }
        })
    }

    /// The recipe as HJSON that [`Spec::parse`] reads back to this `Spec`, every parameter
    /// written; a decimal is quoted, so it is read as written.
    pub fn hjson(&self) -> String {
        let body = match self {
            Spec::Cusum(p) => format!(r#"k: "{}", h: "{}", baseline: {}"#, p.k, p.h, p.baseline),
            Spec::Pelt(p) => format!(r#"penalty: "{}", min_size: {}"#, p.penalty, p.min_size),
            Spec::Mad(p) => format!(
                r#"threshold: "{}", baseline: "{}-{}""#,
                p.threshold, p.baseline_from, p.baseline_to
            ),
            Spec::Episode(p) => format!("gap: {}, min: {}", p.gap_ms, p.min_members),
            Spec::Baro(p) => format!(
                "at: {}, pre: {}, post: {}, top: {}",
                p.at_ms, p.pre_ms, p.post_ms, p.top
            ),
        };
        format!("{{ detector: {}, {body} }}", self.id())
    }

    /// The detector's id.
    pub fn id(&self) -> &'static str {
        match self {
            Spec::Cusum(_) => cusum::ID,
            Spec::Pelt(_) => pelt::ID,
            Spec::Mad(_) => mad::ID,
            Spec::Episode(_) => episode::ID,
            Spec::Baro(_) => baro::ID,
        }
    }
}

#[cfg(test)]
mod spec_tests {
    use super::*;

    #[test]
    fn recipes_parse_with_defaults_and_refuse_what_nobody_reads() {
        assert_eq!(
            Spec::parse("{ detector: cusum/1 }").unwrap(),
            Spec::Cusum(cusum::Params::default())
        );
        let Spec::Cusum(p) = Spec::parse(r#"{ detector: cusum/1, k: "0.10", h: 4 }"#).unwrap()
        else {
            panic!()
        };
        assert_eq!(p.k.to_string(), "0.1");
        let Spec::Mad(m) =
            Spec::parse(r#"{ detector: mad/1, baseline: "100-200", threshold: 3 }"#).unwrap()
        else {
            panic!()
        };
        assert_eq!((m.baseline_from, m.baseline_to), (100, 200));
        for text in [
            r#"{ detector: cusum/1, k: "0.10", h: "1.2e7" }"#,
            "{ detector: pelt/1, penalty: 250 }",
            r#"{ detector: mad/1, baseline: "100-200", threshold: "4.5" }"#,
            "{ detector: episode/1 }",
            "{ detector: baro/1, at: 1000 }",
        ] {
            let s = Spec::parse(text).unwrap();
            assert_eq!(Spec::parse(&s.hjson()).unwrap(), s, "{}", s.hjson());
        }
        for bad in [
            "{ }",
            "{ detector: lstm/1 }",
            "{ detector: cusum/1, kk: 1 }",
            "{ detector: cusum/1, h: -1 }",
            "{ detector: mad/1 }",
            r#"{ detector: mad/1, baseline: "200-100" }"#,
            "{ detector: baro/1 }",
            "{ detector: pelt/1, min_size: -2 }",
        ] {
            assert!(Spec::parse(bad).is_err(), "{bad}");
        }
    }
}

/// What a run of a recipe produced: its units, and their edges (`x.tel/of-series` from each
/// segment and anomaly to its series, `x.tel/during` from each episode member to its episode).
#[derive(Debug, Clone, Default)]
pub struct Detected {
    pub units: Vec<UnitCore>,
    pub relations: Vec<smysl_core::Relation>,
}

/// Run `spec` over `substrate` (RFC SMYSL-3 §23 `tel detect`), the one path the CLI and GE-M3
/// share.
///
/// - CUSUM, PELT and robust z run over each of `series`, per instrument it has samples from, in
///   `[from_ms, to_ms)`; robust z reads its baseline window from the same substrate.
/// - The BARO-style ranking takes every `(series, instrument)` as a candidate, around its `at`.
/// - Episodes group `members` (units from a store) in `[from_ms, to_ms)`.
///
/// Series are taken in the order given and instruments in uid order, so the output's order is a
/// function of the arguments too.
pub fn run_on<B: smysl_series::Backend>(
    spec: &Spec,
    substrate: &smysl_series::Substrate<B>,
    series: &[Uid],
    from_ms: u64,
    to_ms: u64,
    members: &[UnitCore],
    trigger: Option<Uid>,
) -> Result<Detected, String> {
    let mut out = Detected::default();
    if let Spec::Episode(p) = spec {
        for e in episode::run(members, from_ms, to_ms, p) {
            out.relations.extend(e.relations);
            out.units.push(e.unit);
        }
        return Ok(out);
    }
    // Every (series, instrument) and its row.
    let mut streams = Vec::new();
    for s in series {
        let row = substrate
            .row(s)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("the substrate holds no sample of series {}", s.canonical()))?;
        for inst in substrate.instruments(s).map_err(|e| e.to_string())? {
            streams.push((*s, inst, row.scheme.clone(), row.key.clone()));
        }
    }
    let read = |s: &Uid, i: &Uid, a: u64, b: u64| -> Result<Vec<Sample>, String> {
        if a >= b {
            return Ok(Vec::new());
        }
        Ok(substrate
            .window(s, i, a, b)
            .map_err(|e| e.to_string())?
            .samples)
    };
    if let Spec::Baro(p) = spec {
        let (a, b) = (
            p.at_ms.saturating_sub(p.pre_ms),
            p.at_ms.saturating_add(p.post_ms),
        );
        let reads: Vec<Vec<Sample>> = streams
            .iter()
            .map(|(s, i, _, _)| read(s, i, a, b))
            .collect::<Result<_, _>>()?;
        let inputs: Vec<Input<'_>> = streams
            .iter()
            .zip(&reads)
            .map(|((s, i, scheme, key), samples)| Input {
                series: *s,
                scheme,
                key,
                instrument: *i,
                samples,
            })
            .collect();
        out.units = baro::run(&inputs, trigger, p);
        return Ok(out);
    }
    for (s, i, scheme, key) in &streams {
        let samples = read(s, i, from_ms, to_ms)?;
        let input = Input {
            series: *s,
            scheme,
            key,
            instrument: *i,
            samples: &samples,
        };
        let units = match spec {
            Spec::Cusum(p) => cusum::run(&input, from_ms, to_ms, p),
            Spec::Pelt(p) => pelt::run(&input, from_ms, to_ms, p),
            Spec::Mad(p) => {
                let base = read(s, i, p.baseline_from, p.baseline_to)?;
                mad::run(&input, &base, from_ms, to_ms, p)
            }
            Spec::Episode(_) | Spec::Baro(_) => unreachable!("handled above"),
        };
        for u in units {
            out.relations.push(smysl_core::Relation::new(
                smysl_core::RelKind::Extension(tel::OF_SERIES.to_string()),
                canonical_uid(&u),
                *s,
            ));
            out.units.push(u);
        }
    }
    Ok(out)
}

/// Calibrate `spec` over `substrate` (RFC SMYSL-3 §21.1, `tel calibrate`): one trial per window
/// `(reference_from, test_from, test_to)` in `windows`, each of `series` per instrument it has
/// samples from read as a [`calibrate::Pair`] of `[reference_from, test_from)` and `[test_from,
/// test_to)`. The windows are ones the caller holds fault-free.
pub fn calibrate_on<B: smysl_series::Backend>(
    spec: &Spec,
    substrate: &smysl_series::Substrate<B>,
    series: &[Uid],
    windows: &[(u64, u64, u64)],
    rate: Decimal,
) -> Result<calibrate::Calibrated, String> {
    let mut streams = Vec::new();
    for s in series {
        if substrate.row(s).map_err(|e| e.to_string())?.is_none() {
            return Err(format!(
                "the substrate holds no sample of series {}",
                s.canonical()
            ));
        }
        for inst in substrate.instruments(s).map_err(|e| e.to_string())? {
            streams.push((*s, inst));
        }
    }
    let points = |s: &Uid, i: &Uid, a: u64, b: u64| -> Result<Vec<(u64, f64)>, String> {
        if a >= b {
            return Ok(Vec::new());
        }
        Ok(substrate
            .window(s, i, a, b)
            .map_err(|e| e.to_string())?
            .samples
            .iter()
            .filter(|x| counted(x))
            .map(|x| (x.ts_ms(), to_f64(x.value_dec())))
            .collect())
    };
    let mut read = Vec::new();
    for &(r0, t0, t1) in windows {
        if !(r0 < t0 && t0 < t1) {
            return Err(format!(
                "a calibration window is REF-FROM-TO with REF < FROM < TO, not {r0}-{t0}-{t1}"
            ));
        }
        let mut pairs = Vec::new();
        for (s, i) in &streams {
            pairs.push((points(s, i, r0, t0)?, points(s, i, t0, t1)?));
        }
        read.push(pairs);
    }
    let trials: Vec<Vec<calibrate::Pair<'_>>> = read
        .iter()
        .map(|pairs| {
            pairs
                .iter()
                .map(|(r, t)| calibrate::Pair {
                    reference: r,
                    test: t,
                })
                .collect()
        })
        .collect();
    calibrate::calibrate(spec, &trials, rate)
}
