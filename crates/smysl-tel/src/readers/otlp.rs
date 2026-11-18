//! `otlp-json/1` and `otlp/1` — an OTLP metrics export (RFC SMYSL-3 §6, §9.1, §20.1).
//!
//! **Two encodings, one reading.** An `ExportMetricsServiceRequest` arrives as OTLP/JSON
//! ([`read_json`], always built) or as protobuf ([`read_proto`], behind `reader-otlp`). Each
//! decodes into the same [`Raw`] data points, and [`points`] turns those into series samples, so
//! the two cannot disagree about a key or a value: the fixture records one export both ways and
//! requires the same digests.
//!
//! **Our own JSON reader** (§20.1). `opentelemetry-proto`'s serde support drops a spec-compliant
//! string `asInt`; here a 64-bit integer is read from a string or a number, and an integer is
//! never routed through a double. Field names are accepted in lowerCamelCase, as OTLP/JSON
//! writes them, and in the proto's snake_case, as the protobuf JSON mapping allows; enums as
//! integers, as OTLP/JSON requires, or by name. Unknown fields are ignored, as OTLP requires.
//!
//! **Values** (§5.3). `asInt`, counts and bucket counts are typed integers: `[n, 0]`. `asDouble`,
//! sums and quantile values are binary64: the decimal `Number::toString` would write. A JSON
//! token is parsed to the binary64 it denotes first, so `0.1` in JSON and `0.1` in protobuf are
//! one value. `NaN` and `±Infinity` are not values: such a sample is kept with flag 4 and
//! counted.
//!
//! **Series identity** (§6, scheme `otel`) is the JCS text, numbers in their `dec` spelling, of
//! `{resource, scope: {name, version}, name, kind, unit, temporality, monotonic, attributes}`,
//! attribute sets as objects (so sorted by key). `description` and bucket bounds are not in it.
//!
//! **One data point, several series.** The substrate holds scalars, so a point that is not one
//! number is split, and the key gains a `part` member:
//! - a histogram: `count`, `sum`, and `le:<bound>` per bucket, cumulative and ending in
//!   `le:+Inf` as Prometheus writes them, so the two sources' buckets read alike;
//! - a summary: `count`, `sum`, and `quantile:<q>`;
//! - an exponential histogram: `count` and `sum`. Its buckets are not series yet: their index is
//!   relative to a scale that can change between points, so a bucket index is not an identity.
//!   They are counted in [`Export::unsplit`].
//!
//! A gauge's and a sum's single number has no `part`. A part's `le:` bound is in the key while
//! the histogram's bounds are not its identity: a histogram whose layout changes keeps its
//! `count` and `sum` series and gains new bucket series, which is what a layout change is.

use smysl_core::Decimal;

use crate::jcs;
use crate::json::{self, Json};
use crate::readers::{shape, ReadError};

/// The reader id of the JSON encoding.
pub const ID_JSON: &str = "otlp-json/1";
/// The reader id of the protobuf encoding.
pub const ID: &str = "otlp/1";

/// `DataPointFlags.FLAG_NO_RECORDED_VALUE`.
pub const FLAG_NO_RECORDED_VALUE: u32 = 1;

/// A metric's data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Gauge,
    Sum,
    Histogram,
    ExponentialHistogram,
    Summary,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Gauge => "gauge",
            Kind::Sum => "sum",
            Kind::Histogram => "histogram",
            Kind::ExponentialHistogram => "exponential_histogram",
            Kind::Summary => "summary",
        }
    }
}

/// `AggregationTemporality`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Temporality {
    Unspecified,
    Delta,
    Cumulative,
}

impl Temporality {
    pub fn as_str(self) -> &'static str {
        match self {
            Temporality::Unspecified => "unspecified",
            Temporality::Delta => "delta",
            Temporality::Cumulative => "cumulative",
        }
    }

    fn of(n: i64) -> Temporality {
        match n {
            1 => Temporality::Delta,
            2 => Temporality::Cumulative,
            _ => Temporality::Unspecified,
        }
    }
}

/// An attribute value (`AnyValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum AnyValue {
    Empty,
    Str(String),
    Bool(bool),
    Int(i64),
    Double(f64),
    Bytes(Vec<u8>),
    Array(Vec<AnyValue>),
    KvList(Vec<(String, AnyValue)>),
}

/// A number as the source typed it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Num {
    Int(i64),
    UInt(u64),
    /// A cumulative bucket count can exceed `u64` only in theory; it is kept exactly.
    Wide(u128),
    Double(f64),
}

impl Num {
    /// The `dec` value (§5.3), or `None` for `NaN` and `±Inf`.
    pub fn dec(self) -> Option<Decimal> {
        match self {
            Num::Int(n) => Some(Decimal::from_i64(n)),
            Num::UInt(n) => Some(Decimal::from_u64(n)),
            Num::Wide(n) => Decimal::new(i128::try_from(n).ok()?, 0).ok(),
            Num::Double(x) => Decimal::from_f64(x).ok(),
        }
    }
}

/// One data point of one metric, as decoded, before it is split into series.
#[derive(Debug, Clone, PartialEq)]
pub struct Raw {
    pub resource: Vec<(String, AnyValue)>,
    pub scope_name: String,
    pub scope_version: String,
    pub name: String,
    pub unit: String,
    pub kind: Kind,
    pub temporality: Temporality,
    pub monotonic: bool,
    pub attributes: Vec<(String, AnyValue)>,
    pub start_ns: u64,
    pub time_ns: u64,
    pub flags: u32,
    /// `(part, number)`: one with no part for a gauge or sum, several otherwise.
    pub parts: Vec<(Option<String>, Num)>,
    /// Exponential-histogram buckets not split into series.
    pub unsplit: usize,
}

impl Raw {
    /// The source states the series is stale here.
    pub fn no_recorded_value(&self) -> bool {
        self.flags & FLAG_NO_RECORDED_VALUE != 0
    }

    /// A cumulative count that only goes up: a monotonic cumulative sum, a cumulative
    /// histogram's `count`, a summary's `count`. Only these reset.
    pub fn resets(&self) -> bool {
        match self.kind {
            Kind::Sum => self.monotonic && self.temporality == Temporality::Cumulative,
            Kind::Histogram | Kind::ExponentialHistogram => {
                self.temporality == Temporality::Cumulative
            }
            Kind::Summary => true,
            Kind::Gauge => false,
        }
    }

    /// The leaf class (§4.3) of this point's series.
    pub fn class(&self) -> &'static str {
        match self.kind {
            Kind::Sum if self.monotonic => "counter",
            Kind::Gauge | Kind::Sum => "gauge",
            Kind::Histogram | Kind::ExponentialHistogram | Kind::Summary => "histogram",
        }
    }

    /// The origin key of this point's resource (§7): `otel-sdk:<service.namespace>/<service.name>/<service.instance.id>`,
    /// the process whose SDK measured it. `None` without a string `service.name`: the resource
    /// does not say what measured it, and a guess would make two unrelated processes one origin.
    pub fn origin(&self) -> Option<String> {
        let get = |k: &str| {
            self.resource.iter().find_map(|(key, v)| match v {
                AnyValue::Str(s) if key == k => Some(s.as_str()),
                _ => None,
            })
        };
        let name = get("service.name").filter(|s| !s.is_empty())?;
        Some(format!(
            "otel-sdk:{}/{name}/{}",
            get("service.namespace").unwrap_or(""),
            get("service.instance.id").unwrap_or("")
        ))
    }

    /// The series key of `part` (§6).
    pub fn key(&self, part: Option<&str>) -> String {
        let s = |v: &str| Json::String(v.to_string());
        let mut members = vec![
            ("resource".to_string(), attrs(&self.resource)),
            (
                "scope".to_string(),
                Json::Object(vec![
                    ("name".to_string(), s(&self.scope_name)),
                    ("version".to_string(), s(&self.scope_version)),
                ]),
            ),
            ("name".to_string(), s(&self.name)),
            ("kind".to_string(), s(self.kind.as_str())),
            ("unit".to_string(), s(&self.unit)),
            ("temporality".to_string(), s(self.temporality.as_str())),
            ("monotonic".to_string(), Json::Bool(self.monotonic)),
            ("attributes".to_string(), attrs(&self.attributes)),
        ];
        if let Some(p) = part {
            members.push(("part".to_string(), s(p)));
        }
        jcs::canonical(&Json::Object(members)).0
    }
}

/// An attribute set as a JSON object. A later duplicate key wins, as in the OTel data model.
fn attrs(kvs: &[(String, AnyValue)]) -> Json {
    let mut out: Vec<(String, Json)> = Vec::with_capacity(kvs.len());
    for (k, v) in kvs {
        let v = any(v);
        match out.iter_mut().find(|(o, _)| o == k) {
            Some(slot) => slot.1 = v,
            None => out.push((k.clone(), v)),
        }
    }
    Json::Object(out)
}

fn any(v: &AnyValue) -> Json {
    match v {
        AnyValue::Empty => Json::Null,
        AnyValue::Str(s) => Json::String(s.clone()),
        AnyValue::Bool(b) => Json::Bool(*b),
        AnyValue::Int(n) => Json::Number(n.to_string()),
        AnyValue::Double(x) => match Decimal::from_f64(*x) {
            Ok(d) => Json::Number(d.to_string()),
            Err(_) if x.is_nan() => Json::String("NaN".into()),
            Err(_) if *x > 0.0 => Json::String("Infinity".into()),
            Err(_) => Json::String("-Infinity".into()),
        },
        // Bytes are not text, so they are not written as a string: `{"bytes": hex}`.
        AnyValue::Bytes(b) => Json::Object(vec![(
            "bytes".to_string(),
            Json::String(b.iter().map(|x| format!("{x:02x}")).collect()),
        )]),
        AnyValue::Array(items) => Json::Array(items.iter().map(any).collect()),
        AnyValue::KvList(kvs) => attrs(kvs),
    }
}

/// One series sample of an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    /// The series key (§6, scheme `otel`).
    pub key: String,
    /// The data point it came from, an index into the export's points: a data point's parts
    /// reset and go stale together.
    pub group: usize,
    /// The part this series is, `None` for a gauge's or sum's number.
    pub part: Option<String>,
    /// The series a reset or stale unit is written on: the number itself, or `count`.
    pub headline: bool,
    pub class: &'static str,
    pub start_ns: u64,
    pub time_ns: u64,
    /// `None`: `NaN` or `±Inf`, kept with flag 4.
    pub value: Option<Decimal>,
    /// `FLAG_NO_RECORDED_VALUE`.
    pub stale: bool,
    /// A count that resets (see [`Raw::resets`]); only meaningful on the headline.
    pub resets: bool,
    /// Its resource's origin key ([`Raw::origin`]).
    pub origin: Option<String>,
}

/// What a read produced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Export {
    pub raw: Vec<Raw>,
}

impl Export {
    /// Every series sample, data point by data point.
    pub fn points(&self) -> Vec<Point> {
        points(&self.raw)
    }

    /// Samples whose value is `NaN` or `±Inf`.
    pub fn not_values(&self) -> usize {
        self.raw
            .iter()
            .filter(|r| !r.no_recorded_value())
            .flat_map(|r| &r.parts)
            .filter(|(_, n)| n.dec().is_none())
            .count()
    }

    /// Exponential-histogram buckets not split into series.
    pub fn unsplit(&self) -> usize {
        self.raw.iter().map(|r| r.unsplit).sum()
    }
}

/// Split data points into series samples.
pub fn points(raw: &[Raw]) -> Vec<Point> {
    let mut out = Vec::new();
    for (group, r) in raw.iter().enumerate() {
        let stale = r.no_recorded_value();
        for (part, n) in &r.parts {
            out.push(Point {
                key: r.key(part.as_deref()),
                group,
                headline: matches!(part.as_deref(), None | Some("count")),
                part: part.clone(),
                class: r.class(),
                start_ns: r.start_ns,
                time_ns: r.time_ns,
                value: if stale { None } else { n.dec() },
                stale,
                resets: r.resets(),
                origin: r.origin(),
            });
        }
    }
    out
}

/// A histogram's parts: `count`, `sum` if present, and the buckets as cumulative `le:` counts.
pub(crate) fn histogram_parts(
    count: u64,
    sum: Option<f64>,
    buckets: &[u64],
    bounds: &[f64],
) -> Result<Vec<(Option<String>, Num)>, String> {
    let mut parts = vec![(Some("count".to_string()), Num::UInt(count))];
    if let Some(s) = sum {
        parts.push((Some("sum".to_string()), Num::Double(s)));
    }
    if buckets.is_empty() {
        return Ok(parts);
    }
    if buckets.len() != bounds.len() + 1 {
        return Err(format!(
            "a histogram point has {} bucket counts for {} bounds; OTLP requires one more count \
             than bounds",
            buckets.len(),
            bounds.len()
        ));
    }
    let mut cum: u128 = 0;
    for (i, c) in buckets.iter().enumerate() {
        cum += u128::from(*c);
        let le = match bounds.get(i) {
            Some(b) => match Decimal::from_f64(*b) {
                Ok(d) => d.to_string(),
                Err(_) => return Err("a histogram bound is not a finite number".into()),
            },
            None => "+Inf".to_string(),
        };
        parts.push((Some(format!("le:{le}")), Num::Wide(cum)));
    }
    Ok(parts)
}

/// A summary's parts: `count`, `sum`, and `quantile:<q>` per quantile.
pub(crate) fn summary_parts(
    count: u64,
    sum: f64,
    quantiles: &[(f64, f64)],
) -> Result<Vec<(Option<String>, Num)>, String> {
    let mut parts = vec![
        (Some("count".to_string()), Num::UInt(count)),
        (Some("sum".to_string()), Num::Double(sum)),
    ];
    for (q, v) in quantiles {
        let q = Decimal::from_f64(*q)
            .map_err(|_| "a summary quantile is not a finite number".to_string())?;
        parts.push((Some(format!("quantile:{q}")), Num::Double(*v)));
    }
    Ok(parts)
}

// ---------------------------------------------------------------------------------------------
// OTLP/JSON
// ---------------------------------------------------------------------------------------------

/// Read an OTLP/JSON `ExportMetricsServiceRequest`.
pub fn read_json(text: &str) -> Result<Export, ReadError> {
    let doc = json::parse(text)?;
    let mut raw = Vec::new();
    for rm in array(&doc, "resourceMetrics")? {
        let resource = match field(rm, "resource") {
            Some(r) => kvs(r, "attributes")?,
            None => Vec::new(),
        };
        for sm in array(rm, "scopeMetrics")? {
            let (scope_name, scope_version) = match field(sm, "scope") {
                Some(s) => (string(s, "name")?, string(s, "version")?),
                None => (String::new(), String::new()),
            };
            for m in array(sm, "metrics")? {
                let base = Raw {
                    resource: resource.clone(),
                    scope_name: scope_name.clone(),
                    scope_version: scope_version.clone(),
                    name: string(m, "name")?,
                    unit: string(m, "unit")?,
                    kind: Kind::Gauge,
                    temporality: Temporality::Unspecified,
                    monotonic: false,
                    attributes: Vec::new(),
                    start_ns: 0,
                    time_ns: 0,
                    flags: 0,
                    parts: Vec::new(),
                    unsplit: 0,
                };
                metric(m, &base, &mut raw)?;
            }
        }
    }
    Ok(Export { raw })
}

fn metric(m: &Json, base: &Raw, out: &mut Vec<Raw>) -> Result<(), ReadError> {
    let data = [
        ("gauge", Kind::Gauge),
        ("sum", Kind::Sum),
        ("histogram", Kind::Histogram),
        ("exponentialHistogram", Kind::ExponentialHistogram),
        ("summary", Kind::Summary),
    ];
    let Some((d, kind)) = data.iter().find_map(|(n, k)| field(m, n).map(|d| (d, *k))) else {
        // A metric with no data (or a kind this reader does not know) has no points.
        return Ok(());
    };
    let temporality = Temporality::of(enum_value(d, "aggregationTemporality", TEMPORALITIES)?);
    let monotonic = boolean(d, "isMonotonic")?;
    for p in array(d, "dataPoints")? {
        let mut r = Raw {
            kind,
            temporality,
            monotonic,
            attributes: kvs(p, "attributes")?,
            start_ns: u64_of(p, "startTimeUnixNano")?.unwrap_or(0),
            time_ns: u64_of(p, "timeUnixNano")?.unwrap_or(0),
            flags: u32::try_from(u64_of(p, "flags")?.unwrap_or(0))
                .map_err(|_| shape("a data point's flags exceed 32 bits"))?,
            ..base.clone()
        };
        r.parts = match kind {
            Kind::Gauge | Kind::Sum => {
                let n = if let Some(v) = field(p, "asInt") {
                    Num::Int(i64_json(v, "asInt")?)
                } else if let Some(v) = field(p, "asDouble") {
                    Num::Double(f64_json(v, "asDouble")?)
                } else if r.no_recorded_value() {
                    Num::Int(0)
                } else {
                    return Err(shape(format!(
                        "a data point of {:?} has neither asInt nor asDouble",
                        base.name
                    )));
                };
                vec![(None, n)]
            }
            Kind::Histogram => {
                let buckets = array(p, "bucketCounts")?
                    .iter()
                    .map(|v| u64_json(v, "bucketCounts"))
                    .collect::<Result<Vec<_>, _>>()?;
                let bounds = array(p, "explicitBounds")?
                    .iter()
                    .map(|v| f64_json(v, "explicitBounds"))
                    .collect::<Result<Vec<_>, _>>()?;
                histogram_parts(
                    u64_of(p, "count")?.unwrap_or(0),
                    f64_of(p, "sum")?,
                    &buckets,
                    &bounds,
                )
                .map_err(shape)?
            }
            Kind::ExponentialHistogram => {
                let mut parts = vec![(
                    Some("count".to_string()),
                    Num::UInt(u64_of(p, "count")?.unwrap_or(0)),
                )];
                if let Some(s) = f64_of(p, "sum")? {
                    parts.push((Some("sum".to_string()), Num::Double(s)));
                }
                r.unsplit = ["positive", "negative"]
                    .iter()
                    .filter_map(|b| field(p, b))
                    .map(|b| array(b, "bucketCounts").map(<[Json]>::len))
                    .sum::<Result<usize, _>>()?
                    // The zero bucket: protobuf cannot tell it absent from 0, so it always counts.
                    + 1;
                parts
            }
            Kind::Summary => {
                let qs = array(p, "quantileValues")?
                    .iter()
                    .map(|q| {
                        Ok((
                            f64_of(q, "quantile")?.unwrap_or(0.0),
                            f64_of(q, "value")?.unwrap_or(0.0),
                        ))
                    })
                    .collect::<Result<Vec<_>, ReadError>>()?;
                summary_parts(
                    u64_of(p, "count")?.unwrap_or(0),
                    f64_of(p, "sum")?.unwrap_or(0.0),
                    &qs,
                )
                .map_err(shape)?
            }
        };
        out.push(r);
    }
    Ok(())
}

const TEMPORALITIES: &[&str] = &[
    "AGGREGATION_TEMPORALITY_UNSPECIFIED",
    "AGGREGATION_TEMPORALITY_DELTA",
    "AGGREGATION_TEMPORALITY_CUMULATIVE",
];

/// `lowerCamelCase` as OTLP/JSON writes it, or the proto's `snake_case`.
fn field<'a>(v: &'a Json, camel: &str) -> Option<&'a Json> {
    v.get(camel).or_else(|| {
        let snake: String = camel
            .chars()
            .flat_map(|c| {
                let lower = c.to_ascii_lowercase();
                (c.is_ascii_uppercase())
                    .then_some('_')
                    .into_iter()
                    .chain(std::iter::once(lower))
            })
            .collect();
        v.get(&snake)
    })
}

/// A repeated field: absent is empty; anything but an array is refused.
fn array<'a>(v: &'a Json, name: &str) -> Result<&'a [Json], ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(&[]),
        Some(Json::Array(items)) => Ok(items),
        Some(_) => Err(shape(format!("{name} is not an array"))),
    }
}

fn string(v: &Json, name: &str) -> Result<String, ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(String::new()),
        Some(Json::String(s)) => Ok(s.clone()),
        Some(_) => Err(shape(format!("{name} is not a string"))),
    }
}

fn boolean(v: &Json, name: &str) -> Result<bool, ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(false),
        Some(Json::Bool(b)) => Ok(*b),
        Some(_) => Err(shape(format!("{name} is not a boolean"))),
    }
}

/// An enum, as its number or its name.
fn enum_value(v: &Json, name: &str, names: &[&str]) -> Result<i64, ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(0),
        Some(Json::String(s)) => names
            .iter()
            .position(|n| n == s)
            .map(|i| i as i64)
            .ok_or_else(|| shape(format!("{name}: unknown value {s:?}"))),
        Some(j) => i64_json(j, name),
    }
}

/// An integer token: digits only, with `-` if `signed`. Never through a double.
fn integer_token<'a>(v: &'a Json, name: &str) -> Result<&'a str, ReadError> {
    let t = match v {
        Json::Number(t) | Json::String(t) => t.as_str(),
        _ => return Err(shape(format!("{name} is not an integer"))),
    };
    let digits = t.strip_prefix('-').unwrap_or(t);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(shape(format!("{name}: {t:?} is not an integer")));
    }
    Ok(t)
}

fn i64_json(v: &Json, name: &str) -> Result<i64, ReadError> {
    integer_token(v, name)?
        .parse()
        .map_err(|_| shape(format!("{name} is outside int64")))
}

fn u64_json(v: &Json, name: &str) -> Result<u64, ReadError> {
    integer_token(v, name)?
        .parse()
        .map_err(|_| shape(format!("{name} is outside uint64")))
}

fn u64_of(v: &Json, name: &str) -> Result<Option<u64>, ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(None),
        Some(j) => u64_json(j, name).map(Some),
    }
}

/// A double: a number token, or one of the proto3 JSON spellings `NaN`, `Infinity`,
/// `-Infinity`, or a numeric string.
fn f64_json(v: &Json, name: &str) -> Result<f64, ReadError> {
    let t = match v {
        Json::Number(t) => t.as_str(),
        Json::String(s) => match s.as_str() {
            "NaN" => return Ok(f64::NAN),
            "Infinity" => return Ok(f64::INFINITY),
            "-Infinity" => return Ok(f64::NEG_INFINITY),
            s => s,
        },
        _ => return Err(shape(format!("{name} is not a number"))),
    };
    // A string must still be a JSON number; `str::parse` would take `inf` and `1_0`.
    if !matches!(json::parse(t), Ok(Json::Number(_))) {
        return Err(shape(format!("{name}: {t:?} is not a number")));
    }
    t.parse()
        .map_err(|_| shape(format!("{name}: {t:?} is not a number")))
}

fn f64_of(v: &Json, name: &str) -> Result<Option<f64>, ReadError> {
    match field(v, name) {
        None | Some(Json::Null) => Ok(None),
        Some(j) => f64_json(j, name).map(Some),
    }
}

fn kvs(v: &Json, name: &str) -> Result<Vec<(String, AnyValue)>, ReadError> {
    array(v, name)?
        .iter()
        .map(|kv| Ok((string(kv, "key")?, any_json(field(kv, "value"), 0)?)))
        .collect()
}

fn any_json(v: Option<&Json>, depth: usize) -> Result<AnyValue, ReadError> {
    if depth > json::MAX_DEPTH {
        return Err(shape("an attribute value nests too deeply"));
    }
    let Some(v) = v else {
        return Ok(AnyValue::Empty);
    };
    Ok(if let Some(s) = field(v, "stringValue") {
        AnyValue::Str(s.as_str().ok_or_else(|| shape("stringValue"))?.to_string())
    } else if let Some(b) = field(v, "boolValue") {
        match b {
            Json::Bool(b) => AnyValue::Bool(*b),
            _ => return Err(shape("boolValue is not a boolean")),
        }
    } else if let Some(n) = field(v, "intValue") {
        AnyValue::Int(i64_json(n, "intValue")?)
    } else if let Some(x) = field(v, "doubleValue") {
        AnyValue::Double(f64_json(x, "doubleValue")?)
    } else if let Some(a) = field(v, "arrayValue") {
        AnyValue::Array(
            array(a, "values")?
                .iter()
                .map(|i| any_json(Some(i), depth + 1))
                .collect::<Result<_, _>>()?,
        )
    } else if let Some(l) = field(v, "kvlistValue") {
        AnyValue::KvList(
            array(l, "values")?
                .iter()
                .map(|kv| Ok((string(kv, "key")?, any_json(field(kv, "value"), depth + 1)?)))
                .collect::<Result<_, ReadError>>()?,
        )
    } else if let Some(b) = field(v, "bytesValue") {
        AnyValue::Bytes(base64(b.as_str().ok_or_else(|| shape("bytesValue"))?)?)
    } else {
        AnyValue::Empty
    })
}

/// Standard base64, as the protobuf JSON mapping writes bytes; the URL-safe alphabet is
/// accepted too, as the mapping requires of a parser. Padding is optional.
fn base64(s: &str) -> Result<Vec<u8>, ReadError> {
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let body = s.trim_end_matches('=');
    if s.len() - body.len() > 2 || body.len() % 4 == 1 {
        return Err(shape("bytesValue is not base64"));
    }
    let mut out = Vec::with_capacity(body.len() * 3 / 4);
    for chunk in body.as_bytes().chunks(4) {
        let mut acc = 0u32;
        for (i, c) in chunk.iter().enumerate() {
            acc |= val(*c).ok_or_else(|| shape("bytesValue is not base64"))? << (18 - 6 * i);
        }
        let n = chunk.len() * 6 / 8;
        out.extend_from_slice(&acc.to_be_bytes()[1..=n]);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// OTLP protobuf
// ---------------------------------------------------------------------------------------------

/// Read a protobuf `ExportMetricsServiceRequest`.
#[cfg(feature = "reader-otlp")]
pub fn read_proto(bytes: &[u8]) -> Result<Export, ReadError> {
    crate::readers::otlp_pb::read(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RES: &str = r#"{"attributes":[{"key":"service.name","value":{"stringValue":"cart"}}]}"#;

    fn export(metric: &str) -> String {
        format!(
            r#"{{"resourceMetrics":[{{"resource":{RES},"scopeMetrics":[{{"scope":{{"name":"s","version":"1"}},"metrics":[{metric}]}}]}}]}}"#
        )
    }

    fn one(metric: &str) -> Vec<Point> {
        read_json(&export(metric)).unwrap().points()
    }

    #[test]
    fn as_int_is_read_from_a_string_or_a_number_and_never_through_a_double() {
        let big = "9007199254740993"; // 2^53 + 1: a double would make it ...992.
        for v in [format!("\"{big}\""), big.to_string()] {
            let p = one(&format!(
                r#"{{"name":"n","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{{"timeUnixNano":"5","asInt":{v}}}]}}}}"#
            ));
            assert_eq!(p[0].value.unwrap().to_string(), big);
            assert!(p[0].resets && p[0].headline && p[0].class == "counter");
        }
    }

    #[test]
    fn as_double_is_the_binary64_the_token_denotes() {
        let p = one(
            r#"{"name":"g","gauge":{"dataPoints":[{"timeUnixNano":"1","asDouble":0.1},{"timeUnixNano":"2","asDouble":1e0},{"timeUnixNano":"3","asDouble":"NaN"},{"timeUnixNano":"4","asDouble":0.30000000000000004}]}}"#,
        );
        let v: Vec<Option<String>> = p.iter().map(|p| p.value.map(|d| d.to_string())).collect();
        assert_eq!(
            v,
            [
                Some("0.1".into()),
                Some("1".into()),
                None,
                Some("0.30000000000000004".into())
            ]
        );
        assert_eq!(p[0].key, p[1].key, "one series");
        assert!(!p[0].resets);
    }

    #[test]
    fn snake_case_and_enum_names_are_accepted() {
        let camel = read_json(&export(
            r#"{"name":"n","sum":{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{"timeUnixNano":"5","startTimeUnixNano":"1","asInt":"3"}]}}"#,
        ))
        .unwrap();
        let snake = read_json(
            &export(
                r#"{"name":"n","sum":{"aggregation_temporality":"AGGREGATION_TEMPORALITY_CUMULATIVE","is_monotonic":true,"data_points":[{"time_unix_nano":"5","start_time_unix_nano":"1","as_int":"3"}]}}"#,
            )
            .replace("resourceMetrics", "resource_metrics")
            .replace("scopeMetrics", "scope_metrics"),
        )
        .unwrap();
        assert_eq!(camel, snake);
    }

    #[test]
    fn the_key_is_jcs_of_the_identity_and_attributes_are_sorted() {
        let a = one(
            r#"{"name":"n","unit":"1","description":"one","gauge":{"dataPoints":[{"timeUnixNano":"1","asInt":"1","attributes":[{"key":"b","value":{"intValue":"2"}},{"key":"a","value":{"doubleValue":2.0}}]}]}}"#,
        );
        let b = one(
            r#"{"name":"n","unit":"1","description":"two","gauge":{"dataPoints":[{"timeUnixNano":"1","asInt":"1","attributes":[{"key":"a","value":{"intValue":2}},{"key":"b","value":{"doubleValue":2}}]}]}}"#,
        );
        assert_eq!(
            a[0].key, b[0].key,
            "description is not identity; 2 and 2.0 are one value"
        );
        assert_eq!(
            a[0].key,
            r#"{"attributes":{"a":2,"b":2},"kind":"gauge","monotonic":false,"name":"n","resource":{"service.name":"cart"},"scope":{"name":"s","version":"1"},"temporality":"unspecified","unit":"1"}"#
        );
    }

    #[test]
    fn a_histogram_is_count_sum_and_cumulative_buckets() {
        let p = one(
            r#"{"name":"h","histogram":{"aggregationTemporality":2,"dataPoints":[{"timeUnixNano":"1","count":"4","sum":0.9,"bucketCounts":["1","2","1"],"explicitBounds":[0.1,0.5]}]}}"#,
        );
        let parts: Vec<(String, String)> = p
            .iter()
            .map(|p| (p.part.clone().unwrap(), p.value.unwrap().to_string()))
            .collect();
        let want = [
            ("count", "4"),
            ("sum", "0.9"),
            ("le:0.1", "1"),
            ("le:0.5", "3"),
            ("le:+Inf", "4"),
        ];
        assert_eq!(
            parts,
            want.map(|(a, b)| (a.to_string(), b.to_string())).to_vec()
        );
        assert!(p[0].headline && !p[1].headline && p.iter().all(|p| p.group == 0));
        let bad = read_json(&export(
            r#"{"name":"h","histogram":{"dataPoints":[{"count":"1","bucketCounts":["1"],"explicitBounds":[0.1]}]}}"#,
        ));
        assert!(bad.is_err());
    }

    #[test]
    fn a_summary_and_an_exponential_histogram() {
        let s = one(
            r#"{"name":"s","summary":{"dataPoints":[{"timeUnixNano":"1","count":"3","sum":0.03,"quantileValues":[{"quantile":0.5,"value":0.01},{"quantile":0.99,"value":0.015}]}]}}"#,
        );
        let parts: Vec<String> = s.iter().map(|p| p.part.clone().unwrap()).collect();
        assert_eq!(parts, ["count", "sum", "quantile:0.5", "quantile:0.99"]);
        assert!(s[0].resets);
        let e = read_json(&export(
            r#"{"name":"e","exponentialHistogram":{"aggregationTemporality":1,"dataPoints":[{"timeUnixNano":"1","count":"3","sum":1.5,"scale":2,"zeroCount":"1","positive":{"offset":1,"bucketCounts":["1","1"]}}]}}"#,
        ))
        .unwrap();
        assert_eq!(e.points().len(), 2);
        assert_eq!(e.unsplit(), 3);
        assert!(!e.points()[0].resets, "delta does not reset");
    }

    #[test]
    fn no_recorded_value_is_stale_and_needs_no_value() {
        let p = one(r#"{"name":"n","gauge":{"dataPoints":[{"timeUnixNano":"1","flags":1}]}}"#);
        assert!(p[0].stale && p[0].value.is_none());
    }

    #[test]
    fn attribute_values_of_every_kind() {
        let p = one(
            r#"{"name":"n","gauge":{"dataPoints":[{"timeUnixNano":"1","asInt":"1","attributes":[
                {"key":"s","value":{"stringValue":"x"}},{"key":"t","value":{"boolValue":true}},
                {"key":"b","value":{"bytesValue":"AQL/"}},{"key":"e","value":{}},
                {"key":"a","value":{"arrayValue":{"values":[{"intValue":"1"},{"doubleValue":"NaN"}]}}},
                {"key":"k","value":{"kvlistValue":{"values":[{"key":"z","value":{"stringValue":"1"}}]}}}]}]}}"#,
        );
        assert!(
            p[0].key.contains(
                r#""attributes":{"a":[1,"NaN"],"b":{"bytes":"0102ff"},"e":null,"k":{"z":"1"},"s":"x","t":true}"#
            ),
            "{}",
            p[0].key
        );
    }

    #[test]
    fn malformed_exports_are_refused() {
        for bad in [
            r#"{"resourceMetrics":{}}"#,
            r#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"n","gauge":{"dataPoints":[{"asInt":"1.5"}]}}]}]}]}"#,
            r#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"n","gauge":{"dataPoints":[{"timeUnixNano":"-1","asInt":"1"}]}}]}]}]}"#,
            r#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"n","gauge":{"dataPoints":[{"asDouble":"inf"}]}}]}]}]}"#,
            r#"{"resourceMetrics":[{"scopeMetrics":[{"metrics":[{"name":"n","gauge":{"dataPoints":[{}]}}]}]}]}"#,
            r#"{"resourceMetrics":[{"resource":{"attributes":[{"key":"b","value":{"bytesValue":"A"}}]}}]}"#,
        ] {
            assert!(read_json(bad).is_err(), "{bad}");
        }
    }
}
