//! `otlp-traces-json/1` and `otlp-traces/1` — an OTLP traces export (RFC SMYSL-3 §6, §19.6,
//! §20.1, TL-P9).
//!
//! **Two encodings, one reading**, as the metrics reader's: an `ExportTraceServiceRequest`
//! arrives as OTLP/JSON ([`read_json`], always built) or as protobuf ([`read_proto`], behind
//! `reader-otlp`), and both decode into the same [`Span`]s.
//!
//! **A span is a reading** (D-T1): it goes to the event substrate, not into a unit. Its event is
//! at its start, and its **body** is
//!
//! ```text
//! [ trace_id, span_id, parent_span_id, name, kind, start_unix_nano, end_unix_nano,
//!   status_code, status_message, attributes ]
//! ```
//!
//! the ids as byte strings (`parent_span_id` empty for a root), `kind` and `status_code` as
//! OTLP's numbers, the times in full nanoseconds, and the attributes as the §5.5 JCS text the
//! metrics reader writes an attribute set as. Span events, links, `trace_state`, flags and
//! dropped counts are not kept in the first cut, so two exports of one span that differ only
//! there are one event.
//!
//! **Its stream** (§6) is the JCS text of `{signal: "spans", resource, scope: {name, version}}`:
//! one stream per (resource, scope), as OTLP groups spans.
//!
//! **Ids** are refused when they are not OTLP's: a trace id of 16 bytes and a span id of 8, not
//! all zero. OTLP/JSON writes them as hex, not base64, and this reader reads hex in either case.

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::Enc;

use crate::jcs;
use crate::json::{self, Json};
use crate::readers::otlp::{self, AnyValue};
use crate::readers::{shape, ReadError};

/// The reader id of the JSON encoding.
pub const ID_JSON: &str = "otlp-traces-json/1";
/// The reader id of the protobuf encoding.
pub const ID: &str = "otlp-traces/1";

/// `SpanKind`, by number, as `x.tel/span`'s `span_kind` spells them; 0 is unspecified.
pub const KINDS: [&str; 6] = ["", "internal", "server", "client", "producer", "consumer"];
/// `Status.StatusCode`, by number, as `x.tel/span`'s `status` spells them.
pub const STATUSES: [&str; 3] = ["unset", "ok", "error"];

const KIND_NAMES: [&str; 6] = [
    "SPAN_KIND_UNSPECIFIED",
    "SPAN_KIND_INTERNAL",
    "SPAN_KIND_SERVER",
    "SPAN_KIND_CLIENT",
    "SPAN_KIND_PRODUCER",
    "SPAN_KIND_CONSUMER",
];
const STATUS_NAMES: [&str; 3] = ["STATUS_CODE_UNSET", "STATUS_CODE_OK", "STATUS_CODE_ERROR"];

/// One span, as decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub resource: Vec<(String, AnyValue)>,
    pub scope_name: String,
    pub scope_version: String,
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent: Option<[u8; 8]>,
    pub name: String,
    /// OTLP's `SpanKind`, 0–5 (an unknown number is kept).
    pub kind: u32,
    pub start_ns: u64,
    pub end_ns: u64,
    /// OTLP's `StatusCode`, 0–2 (an unknown number is kept).
    pub status: u32,
    pub status_message: String,
    pub attributes: Vec<(String, AnyValue)>,
}

impl Span {
    /// The key of the span stream it belongs to (§6).
    pub fn stream_key(&self) -> String {
        let s = |v: &str| Json::String(v.to_string());
        jcs::canonical(&Json::Object(vec![
            ("signal".to_string(), s("spans")),
            ("resource".to_string(), otlp::attrs(&self.resource)),
            (
                "scope".to_string(),
                Json::Object(vec![
                    ("name".to_string(), s(&self.scope_name)),
                    ("version".to_string(), s(&self.scope_version)),
                ]),
            ),
        ]))
        .0
    }

    /// Its event body (§20.1), in the kernel's deterministic CBOR.
    pub fn body(&self) -> Vec<u8> {
        let mut e = Enc::new();
        e.array_head(10);
        e.bytes(&self.trace_id);
        e.bytes(&self.span_id);
        e.bytes(self.parent.as_ref().map_or(&[][..], |p| &p[..]));
        e.text(&self.name);
        e.uint(u64::from(self.kind));
        e.uint(self.start_ns);
        e.uint(self.end_ns);
        e.uint(u64::from(self.status));
        e.text(&self.status_message);
        e.text(&jcs::canonical(&otlp::attrs(&self.attributes)).0);
        e.into_bytes()
    }

    /// The resource's `service.name`, when it is a string.
    pub fn service(&self) -> Option<&str> {
        self.resource.iter().find_map(|(k, v)| match v {
            AnyValue::Str(s) if k == "service.name" => Some(s.as_str()),
            _ => None,
        })
    }

    /// The origin key of its resource (§7), as the metrics reader names it.
    pub fn origin(&self) -> Option<String> {
        let get = |k: &str| {
            self.resource.iter().find_map(|(key, v)| match v {
                AnyValue::Str(s) if key == k => Some(s.as_str()),
                _ => None,
            })
        };
        let name = self.service().filter(|s| !s.is_empty())?;
        Some(format!(
            "otel-sdk:{}/{name}/{}",
            get("service.namespace").unwrap_or(""),
            get("service.instance.id").unwrap_or("")
        ))
    }
}

/// A span body's fields, as [`Span::body`] wrote them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent: Option<[u8; 8]>,
    pub name: String,
    pub kind: u32,
    pub start_ns: u64,
    pub end_ns: u64,
    pub status: u32,
    pub status_message: String,
    /// JCS text.
    pub attributes: String,
}

/// Read a span body. `Err` names what is malformed.
pub fn decode_body(bytes: &[u8]) -> Result<Body, String> {
    let mut d = Dec::new(bytes);
    let c = |e: smysl_core::CodecError| e.to_string();
    if d.array_head().map_err(c)? != 10 {
        return Err("a span body is an array of ten".into());
    }
    let trace_id: [u8; 16] = d
        .bytes()
        .map_err(c)?
        .try_into()
        .map_err(|_| "a trace id of 16 bytes".to_string())?;
    let span_id: [u8; 8] = d
        .bytes()
        .map_err(c)?
        .try_into()
        .map_err(|_| "a span id of 8 bytes".to_string())?;
    let parent = match d.bytes().map_err(c)? {
        [] => None,
        p => Some(
            p.try_into()
                .map_err(|_| "a parent span id of 8 bytes".to_string())?,
        ),
    };
    let name = d.text().map_err(c)?.to_string();
    let kind = u32::try_from(d.uint().map_err(c)?).map_err(|_| "a kind".to_string())?;
    let (start_ns, end_ns) = (d.uint().map_err(c)?, d.uint().map_err(c)?);
    let status = u32::try_from(d.uint().map_err(c)?).map_err(|_| "a status".to_string())?;
    let status_message = d.text().map_err(c)?.to_string();
    let attributes = d.text().map_err(c)?.to_string();
    if !d.is_done() {
        return Err("bytes after a span body".into());
    }
    Ok(Body {
        trace_id,
        span_id,
        parent,
        name,
        kind,
        start_ns,
        end_ns,
        status,
        status_message,
        attributes,
    })
}

/// The trace id a span body names, without decoding the rest: the tag `tel trace` finds a
/// trace's hours by (§19.6).
pub fn trace_of(body: &[u8]) -> Option<[u8; 16]> {
    let mut d = Dec::new(body);
    d.array_head().ok()?;
    d.bytes().ok()?.try_into().ok()
}

/// Lowercase hex.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Hex in either case, of exactly `N` bytes.
pub fn unhex<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// An id OTLP allows: the right length and not all zero.
fn id<const N: usize>(b: &[u8], what: &str) -> Result<[u8; N], ReadError> {
    let a: [u8; N] = b
        .try_into()
        .map_err(|_| shape(format!("a {what} of {} bytes, not {N}", b.len())))?;
    if a == [0u8; N] {
        return Err(shape(format!("an all-zero {what}")));
    }
    Ok(a)
}

/// Read an OTLP/JSON `ExportTraceServiceRequest`.
pub fn read_json(text: &str) -> Result<Vec<Span>, ReadError> {
    let doc = json::parse(text)?;
    let mut out = Vec::new();
    for rs in otlp::array(&doc, "resourceSpans")? {
        let resource = match otlp::field(rs, "resource") {
            Some(r) => otlp::kvs(r, "attributes")?,
            None => Vec::new(),
        };
        for ss in otlp::array(rs, "scopeSpans")? {
            let (scope_name, scope_version) = match otlp::field(ss, "scope") {
                Some(s) => (otlp::string(s, "name")?, otlp::string(s, "version")?),
                None => (String::new(), String::new()),
            };
            for sp in otlp::array(ss, "spans")? {
                let hexid = |name: &str| -> Result<Vec<u8>, ReadError> {
                    let s = otlp::string(sp, name)?;
                    if s.len() % 2 != 0 {
                        return Err(shape(format!("{name} {s:?} is not hex")));
                    }
                    (0..s.len() / 2)
                        .map(|i| {
                            s.get(2 * i..2 * i + 2)
                                .and_then(|h| u8::from_str_radix(h, 16).ok())
                                .ok_or_else(|| shape(format!("{name} {s:?} is not hex")))
                        })
                        .collect()
                };
                let parent = hexid("parentSpanId")?;
                let (status, status_message) = match otlp::field(sp, "status") {
                    Some(st) => (
                        otlp::enum_value(st, "code", &STATUS_NAMES)?,
                        otlp::string(st, "message")?,
                    ),
                    None => (0, String::new()),
                };
                out.push(Span {
                    resource: resource.clone(),
                    scope_name: scope_name.clone(),
                    scope_version: scope_version.clone(),
                    trace_id: id(&hexid("traceId")?, "trace id")?,
                    span_id: id(&hexid("spanId")?, "span id")?,
                    parent: if parent.is_empty() {
                        None
                    } else {
                        Some(id(&parent, "parent span id")?)
                    },
                    name: otlp::string(sp, "name")?,
                    kind: u32::try_from(otlp::enum_value(sp, "kind", &KIND_NAMES)?)
                        .map_err(|_| shape("kind is negative"))?,
                    start_ns: otlp::u64_of(sp, "startTimeUnixNano")?.unwrap_or(0),
                    end_ns: otlp::u64_of(sp, "endTimeUnixNano")?.unwrap_or(0),
                    status: u32::try_from(status).map_err(|_| shape("status is negative"))?,
                    status_message,
                    attributes: otlp::kvs(sp, "attributes")?,
                });
            }
        }
    }
    Ok(out)
}

/// Read a protobuf `ExportTraceServiceRequest`.
#[cfg(feature = "reader-otlp")]
pub fn read_proto(bytes: &[u8]) -> Result<Vec<Span>, ReadError> {
    pb::read(bytes)
}

/// The traces subset of `opentelemetry/proto/{collector/trace,trace}/v1`, upstream field numbers.
#[cfg(feature = "reader-otlp")]
mod pb {
    use prost::Message;

    use super::{id, Span};
    use crate::readers::otlp_pb::{kvs, InstrumentationScope, KeyValue, Resource};
    use crate::readers::{shape, ReadError};

    #[derive(Clone, PartialEq, Message)]
    pub struct ExportTraceServiceRequest {
        #[prost(message, repeated, tag = "1")]
        pub resource_spans: Vec<ResourceSpans>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct ResourceSpans {
        #[prost(message, optional, tag = "1")]
        pub resource: Option<Resource>,
        #[prost(message, repeated, tag = "2")]
        pub scope_spans: Vec<ScopeSpans>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct ScopeSpans {
        #[prost(message, optional, tag = "1")]
        pub scope: Option<InstrumentationScope>,
        #[prost(message, repeated, tag = "2")]
        pub spans: Vec<PbSpan>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct PbSpan {
        #[prost(bytes = "vec", tag = "1")]
        pub trace_id: Vec<u8>,
        #[prost(bytes = "vec", tag = "2")]
        pub span_id: Vec<u8>,
        #[prost(bytes = "vec", tag = "4")]
        pub parent_span_id: Vec<u8>,
        #[prost(string, tag = "5")]
        pub name: String,
        #[prost(int32, tag = "6")]
        pub kind: i32,
        #[prost(fixed64, tag = "7")]
        pub start_time_unix_nano: u64,
        #[prost(fixed64, tag = "8")]
        pub end_time_unix_nano: u64,
        #[prost(message, repeated, tag = "9")]
        pub attributes: Vec<KeyValue>,
        #[prost(message, optional, tag = "15")]
        pub status: Option<Status>,
    }

    #[derive(Clone, PartialEq, Message)]
    pub struct Status {
        #[prost(string, tag = "2")]
        pub message: String,
        #[prost(int32, tag = "3")]
        pub code: i32,
    }

    pub fn read(bytes: &[u8]) -> Result<Vec<Span>, ReadError> {
        let req = ExportTraceServiceRequest::decode(bytes)
            .map_err(|e| shape(format!("not an OTLP ExportTraceServiceRequest: {e}")))?;
        let mut out = Vec::new();
        for rs in &req.resource_spans {
            let resource = rs
                .resource
                .as_ref()
                .map(|r| kvs(&r.attributes))
                .unwrap_or_default();
            for ss in &rs.scope_spans {
                let (scope_name, scope_version) = ss
                    .scope
                    .as_ref()
                    .map(|s| (s.name.clone(), s.version.clone()))
                    .unwrap_or_default();
                for sp in &ss.spans {
                    let (status, status_message) = sp
                        .status
                        .as_ref()
                        .map(|s| (s.code, s.message.clone()))
                        .unwrap_or_default();
                    out.push(Span {
                        resource: resource.clone(),
                        scope_name: scope_name.clone(),
                        scope_version: scope_version.clone(),
                        trace_id: id(&sp.trace_id, "trace id")?,
                        span_id: id(&sp.span_id, "span id")?,
                        parent: if sp.parent_span_id.is_empty() {
                            None
                        } else {
                            Some(id(&sp.parent_span_id, "parent span id")?)
                        },
                        name: sp.name.clone(),
                        kind: u32::try_from(sp.kind).map_err(|_| shape("kind is negative"))?,
                        start_ns: sp.start_time_unix_nano,
                        end_ns: sp.end_time_unix_nano,
                        status: u32::try_from(status).map_err(|_| shape("status is negative"))?,
                        status_message,
                        attributes: kvs(&sp.attributes),
                    });
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const ONE: &str = r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"cart"}}]},"scopeSpans":[{"scope":{"name":"io.opentelemetry.http","version":"1.2.0"},"spans":[
      {"traceId":"4BF92F3577B34DA6A3CE929D0E0E4736","spanId":"00f067aa0ba902b7","name":"GET /cart","kind":2,"startTimeUnixNano":"1768557600000123456","endTimeUnixNano":"1768557600012000000","status":{"code":"STATUS_CODE_OK"},"attributes":[{"key":"http.status_code","value":{"intValue":"200"}}]},
      {"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"53995c3f42cd8ad8","parentSpanId":"00f067aa0ba902b7","name":"SELECT","kind":"SPAN_KIND_CLIENT","startTimeUnixNano":1768557600001000000,"endTimeUnixNano":"1768557600009000000"}
    ]}]}]}"#;

    #[test]
    fn a_json_export_reads_into_spans_whose_bodies_round_trip() {
        let spans = read_json(ONE).unwrap();
        assert_eq!(spans.len(), 2);
        let (root, child) = (&spans[0], &spans[1]);
        assert_eq!(root.trace_id, child.trace_id, "hex in either case");
        assert_eq!(child.parent, Some(root.span_id));
        assert_eq!((root.kind, child.kind), (2, 3));
        assert_eq!((root.status, child.status), (1, 0));
        assert_eq!(root.stream_key(), child.stream_key());
        assert!(root
            .stream_key()
            .starts_with(r#"{"resource":{"service.name":"cart"}"#));
        let b = decode_body(&root.body()).unwrap();
        assert_eq!(
            b.start_ns % 1_000_000,
            123_456,
            "the remainder is in the body"
        );
        assert_eq!(b.attributes, r#"{"http.status_code":200}"#);
        assert_eq!(trace_of(&root.body()), Some(root.trace_id));
        assert_eq!(hex(&b.span_id), "00f067aa0ba902b7");
        assert_eq!(unhex::<8>("00F067AA0BA902B7"), Some(root.span_id));
        assert_eq!(root.origin().as_deref(), Some("otel-sdk:/cart/"));
    }

    #[test]
    fn ids_that_are_not_otlp_ids_are_refused() {
        let bad = |t: &str, s: &str| {
            ONE.replace("4BF92F3577B34DA6A3CE929D0E0E4736", t).replace(
                r#""spanId":"00f067aa0ba902b7""#,
                &format!(r#""spanId":"{s}""#),
            )
        };
        assert!(read_json(&bad("00000000000000000000000000000000", "00f067aa0ba902b7")).is_err());
        assert!(read_json(&bad("4bf92f35", "00f067aa0ba902b7")).is_err());
        assert!(read_json(&bad("4bf92f3577b34da6a3ce929d0e0e4736", "zz")).is_err());
    }

    #[cfg(feature = "reader-otlp")]
    #[test]
    fn a_proto_export_reads_as_its_json_twin() {
        use prost::Message;
        let json = read_json(ONE).unwrap();
        let req = pb::ExportTraceServiceRequest {
            resource_spans: vec![pb::ResourceSpans {
                resource: Some(crate::readers::otlp_pb::Resource {
                    attributes: vec![crate::readers::otlp_pb::KeyValue {
                        key: "service.name".into(),
                        value: Some(crate::readers::otlp_pb::PbAnyValue {
                            value: Some(crate::readers::otlp_pb::AnyKind::Str("cart".into())),
                        }),
                    }],
                }),
                scope_spans: vec![pb::ScopeSpans {
                    scope: Some(crate::readers::otlp_pb::InstrumentationScope {
                        name: "io.opentelemetry.http".into(),
                        version: "1.2.0".into(),
                    }),
                    spans: json
                        .iter()
                        .map(|s| pb::PbSpan {
                            trace_id: s.trace_id.to_vec(),
                            span_id: s.span_id.to_vec(),
                            parent_span_id: s.parent.map(|p| p.to_vec()).unwrap_or_default(),
                            name: s.name.clone(),
                            kind: s.kind as i32,
                            start_time_unix_nano: s.start_ns,
                            end_time_unix_nano: s.end_ns,
                            attributes: if s.attributes.is_empty() {
                                vec![]
                            } else {
                                vec![crate::readers::otlp_pb::KeyValue {
                                    key: "http.status_code".into(),
                                    value: Some(crate::readers::otlp_pb::PbAnyValue {
                                        value: Some(crate::readers::otlp_pb::AnyKind::Int(200)),
                                    }),
                                }]
                            },
                            status: Some(pb::Status {
                                message: String::new(),
                                code: s.status as i32,
                            }),
                        })
                        .collect(),
                }],
            }],
        };
        let proto = read_proto(&req.encode_to_vec()).unwrap();
        assert_eq!(proto, json);
        let bodies: Vec<Vec<u8>> = proto.iter().map(Span::body).collect();
        assert_eq!(bodies, json.iter().map(Span::body).collect::<Vec<_>>());
    }
}
