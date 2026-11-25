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

/// How a level detector floors its scale (RFC SMYSL-3 §21.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Floor {
    /// The shipped floor: a scale of zero becomes `10⁻⁹` (CUSUM, MAD) or `10⁻¹⁸` (PELT's
    /// variance) of the level, so any departure from a constant is a change. No recipe key.
    #[default]
    Tiny,
    /// `floor: resolution`: the scale is at least the series' **resolution**, the smallest
    /// positive gap between two of its distinct values in what the detector reads, so a series
    /// that moves by one step of its own quantum is not read as an astronomical change.
    Resolution,
    /// `floor: spread`: the scale is at least a tenth of the **spread** of what the detector
    /// reads, its standard deviation, so a step-like float series, whose differences are
    /// mostly zero and whose resolution is noise-sized, is read on its own scale. It is zero
    /// only for a constant series, where the shipped floor applies.
    Spread,
}

/// What a recipe may carry beside its detector's parameters (§21.2). The default is what a
/// recipe without these keys has always meant, so its text and uids do not change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Options {
    pub floor: Floor,
    /// `step` and `agg` (RFC SMYSL-9 §4.2): the detector reads the series resampled.
    pub resample: Option<Resample>,
}

/// A coarser cadence a level detector reads its series at: `step: MS, agg: last|mean|max` in a
/// recipe, by [`smysl_series::resample()`], each window aligned to its own start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Resample {
    pub step_ms: u64,
    pub agg: smysl_series::Agg,
}

impl Resample {
    pub fn new(step_ms: u64, agg: smysl_series::Agg) -> Resample {
        Resample { step_ms, agg }
    }
}

/// The fewest points a calibration window may hold once resampled (RFC SMYSL-9 §4.2): fewer is
/// refused, naming the window. Twice CUSUM's default `baseline`, and more than PELT's or MAD's
/// own minimum.
pub const MIN_N: usize = 20;

impl Options {
    /// The options with `floor` set.
    pub fn with_floor(mut self, floor: Floor) -> Options {
        self.floor = floor;
        self
    }

    /// The options with `resample` set.
    pub fn with_resample(mut self, r: Resample) -> Options {
        self.resample = Some(r);
        self
    }

    /// `recipe` with these options' keys: none for the defaults.
    pub fn stamp(&self, recipe: Recipe) -> Recipe {
        let recipe = match self.floor {
            Floor::Tiny => recipe,
            Floor::Resolution => recipe.with("floor", "resolution"),
            Floor::Spread => recipe.with("floor", "spread"),
        };
        match self.resample {
            None => recipe,
            Some(r) => recipe.with("step", r.step_ms).with("agg", r.agg.as_str()),
        }
    }

    /// `samples` (a window read from `from_ms`) as the detector reads them: resampled when the
    /// options say so, and otherwise as they are.
    pub fn read(&self, samples: Vec<Sample>, from_ms: u64) -> Vec<Sample> {
        match self.resample {
            None => samples,
            Some(r) => smysl_series::resample(&samples, from_ms, r.step_ms, r.agg),
        }
    }

    /// The scale floor these options put under `values`, if any: their resolution under
    /// [`Floor::Resolution`], their spread under [`Floor::Spread`], none under the shipped
    /// floor or when no two values differ.
    pub fn floor_of(&self, values: impl Iterator<Item = f64>) -> Option<f64> {
        match self.floor {
            Floor::Tiny => None,
            Floor::Resolution => resolution(values),
            Floor::Spread => spread(values),
        }
    }
}

/// The smallest positive gap between two distinct values, by total order; none when fewer than
/// two values differ.
pub fn resolution(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut v: Vec<f64> = values.filter(|x| x.is_finite()).collect();
    v.sort_by(f64::total_cmp);
    v.windows(2)
        .map(|w| w[1] - w[0])
        .filter(|d| *d > 0.0)
        .reduce(f64::min)
}

/// A tenth of the standard deviation of the finite values, `√(Σ(x − x̄)² / n) / 10`, summed in
/// order so it replays to the byte; none when fewer than two values differ.
pub fn spread(values: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = values.filter(|x| x.is_finite()).collect();
    if v.len() < 2 {
        return None;
    }
    let n = v.len() as f64;
    let mean = v.iter().sum::<f64>() / n;
    let var = v.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / n;
    let sd = var.sqrt() / 10.0;
    (sd > 0.0).then_some(sd)
}

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
        Spec::parse_inner(text, false).map(|(s, _)| s)
    }

    /// [`Spec::parse`], also admitting the [`Options`] keys (§21.2): `floor: resolution`,
    /// `floor: spread` (or `tiny`, the default) on `cusum/1`, `pelt/1` and `mad/1`. A recipe without them parses to
    /// the default options, so it means what it always has.
    pub fn parse_with_options(text: &str) -> Result<(Spec, Options), String> {
        Spec::parse_inner(text, true)
    }

    fn parse_inner(text: &str, options: bool) -> Result<(Spec, Options), String> {
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
        let level = matches!(detector, cusum::ID | pelt::ID | mad::ID);
        for (k, _) in o.iter() {
            let option = options && level && matches!(k.value.as_str(), "floor" | "step" | "agg");
            if k.value != "detector" && !known.contains(&k.value.as_str()) && !option {
                return Err(format!("`{}` is not a parameter of {detector}", k.value));
            }
        }
        let opts = match get("floor") {
            None => Options::default(),
            Some(HValue::Str(s)) if s == "resolution" => {
                Options::default().with_floor(Floor::Resolution)
            }
            Some(HValue::Str(s)) if s == "spread" => Options::default().with_floor(Floor::Spread),
            Some(HValue::Str(s)) if s == "tiny" => Options::default(),
            Some(_) => return Err("`floor` is `resolution`, `spread` or `tiny`".into()),
        };
        let opts = match (get("step"), get("agg")) {
            (None, None) => opts,
            (None, Some(_)) => return Err("`agg` goes with `step`".into()),
            (Some(step), agg) => {
                let step_ms = match step {
                    HValue::Int(n) if *n > 0 => *n as u64,
                    _ => return Err("`step` is a positive integer, in ms".into()),
                };
                let agg = match agg {
                    None => smysl_series::Agg::Last,
                    Some(HValue::Str(a)) => {
                        smysl_series::Agg::parse(a).ok_or("`agg` is `last`, `mean` or `max`")?
                    }
                    Some(_) => return Err("`agg` is `last`, `mean` or `max`".into()),
                };
                opts.with_resample(Resample::new(step_ms, agg))
            }
        };
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
        let spec = match detector {
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
        };
        Ok((spec, opts))
    }

    /// The recipe as HJSON that [`Spec::parse_with_options`] reads back to this `Spec` and
    /// `opts`, every parameter written and the options only when not the default (so that
    /// [`Spec::parse`] reads it too); a decimal is quoted, so it is read as written.
    pub fn hjson(&self, opts: &Options) -> String {
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
        let floor = match opts.floor {
            Floor::Tiny => "",
            Floor::Resolution => ", floor: resolution",
            Floor::Spread => ", floor: spread",
        };
        let step = match opts.resample {
            None => String::new(),
            Some(r) => format!(", step: {}, agg: {}", r.step_ms, r.agg.as_str()),
        };
        format!("{{ detector: {}, {body}{floor}{step} }}", self.id())
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
    fn resolution_is_the_smallest_gap_between_distinct_values() {
        assert_eq!(resolution([3.0, 1.0, 1.0, 1.5, 7.0].into_iter()), Some(0.5));
        assert_eq!(resolution([2.0, 2.0].into_iter()), None);
        assert_eq!(resolution(std::iter::empty()), None);
        assert_eq!(resolution([1.0, f64::NAN, 4.0].into_iter()), Some(3.0));
        // The spread: a tenth of the standard deviation, none for a constant series.
        assert_eq!(spread([1.0, 3.0, 1.0, 3.0].into_iter()), Some(0.1));
        assert_eq!(spread([2.0, 2.0, f64::NAN].into_iter()), None);
        assert_eq!(spread([5.0].into_iter()), None);
    }

    #[test]
    fn the_default_options_change_no_byte_and_a_floor_is_recorded() {
        use smysl_series::Sample;
        // Flat, then one quantum up for a stretch.
        let values: Vec<f64> = (0..60)
            .map(|i| if (30..45).contains(&i) { 6.0 } else { 5.0 })
            .collect();
        let samples: Vec<Sample> = values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                Sample::new(1_000 * i as u64, None, Decimal::from_f64(*v).unwrap(), 0).unwrap()
            })
            .collect();
        let input = Input {
            series: Uid::from_bytes([1; 32]),
            scheme: "prom",
            key: "flat",
            instrument: Uid::from_bytes([2; 32]),
            samples: &samples,
        };
        let floor = Options::default().with_floor(Floor::Resolution);
        let uids = |us: &[UnitCore]| us.iter().map(canonical_uid).collect::<Vec<_>>();
        let (c, p) = (cusum::Params::default(), pelt::Params::default());
        let m = mad::Params::new(0, 30_000);
        assert_eq!(
            uids(&cusum::run(&input, 0, 60_000, &c)),
            uids(&cusum::run_with(&input, 0, 60_000, &c, &Options::default()))
        );
        assert_eq!(
            uids(&pelt::run(&input, 0, 60_000, &p)),
            uids(&pelt::run_with(&input, 0, 60_000, &p, &Options::default()))
        );
        assert_eq!(
            uids(&mad::run(&input, &samples, 30_000, 60_000, &m)),
            uids(&mad::run_with(
                &input,
                &samples,
                30_000,
                60_000,
                &m,
                &Options::default()
            ))
        );
        let stamped = cusum::run_with(&input, 0, 60_000, &c, &floor);
        assert!(!stamped.is_empty());
        for u in &stamped {
            let bytes = u.payload.as_deref().unwrap();
            let text = String::from_utf8_lossy(bytes);
            assert!(text.contains("floor=resolution"), "{text}");
        }
        assert_ne!(uids(&stamped), uids(&cusum::run(&input, 0, 60_000, &c)));
        // One quantum is one robust z under the floor, not 10⁹.
        let z = mad::run_with(&input, &samples, 30_000, 60_000, &m, &floor);
        assert!(
            z.is_empty(),
            "a one-quantum move is under 3.5 z at the resolution floor"
        );
        assert!(!mad::run(&input, &samples, 30_000, 60_000, &m).is_empty());
    }

    #[test]
    fn a_step_is_parsed_written_back_and_stamped_and_its_absence_changes_nothing() {
        use smysl_series::Agg;
        let (s, o) =
            Spec::parse_with_options("{ detector: cusum/1, floor: resolution, step: 15000 }")
                .unwrap();
        assert_eq!(o.resample, Some(Resample::new(15_000, Agg::Last)));
        assert!(s
            .hjson(&o)
            .ends_with(", floor: resolution, step: 15000, agg: last }"));
        assert_eq!(
            Spec::parse_with_options(&s.hjson(&o)).unwrap(),
            (s.clone(), o)
        );
        let (_, o) =
            Spec::parse_with_options("{ detector: pelt/1, step: 5000, agg: max }").unwrap();
        assert_eq!(o.resample, Some(Resample::new(5_000, Agg::Max)));
        let Spec::Cusum(p) = s else { panic!() };
        assert_eq!(
            o.stamp(p.recipe(0, 10)).text(),
            "cusum/1?agg=max&baseline=10&from=0&h=5&k=0.5&step=5000&to=10"
        );
        // Without the keys, the recipe's text is what it was.
        assert_eq!(
            Options::default().stamp(p.recipe(0, 10)).text(),
            "cusum/1?baseline=10&from=0&h=5&k=0.5&to=10"
        );
        for bad in [
            "{ detector: cusum/1, agg: last }",
            "{ detector: cusum/1, step: 0 }",
            "{ detector: cusum/1, step: \"15s\" }",
            "{ detector: cusum/1, step: 1000, agg: min }",
            "{ detector: episode/1, step: 1000 }",
        ] {
            assert!(Spec::parse_with_options(bad).is_err(), "{bad}");
        }
        assert!(Spec::parse("{ detector: cusum/1, step: 1000 }").is_err());
    }

    fn one_second(n: u64, at: u64) -> smysl_series::Substrate<smysl_series::MemBackend> {
        use smysl_series::{MemBackend, Sample, SeriesInfo, Substrate};
        let mut sub = Substrate::new(MemBackend::default());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key: "cpu".into(),
            class: "gauge".into(),
        };
        // Level 5 with a little alternation, and 9 from `at` on.
        let samples: Vec<Sample> = (0..n)
            .map(|i| {
                let v = if i * 1_000 >= at {
                    9
                } else {
                    5 + (i % 2) as i64
                };
                Sample::value(i * 1_000, Decimal::from_i64(v))
            })
            .collect();
        sub.add(
            &Uid::from_bytes([1; 32]),
            &Uid::from_bytes([2; 32]),
            &info,
            &samples,
        )
        .unwrap();
        sub
    }

    #[test]
    fn a_detector_reads_the_resampled_series_and_says_so() {
        use smysl_series::Agg;
        let sub = one_second(1_200, 900_000);
        let series = [Uid::from_bytes([1; 32])];
        let spec = Spec::parse(r#"{ detector: mad/1, baseline: "0-600000" }"#).unwrap();
        let step = Options::default().with_resample(Resample::new(15_000, Agg::Last));
        let plain = run_on_with(
            &spec,
            &Options::default(),
            &sub,
            &series,
            600_000,
            1_200_000,
            &[],
            None,
        )
        .unwrap();
        let coarse =
            run_on_with(&spec, &step, &sub, &series, 600_000, 1_200_000, &[], None).unwrap();
        assert!(!plain.units.is_empty() && !coarse.units.is_empty());
        for u in &coarse.units {
            let text = String::from_utf8_lossy(u.payload.as_deref().unwrap()).to_string();
            assert!(
                text.contains("agg=last") && text.contains("step=15000"),
                "{text}"
            );
        }
        // At 15 s, `Last` reads every fifteenth sample: the alternation is gone, all odd.
        let o = Options::default().with_resample(Resample::new(15_000, Agg::Last));
        let read = o.read(
            sub.window(&series[0], &Uid::from_bytes([2; 32]), 0, 60_000)
                .unwrap()
                .samples,
            0,
        );
        assert_eq!(
            read.iter().map(|s| s.ts_ms()).collect::<Vec<_>>(),
            [14_000, 29_000, 44_000, 59_000]
        );
    }

    #[test]
    fn calibration_refuses_a_window_too_short_at_its_step_and_names_it() {
        use smysl_series::Agg;
        let sub = one_second(1_200, u64::MAX);
        let series = [Uid::from_bytes([1; 32])];
        let spec = Spec::parse("{ detector: cusum/1 }").unwrap();
        let target = calibrate::Target::new(Decimal::parse_token("0.5").unwrap()).unwrap();
        let windows = [(0, 360_000, 720_000)];
        // 360 points a window at 1 s; 24 at 15 s; 6 at 60 s.
        let at = |ms| Options::default().with_resample(Resample::new(ms, Agg::Last));
        assert!(calibrate_on(&spec, &Options::default(), &sub, &series, &windows, target).is_ok());
        assert!(calibrate_on(&spec, &at(15_000), &sub, &series, &windows, target).is_ok());
        let e = calibrate_on(&spec, &at(60_000), &sub, &series, &windows, target).unwrap_err();
        assert!(
            e.contains("0-360000") && e.contains("6 point(s)") && e.contains("fewer than 20"),
            "{e}"
        );
        // Unresampled, a short window is calibrated as before: the bound is the step's.
        let short = [(0, 10_000, 20_000)];
        assert!(calibrate_on(&spec, &Options::default(), &sub, &series, &short, target).is_ok());
    }

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
            let h = s.hjson(&Options::default());
            assert_eq!(Spec::parse(&h).unwrap(), s, "{h}");
        }
        // The options: admitted only by parse_with_options, on the level detectors, and
        // written back.
        let r = r#"{ detector: cusum/1, h: 3, floor: resolution }"#;
        assert!(Spec::parse(r).is_err(), "plain parse refuses an option");
        let (s, o) = Spec::parse_with_options(r).unwrap();
        assert_eq!(o.floor, Floor::Resolution);
        assert_eq!(Spec::parse_with_options(&s.hjson(&o)).unwrap(), (s, o));
        let (s, o) = Spec::parse_with_options("{ detector: pelt/1, floor: spread }").unwrap();
        assert_eq!(o.floor, Floor::Spread);
        assert!(
            s.hjson(&o).ends_with(", floor: spread }"),
            "{}",
            s.hjson(&o)
        );
        assert_eq!(Spec::parse_with_options(&s.hjson(&o)).unwrap(), (s, o));
        assert_eq!(
            Spec::parse_with_options("{ detector: pelt/1 }").unwrap().1,
            Options::default()
        );
        for bad in [
            "{ detector: episode/1, floor: resolution }",
            "{ detector: mad/1, baseline: \"1-2\", floor: coarse }",
        ] {
            assert!(Spec::parse_with_options(bad).is_err(), "{bad}");
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
    run_on_with(
        spec,
        &Options::default(),
        substrate,
        series,
        from_ms,
        to_ms,
        members,
        trigger,
    )
}

/// [`run_on`] under `opts` (§21.2), which reach CUSUM, PELT and robust z.
#[allow(clippy::too_many_arguments)]
pub fn run_on_with<B: smysl_series::Backend>(
    spec: &Spec,
    opts: &Options,
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
        let samples = opts.read(read(s, i, from_ms, to_ms)?, from_ms);
        let input = Input {
            series: *s,
            scheme,
            key,
            instrument: *i,
            samples: &samples,
        };
        let units = match spec {
            Spec::Cusum(p) => cusum::run_with(&input, from_ms, to_ms, p, opts),
            Spec::Pelt(p) => pelt::run_with(&input, from_ms, to_ms, p, opts),
            Spec::Mad(p) => {
                let base = opts.read(read(s, i, p.baseline_from, p.baseline_to)?, p.baseline_from);
                mad::run_with(&input, &base, from_ms, to_ms, p, opts)
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
    opts: &Options,
    substrate: &smysl_series::Substrate<B>,
    series: &[Uid],
    windows: &[(u64, u64, u64)],
    target: calibrate::Target,
) -> Result<calibrate::Calibrated, String> {
    let read = read_trials(opts, substrate, series, windows)?;
    let trials: Vec<Vec<calibrate::Pair<'_>>> = read
        .iter()
        .map(|t| {
            t.values()
                .map(|(r, x)| calibrate::Pair {
                    reference: r,
                    test: x,
                })
                .collect()
        })
        .collect();
    calibrate::calibrate(spec, opts, &trials, target)
}

/// [`calibrate_on`], per series (`calibrate::calibrate_each`): each `(series, instrument)` its
/// own value, the rate held over the windows jointly.
pub fn calibrate_on_each<B: smysl_series::Backend>(
    spec: &Spec,
    opts: &Options,
    substrate: &smysl_series::Substrate<B>,
    series: &[Uid],
    windows: &[(u64, u64, u64)],
    target: calibrate::Target,
) -> Result<calibrate::CalibratedEach<(Uid, Uid)>, String> {
    let read = read_trials(opts, substrate, series, windows)?;
    let trials: Vec<BTreeMap<(Uid, Uid), calibrate::Pair<'_>>> = read
        .iter()
        .map(|t| {
            t.iter()
                .map(|(k, (r, x))| {
                    (
                        *k,
                        calibrate::Pair {
                            reference: r,
                            test: x,
                        },
                    )
                })
                .collect()
        })
        .collect();
    calibrate::calibrate_each(spec, opts, &trials, target)
}

/// Each window's `(reference, test)` points, by `(series, instrument)`.
type Trial = BTreeMap<(Uid, Uid), (Vec<(u64, f64)>, Vec<(u64, f64)>)>;

fn read_trials<B: smysl_series::Backend>(
    opts: &Options,
    substrate: &smysl_series::Substrate<B>,
    series: &[Uid],
    windows: &[(u64, u64, u64)],
) -> Result<Vec<Trial>, String> {
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
        let samples = substrate
            .window(s, i, a, b)
            .map_err(|e| e.to_string())?
            .samples;
        let points: Vec<(u64, f64)> = opts
            .read(samples, a)
            .iter()
            .filter(|x| counted(x))
            .map(|x| (x.ts_ms(), to_f64(x.value_dec())))
            .collect();
        match opts.resample {
            Some(r) if points.len() < MIN_N => Err(format!(
                "a calibration window, {a}-{b} of series {} instrument {}, holds {} point(s) at \
                 step {} ms, fewer than {MIN_N}",
                s.canonical(),
                i.canonical(),
                points.len(),
                r.step_ms
            )),
            _ => Ok(points),
        }
    };
    let mut read = Vec::new();
    for &(r0, t0, t1) in windows {
        if !(r0 < t0 && t0 < t1) {
            return Err(format!(
                "a calibration window is REF-FROM-TO with REF < FROM < TO, not {r0}-{t0}-{t1}"
            ));
        }
        let mut pairs = Trial::new();
        for (s, i) in &streams {
            pairs.insert((*s, *i), (points(s, i, r0, t0)?, points(s, i, t0, t1)?));
        }
        read.push(pairs);
    }
    Ok(read)
}
