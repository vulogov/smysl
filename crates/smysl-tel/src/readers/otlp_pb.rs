//! `otlp/1`: the protobuf `ExportMetricsServiceRequest`, decoded by `prost` (RFC SMYSL-3 §20.1).
//!
//! **The messages are declared here, not taken from `opentelemetry-proto`.** That crate's
//! message types cannot be had without its non-optional `opentelemetry` and `opentelemetry_sdk`
//! dependencies, and an SDK is not what a pure reader of bytes should link. What the reader needs
//! is the metrics subset of `opentelemetry/proto/{collector/metrics,metrics,common,resource}/v1`,
//! with the upstream field numbers and types, and `prost`'s derive. Fields this reader does not
//! use (exemplars, `schema_url`, `description`, dropped counts) are left undeclared: protobuf
//! skips unknown fields, so a full export decodes the same.
//!
//! `fixtures/tel/otlp/metrics.proto` is the same subset as `.proto` text; `protoc --encode`
//! writes the fixture's binary exports from it, so the bytes this reads were not produced by
//! this code.

use prost::Message;

use crate::readers::otlp::{self, AnyValue, Export, Kind, Num, Raw, Temporality};
use crate::readers::{shape, ReadError};

#[derive(Clone, PartialEq, Message)]
pub struct ExportMetricsServiceRequest {
    #[prost(message, repeated, tag = "1")]
    pub resource_metrics: Vec<ResourceMetrics>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ResourceMetrics {
    #[prost(message, optional, tag = "1")]
    pub resource: Option<Resource>,
    #[prost(message, repeated, tag = "2")]
    pub scope_metrics: Vec<ScopeMetrics>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Resource {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct ScopeMetrics {
    #[prost(message, optional, tag = "1")]
    pub scope: Option<InstrumentationScope>,
    #[prost(message, repeated, tag = "2")]
    pub metrics: Vec<Metric>,
}

#[derive(Clone, PartialEq, Message)]
pub struct InstrumentationScope {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "2")]
    pub version: String,
}

#[derive(Clone, PartialEq, Message)]
pub struct Metric {
    #[prost(string, tag = "1")]
    pub name: String,
    #[prost(string, tag = "3")]
    pub unit: String,
    #[prost(oneof = "Data", tags = "5, 7, 9, 10, 11")]
    pub data: Option<Data>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum Data {
    #[prost(message, tag = "5")]
    Gauge(Gauge),
    #[prost(message, tag = "7")]
    Sum(Sum),
    #[prost(message, tag = "9")]
    Histogram(Histogram),
    #[prost(message, tag = "10")]
    ExponentialHistogram(ExponentialHistogram),
    #[prost(message, tag = "11")]
    Summary(Summary),
}

#[derive(Clone, PartialEq, Message)]
pub struct Gauge {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
}

#[derive(Clone, PartialEq, Message)]
pub struct Sum {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<NumberDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
    #[prost(bool, tag = "3")]
    pub is_monotonic: bool,
}

#[derive(Clone, PartialEq, Message)]
pub struct Histogram {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<HistogramDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExponentialHistogram {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<ExponentialHistogramDataPoint>,
    #[prost(int32, tag = "2")]
    pub aggregation_temporality: i32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Summary {
    #[prost(message, repeated, tag = "1")]
    pub data_points: Vec<SummaryDataPoint>,
}

#[derive(Clone, PartialEq, Message)]
pub struct NumberDataPoint {
    #[prost(message, repeated, tag = "7")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(oneof = "NumberValue", tags = "4, 6")]
    pub value: Option<NumberValue>,
    #[prost(uint32, tag = "8")]
    pub flags: u32,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum NumberValue {
    #[prost(double, tag = "4")]
    AsDouble(f64),
    #[prost(sfixed64, tag = "6")]
    AsInt(i64),
}

#[derive(Clone, PartialEq, Message)]
pub struct HistogramDataPoint {
    #[prost(message, repeated, tag = "9")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, optional, tag = "5")]
    pub sum: Option<f64>,
    #[prost(fixed64, repeated, tag = "6")]
    pub bucket_counts: Vec<u64>,
    #[prost(double, repeated, tag = "7")]
    pub explicit_bounds: Vec<f64>,
    #[prost(uint32, tag = "10")]
    pub flags: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ExponentialHistogramDataPoint {
    #[prost(message, repeated, tag = "1")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, optional, tag = "5")]
    pub sum: Option<f64>,
    #[prost(fixed64, tag = "7")]
    pub zero_count: u64,
    #[prost(message, optional, tag = "8")]
    pub positive: Option<Buckets>,
    #[prost(message, optional, tag = "9")]
    pub negative: Option<Buckets>,
    #[prost(uint32, tag = "10")]
    pub flags: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct Buckets {
    #[prost(uint64, repeated, tag = "2")]
    pub bucket_counts: Vec<u64>,
}

#[derive(Clone, PartialEq, Message)]
pub struct SummaryDataPoint {
    #[prost(message, repeated, tag = "7")]
    pub attributes: Vec<KeyValue>,
    #[prost(fixed64, tag = "2")]
    pub start_time_unix_nano: u64,
    #[prost(fixed64, tag = "3")]
    pub time_unix_nano: u64,
    #[prost(fixed64, tag = "4")]
    pub count: u64,
    #[prost(double, tag = "5")]
    pub sum: f64,
    #[prost(message, repeated, tag = "6")]
    pub quantile_values: Vec<ValueAtQuantile>,
    #[prost(uint32, tag = "8")]
    pub flags: u32,
}

#[derive(Clone, PartialEq, Message)]
pub struct ValueAtQuantile {
    #[prost(double, tag = "1")]
    pub quantile: f64,
    #[prost(double, tag = "2")]
    pub value: f64,
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyValue {
    #[prost(string, tag = "1")]
    pub key: String,
    #[prost(message, optional, tag = "2")]
    pub value: Option<PbAnyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct PbAnyValue {
    #[prost(oneof = "AnyKind", tags = "1, 2, 3, 4, 5, 6, 7")]
    pub value: Option<AnyKind>,
}

#[derive(Clone, PartialEq, prost::Oneof)]
pub enum AnyKind {
    #[prost(string, tag = "1")]
    Str(String),
    #[prost(bool, tag = "2")]
    Bool(bool),
    #[prost(int64, tag = "3")]
    Int(i64),
    #[prost(double, tag = "4")]
    Double(f64),
    #[prost(message, tag = "5")]
    Array(ArrayValue),
    #[prost(message, tag = "6")]
    KvList(KeyValueList),
    #[prost(bytes = "vec", tag = "7")]
    Bytes(Vec<u8>),
}

#[derive(Clone, PartialEq, Message)]
pub struct ArrayValue {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<PbAnyValue>,
}

#[derive(Clone, PartialEq, Message)]
pub struct KeyValueList {
    #[prost(message, repeated, tag = "1")]
    pub values: Vec<KeyValue>,
}

/// Decode `bytes` into the data points [`otlp::points`] reads.
pub fn read(bytes: &[u8]) -> Result<Export, ReadError> {
    let req = ExportMetricsServiceRequest::decode(bytes)
        .map_err(|e| shape(format!("not an OTLP ExportMetricsServiceRequest: {e}")))?;
    let mut raw = Vec::new();
    for rm in &req.resource_metrics {
        let resource = rm
            .resource
            .as_ref()
            .map(|r| kvs(&r.attributes))
            .unwrap_or_default();
        for sm in &rm.scope_metrics {
            let (scope_name, scope_version) = sm
                .scope
                .as_ref()
                .map(|s| (s.name.clone(), s.version.clone()))
                .unwrap_or_default();
            for m in &sm.metrics {
                let Some(data) = &m.data else { continue };
                let base = Raw {
                    resource: resource.clone(),
                    scope_name: scope_name.clone(),
                    scope_version: scope_version.clone(),
                    name: m.name.clone(),
                    unit: m.unit.clone(),
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
                metric(data, &base, &mut raw)?;
            }
        }
    }
    Ok(Export { raw })
}

fn temporality(n: i32) -> Temporality {
    match n {
        1 => Temporality::Delta,
        2 => Temporality::Cumulative,
        _ => Temporality::Unspecified,
    }
}

fn number(p: &NumberDataPoint) -> Result<Num, ReadError> {
    match p.value {
        Some(NumberValue::AsInt(n)) => Ok(Num::Int(n)),
        Some(NumberValue::AsDouble(x)) => Ok(Num::Double(x)),
        None if p.flags & otlp::FLAG_NO_RECORDED_VALUE != 0 => Ok(Num::Int(0)),
        None => Err(shape(
            "a number data point has neither as_int nor as_double",
        )),
    }
}

fn metric(data: &Data, base: &Raw, out: &mut Vec<Raw>) -> Result<(), ReadError> {
    let point = |kind, temporality, monotonic, attrs: &[KeyValue], start, time, flags| Raw {
        kind,
        temporality,
        monotonic,
        attributes: kvs(attrs),
        start_ns: start,
        time_ns: time,
        flags,
        ..base.clone()
    };
    match data {
        Data::Gauge(g) => {
            for p in &g.data_points {
                let mut r = point(
                    Kind::Gauge,
                    Temporality::Unspecified,
                    false,
                    &p.attributes,
                    p.start_time_unix_nano,
                    p.time_unix_nano,
                    p.flags,
                );
                r.parts = vec![(None, number(p)?)];
                out.push(r);
            }
        }
        Data::Sum(s) => {
            for p in &s.data_points {
                let mut r = point(
                    Kind::Sum,
                    temporality(s.aggregation_temporality),
                    s.is_monotonic,
                    &p.attributes,
                    p.start_time_unix_nano,
                    p.time_unix_nano,
                    p.flags,
                );
                r.parts = vec![(None, number(p)?)];
                out.push(r);
            }
        }
        Data::Histogram(h) => {
            for p in &h.data_points {
                let mut r = point(
                    Kind::Histogram,
                    temporality(h.aggregation_temporality),
                    false,
                    &p.attributes,
                    p.start_time_unix_nano,
                    p.time_unix_nano,
                    p.flags,
                );
                r.parts =
                    otlp::histogram_parts(p.count, p.sum, &p.bucket_counts, &p.explicit_bounds)
                        .map_err(shape)?;
                out.push(r);
            }
        }
        Data::ExponentialHistogram(h) => {
            for p in &h.data_points {
                let mut r = point(
                    Kind::ExponentialHistogram,
                    temporality(h.aggregation_temporality),
                    false,
                    &p.attributes,
                    p.start_time_unix_nano,
                    p.time_unix_nano,
                    p.flags,
                );
                r.parts = vec![(Some("count".to_string()), Num::UInt(p.count))];
                if let Some(s) = p.sum {
                    r.parts.push((Some("sum".to_string()), Num::Double(s)));
                }
                // protobuf cannot tell an absent zero_count from 0, so it is always counted.
                r.unsplit = [&p.positive, &p.negative]
                    .iter()
                    .filter_map(|b| b.as_ref())
                    .map(|b| b.bucket_counts.len())
                    .sum::<usize>()
                    + 1;
                out.push(r);
            }
        }
        Data::Summary(s) => {
            for p in &s.data_points {
                let mut r = point(
                    Kind::Summary,
                    Temporality::Unspecified,
                    false,
                    &p.attributes,
                    p.start_time_unix_nano,
                    p.time_unix_nano,
                    p.flags,
                );
                let qs: Vec<(f64, f64)> = p
                    .quantile_values
                    .iter()
                    .map(|q| (q.quantile, q.value))
                    .collect();
                r.parts = otlp::summary_parts(p.count, p.sum, &qs).map_err(shape)?;
                out.push(r);
            }
        }
    }
    Ok(())
}

pub(crate) fn kvs(kvs: &[KeyValue]) -> Vec<(String, AnyValue)> {
    kvs.iter()
        .map(|kv| (kv.key.clone(), any(kv.value.as_ref())))
        .collect()
}

fn any(v: Option<&PbAnyValue>) -> AnyValue {
    match v.and_then(|v| v.value.as_ref()) {
        None => AnyValue::Empty,
        Some(AnyKind::Str(s)) => AnyValue::Str(s.clone()),
        Some(AnyKind::Bool(b)) => AnyValue::Bool(*b),
        Some(AnyKind::Int(n)) => AnyValue::Int(*n),
        Some(AnyKind::Double(x)) => AnyValue::Double(*x),
        Some(AnyKind::Bytes(b)) => AnyValue::Bytes(b.clone()),
        Some(AnyKind::Array(a)) => AnyValue::Array(a.values.iter().map(|v| any(Some(v))).collect()),
        Some(AnyKind::KvList(l)) => AnyValue::KvList(kvs(&l.values)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proto_export_reads_as_its_json_twin() {
        let req = ExportMetricsServiceRequest {
            resource_metrics: vec![ResourceMetrics {
                resource: Some(Resource {
                    attributes: vec![KeyValue {
                        key: "service.name".into(),
                        value: Some(PbAnyValue {
                            value: Some(AnyKind::Str("cart".into())),
                        }),
                    }],
                }),
                scope_metrics: vec![ScopeMetrics {
                    scope: Some(InstrumentationScope {
                        name: "s".into(),
                        version: "1".into(),
                    }),
                    metrics: vec![Metric {
                        name: "n".into(),
                        unit: "1".into(),
                        data: Some(Data::Sum(Sum {
                            data_points: vec![NumberDataPoint {
                                attributes: vec![],
                                start_time_unix_nano: 1,
                                time_unix_nano: 5,
                                value: Some(NumberValue::AsInt(9_007_199_254_740_993)),
                                flags: 0,
                            }],
                            aggregation_temporality: 2,
                            is_monotonic: true,
                        })),
                    }],
                }],
            }],
        };
        let pb = read(&req.encode_to_vec()).unwrap();
        let js = otlp::read_json(
            r#"{"resourceMetrics":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"cart"}}]},
               "scopeMetrics":[{"scope":{"name":"s","version":"1"},"metrics":[{"name":"n","unit":"1",
               "sum":{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[
               {"startTimeUnixNano":"1","timeUnixNano":"5","asInt":"9007199254740993"}]}}]}]}]}"#,
        )
        .unwrap();
        assert_eq!(pb.points(), js.points());
    }

    #[test]
    fn bytes_that_are_not_an_export_are_refused() {
        assert!(read(&[0x0a, 0xff]).is_err());
    }
}
