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
