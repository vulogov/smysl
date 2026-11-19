//! Traces (RFC SMYSL-3 §9.6, §19.6, TL-P9): spans read into the event substrate, and span
//! evidence materialised from it.
//!
//! - [`record`] turns a read export into events per span stream, with each stream's series unit
//!   and its lineage. It makes no span unit: a span is a reading (D-T1).
//! - [`materialise`] makes the `x.tel/span` units of one trace from what the substrate holds,
//!   and `x.tel/precedes` from each parent to each child. A unit's digest is its one event's,
//!   so materialising twice gives the same uids.
//! - [`verify`] recomputes a span unit against the substrate: `SMY-E604` when the event is there
//!   and is another span, `SMY-W605` when it is not there.
//!
//! **Two spans with one id** (`SMY-W617`): a stream holding two events of one (trace, span id)
//! whose bodies differ. Both are kept, both materialise, and each is a parent of the other's
//! children; [`TraceRead::duplicates`] names them.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::{
    canonical_uid, Relation, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_series::{event, Backend, Event, StreamInfo, Substrate};

use crate::readers::otlp_traces::{self, Span};
use crate::scrape::{Lineage, ScrapeError};

/// The scheme of a span stream.
pub const SCHEME: &str = "otel";
/// The kind of a span stream (§4.3).
pub const KIND: &str = "span";

/// One span stream of a read: its series unit and its events.
#[derive(Debug, Clone)]
pub struct Stream {
    pub uid: Uid,
    pub unit: UnitCore,
    pub info: StreamInfo,
    pub events: Vec<Event>,
}

/// What reading one export established.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub inst: Uid,
    pub instrument: UnitCore,
    /// By stream uid.
    pub streams: Vec<Stream>,
    pub lineage: Lineage,
}

/// The tag a span event is indexed under: its trace id.
pub fn tag(e: &Event) -> Option<[u8; 16]> {
    otlp_traces::trace_of(&e.body)
}

/// Record `spans`, carried to us by the instrument `(role, instrument_key)`.
pub fn record(spans: &[Span], role: &str, instrument_key: &str) -> Result<Recorded, ScrapeError> {
    let unit_err = |e: tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument(role, instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);
    let mut streams: BTreeMap<Uid, Stream> = BTreeMap::new();
    let mut lineage = Lineage::default();
    for s in spans {
        let key = s.stream_key();
        let unit = tel::series(SCHEME, &key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
        if let Some(origin) = s.origin() {
            lineage.add(uid, "agent", &origin, inst)?;
        }
        streams
            .entry(uid)
            .or_insert_with(|| Stream {
                uid,
                unit,
                info: StreamInfo {
                    scheme: SCHEME.into(),
                    key,
                    kind: KIND.into(),
                },
                events: Vec::new(),
            })
            .events
            .push(Event::at_nanos(s.start_ns, s.body()));
    }
    lineage.settle();
    for st in streams.values_mut() {
        st.events.sort();
        st.events.dedup();
    }
    Ok(Recorded {
        inst,
        instrument,
        streams: streams.into_values().collect(),
        lineage,
    })
}

/// One span as evidence: the unit's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpanEvidence {
    pub stream: Uid,
    pub scheme: String,
    pub key: String,
    pub inst: Uid,
    pub body: otlp_traces::Body,
    pub digest: [u8; 32],
    pub service: Option<String>,
}

impl SpanEvidence {
    /// The evidence of one event of `(stream, inst)`.
    pub fn of(
        stream: Uid,
        scheme: &str,
        key: &str,
        inst: Uid,
        ev: &Event,
    ) -> Result<SpanEvidence, String> {
        let body = otlp_traces::decode_body(&ev.body)?;
        let service = crate::json::parse(key).ok().and_then(|j| {
            j.get("resource")?
                .get("service.name")?
                .as_str()
                .map(str::to_string)
        });
        Ok(SpanEvidence {
            stream,
            scheme: scheme.to_string(),
            key: key.to_string(),
            inst,
            body,
            digest: event::digest(std::slice::from_ref(ev)),
            service,
        })
    }

    pub fn start_ms(&self) -> u64 {
        self.body.start_ns / 1_000_000
    }

    /// The `x.tel/span` payload (§4.1), text keys in canonical order.
    pub fn payload(&self) -> Vec<u8> {
        let b = &self.body;
        let mut entries: Vec<(&str, Vec<u8>)> = Vec::new();
        let text = |s: &str| {
            let mut e = Enc::new();
            e.text(s);
            e.into_bytes()
        };
        let uint = |n: u64| {
            let mut e = Enc::new();
            e.uint(n);
            e.into_bytes()
        };
        entries.push(("tel:series", text(&self.stream.canonical())));
        entries.push(("tel:instrument", text(&self.inst.canonical())));
        entries.push(("tel:trace", text(&otlp_traces::hex(&b.trace_id))));
        entries.push(("tel:span_id", text(&otlp_traces::hex(&b.span_id))));
        entries.push(("tel:name", text(&b.name)));
        entries.push(("tel:start_ms", uint(b.start_ns / 1_000_000)));
        entries.push(("tel:end_ms", uint(b.end_ns / 1_000_000)));
        entries.push(("tel:digest", {
            let mut e = Enc::new();
            e.bytes(&self.digest);
            e.into_bytes()
        }));
        if let Some(p) = b.parent {
            entries.push(("tel:parent", text(&otlp_traces::hex(&p))));
        }
        if let Some(k) = otlp_traces::KINDS
            .get(b.kind as usize)
            .filter(|k| !k.is_empty())
        {
            entries.push(("tel:span_kind", text(k)));
        }
        if let Some(s) = otlp_traces::STATUSES.get(b.status as usize) {
            entries.push(("tel:status", text(s)));
        }
        if let Some(s) = &self.service {
            entries.push(("tel:service", text(s)));
        }
        let mut kv: Vec<(Vec<u8>, Vec<u8>)> =
            entries.into_iter().map(|(k, v)| (text(k), v)).collect();
        kv.sort();
        let mut e = Enc::new();
        e.head(major::MAP, kv.len() as u64);
        for (k, v) in kv {
            e.raw(&k);
            e.raw(&v);
        }
        e.into_bytes()
    }

    /// The `x.tel/span` unit: `measured`, its source the stream observed at the span's start.
    pub fn unit(&self) -> UnitCore {
        let svc = self.service.as_deref().unwrap_or("?");
        UnitCoreBuilder::new(
            SchemaId::Extension(tel::SPAN.to_string()),
            tel::gist_of(&format!("span {svc} {}", self.body.name)),
            Status::Measured,
        )
        .source(
            SourceRef::new(
                SourceKind::Metric,
                format!("{}{}:{}", tel::SERIES_REF, self.scheme, self.key),
            )
            .observed_at(self.start_ms()),
        )
        .payload(self.payload())
        .build()
        .expect("a span unit is well formed by construction")
    }
}

/// What materialising one trace found.
#[derive(Debug, Clone, Default)]
pub struct TraceRead {
    /// Its span units, by uid.
    pub units: BTreeMap<Uid, UnitCore>,
    /// `x.tel/precedes`, parent to child.
    pub relations: Vec<Relation>,
    /// Hours that held spans of the trace and have been evicted: `(stream, inst, hour)`.
    pub evicted: Vec<(Uid, Uid, u64)>,
    /// Span ids held twice with different bodies in one stream (`SMY-W617`).
    pub duplicates: Vec<(Uid, [u8; 8])>,
}

/// Materialise the spans of `trace` the substrate holds (§9.6).
pub fn materialise<B: Backend>(
    sub: &Substrate<B>,
    trace: &[u8; 16],
) -> Result<TraceRead, smysl_series::Error> {
    let mut out = TraceRead::default();
    let mut spans: Vec<(Uid, SpanEvidence)> = Vec::new();
    let hours: BTreeSet<(Uid, Uid, u64)> = sub.tagged(trace)?.into_iter().collect();
    for (stream, inst, hour) in hours {
        let row = sub.stream(&stream)?;
        let read = sub.events(&stream, &inst, hour, hour + event::HOUR_MS)?;
        if read.events.is_empty() {
            if !sub
                .events_present(&stream, &inst, hour, hour + 1)?
                .evicted
                .is_empty()
            {
                out.evicted.push((stream, inst, hour));
            }
            continue;
        }
        let Some(row) = row else { continue };
        for ev in &read.events {
            if otlp_traces::trace_of(&ev.body).as_ref() != Some(trace) {
                continue;
            }
            let Ok(s) = SpanEvidence::of(stream, &row.scheme, &row.key, inst, ev) else {
                continue;
            };
            let u = s.unit();
            let uid = canonical_uid(&u);
            out.units.insert(uid, u);
            spans.push((uid, s));
        }
    }
    let mut by_id: BTreeMap<[u8; 8], Vec<Uid>> = BTreeMap::new();
    let mut seen: BTreeMap<(Uid, [u8; 8]), BTreeSet<[u8; 32]>> = BTreeMap::new();
    for (uid, s) in &spans {
        by_id.entry(s.body.span_id).or_default().push(*uid);
        seen.entry((s.stream, s.body.span_id))
            .or_default()
            .insert(s.digest);
    }
    out.duplicates = seen
        .into_iter()
        .filter(|(_, d)| d.len() > 1)
        .map(|(k, _)| k)
        .collect();
    for (child, s) in &spans {
        if let Some(parents) = s.body.parent.and_then(|p| by_id.get(&p)) {
            for parent in parents {
                out.relations
                    .push(Relation::new(tel_precedes(), *parent, *child));
            }
        }
    }
    out.relations.sort_by_key(|r| (r.from, r.to));
    out.relations.dedup_by_key(|r| (r.from, r.to));
    Ok(out)
}

fn tel_precedes() -> smysl_core::RelKind {
    smysl_core::RelKind::Extension(tel::PRECEDES.to_string())
}

/// The fields of an `x.tel/span` payload [`verify`] reads.
struct Fields {
    stream: Uid,
    inst: Uid,
    trace: [u8; 16],
    span_id: [u8; 8],
    start_ms: u64,
    digest: [u8; 32],
}

fn fields(payload: &[u8]) -> Option<Fields> {
    use smysl_core::cbor::reader::Dec;
    let mut d = Dec::new(payload);
    let n = d.map_head().ok()?;
    let (mut stream, mut inst, mut trace, mut span_id, mut start, mut digest) =
        (None, None, None, None, None, None);
    for _ in 0..n {
        match d.text().ok()? {
            "tel:series" => stream = d.text().ok()?.parse::<Uid>().ok(),
            "tel:instrument" => inst = d.text().ok()?.parse::<Uid>().ok(),
            "tel:trace" => trace = otlp_traces::unhex::<16>(d.text().ok()?),
            "tel:span_id" => span_id = otlp_traces::unhex::<8>(d.text().ok()?),
            "tel:start_ms" => start = Some(d.uint().ok()?),
            "tel:digest" => digest = d.bytes().ok()?.try_into().ok(),
            _ => {
                d.skip_item().ok()?;
            }
        }
    }
    Some(Fields {
        stream: stream?,
        inst: inst?,
        trace: trace?,
        span_id: span_id?,
        start_ms: start?,
        digest: digest?,
    })
}

/// The span half of the `tel-evidence` pass (§9.8): each span unit recomputed from the
/// substrate. `SMY-W605` when no event of its (trace, span id) is at its start, `SMY-E604` when
/// one is and none has its digest.
pub fn verify<'a, B: Backend>(
    units: impl IntoIterator<Item = &'a UnitCore>,
    sub: &Substrate<B>,
) -> Vec<smysl_core::Diagnostic> {
    use smysl_core::{Code, Diagnostic};
    let mut out = Vec::new();
    for unit in units {
        if unit.schema.as_str() != tel::SPAN {
            continue;
        }
        let uid = canonical_uid(unit);
        let Some(f) = unit.payload.as_deref().and_then(fields) else {
            continue;
        };
        let read = match sub.events(&f.stream, &f.inst, f.start_ms, f.start_ms + 1) {
            Ok(r) => r,
            Err(e) => {
                out.push(
                    Diagnostic::on(Code::W605, uid)
                        .with_message(format!("the substrate could not be read: {e}")),
                );
                continue;
            }
        };
        let same: Vec<&Event> = read
            .events
            .iter()
            .filter(|e| {
                otlp_traces::decode_body(&e.body)
                    .is_ok_and(|b| b.trace_id == f.trace && b.span_id == f.span_id)
            })
            .collect();
        if same.is_empty() {
            let evicted = sub
                .events_present(&f.stream, &f.inst, f.start_ms, f.start_ms + 1)
                .map(|c| !c.evicted.is_empty())
                .unwrap_or(false);
            out.push(Diagnostic::on(Code::W605, uid).with_message(format!(
                "the substrate holds no event of span {} at {}{}: not re-derivable",
                otlp_traces::hex(&f.span_id),
                f.start_ms,
                if evicted {
                    " (its hour was evicted)"
                } else {
                    ""
                }
            )));
            continue;
        }
        if same
            .iter()
            .any(|e| event::digest(std::slice::from_ref(*e)) == f.digest)
        {
            continue;
        }
        out.push(Diagnostic::on(Code::E604, uid).with_message(format!(
            "span {} is in the substrate, and no event of it has this unit's digest",
            otlp_traces::hex(&f.span_id)
        )));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_series::MemBackend;

    const TWO: &str = r#"{"resourceSpans":[{"resource":{"attributes":[{"key":"service.name","value":{"stringValue":"cart"}}]},"scopeSpans":[{"scope":{"name":"s","version":"1"},"spans":[
      {"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"00f067aa0ba902b7","name":"GET /cart","kind":2,"startTimeUnixNano":"1768557600000123456","endTimeUnixNano":"1768557600012000000"},
      {"traceId":"4bf92f3577b34da6a3ce929d0e0e4736","spanId":"53995c3f42cd8ad8","parentSpanId":"00f067aa0ba902b7","name":"SELECT","kind":3,"startTimeUnixNano":"1768557600001000000","endTimeUnixNano":"1768557600009000000"},
      {"traceId":"0af7651916cd43dd8448eb211c80319c","spanId":"b7ad6b7169203331","name":"other","startTimeUnixNano":"1768557600002000000","endTimeUnixNano":"1768557600003000000"}
    ]}]}]}"#;

    fn loaded(json: &str) -> (Substrate<MemBackend>, Recorded) {
        let spans = otlp_traces::read_json(json).unwrap();
        let r = record(&spans, "collector", "otel-collector:c1").unwrap();
        let mut sub = Substrate::new(MemBackend::default());
        for s in &r.streams {
            sub.add_events(&s.uid, &r.inst, &s.info, &s.events, &tag)
                .unwrap();
        }
        (sub, r)
    }

    #[test]
    fn a_trace_materialises_its_spans_and_their_order_and_verifies() {
        let (mut sub, r) = loaded(TWO);
        assert_eq!(r.streams.len(), 1);
        assert_eq!(r.streams[0].events.len(), 3);
        assert_eq!(r.lineage.origins.len(), 1);
        let trace = otlp_traces::unhex::<16>("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let t = materialise(&sub, &trace).unwrap();
        assert_eq!(t.units.len(), 2, "the other trace is not this one");
        assert_eq!(t.relations.len(), 1);
        let rel = &t.relations[0];
        let parent = &t.units[&rel.from];
        assert!(parent.gist.contains("GET /cart"), "{}", parent.gist);
        assert_eq!(
            parent.source.as_ref().unwrap().observed,
            Some(1_768_557_600_000)
        );
        assert!(t.duplicates.is_empty());
        // Twice: the same uids.
        let again = materialise(&sub, &trace).unwrap();
        assert_eq!(
            again.units.keys().collect::<Vec<_>>(),
            t.units.keys().collect::<Vec<_>>()
        );
        // Rule TW is satisfied by construction.
        for u in t.units.values() {
            let a = smysl_core::AgentId::new("tool:smysl-tel").unwrap();
            let att = smysl_core::Attestation::new(
                canonical_uid(u),
                a.clone(),
                smysl_core::Op::Imported,
                smysl_core::Rung::Computed,
                smysl_core::Hlc::new(1, 0, a),
            );
            assert!(tel::rule_tw_violations(u, &[att]).is_empty());
        }
        assert!(verify(t.units.values(), &sub).is_empty());

        // A tampered unit is E604; an evicted span is W605.
        let mut forged = parent.clone();
        let p = forged.payload.take().unwrap();
        let mut b = p.clone();
        let pos = p
            .windows(10)
            .position(|w| w == b"tel:digest")
            .expect("a digest key");
        b[pos + 12] ^= 1;
        forged.payload = Some(b);
        let v = verify([&forged], &sub);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].code, smysl_core::Code::E604);
        sub.evict(u64::MAX / 2, 5).unwrap();
        let v = verify(t.units.values(), &sub);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|d| d.code == smysl_core::Code::W605));
        let gone = materialise(&sub, &trace).unwrap();
        assert!(gone.units.is_empty());
        assert_eq!(gone.evicted.len(), 1);
    }

    #[test]
    fn two_spans_with_one_id_both_materialise_and_are_named() {
        let changed = TWO.replace(r#""name":"SELECT""#, r#""name":"SELECT 1""#);
        let (mut sub, r) = loaded(TWO);
        let spans = otlp_traces::read_json(&changed).unwrap();
        let r2 = record(&spans, "collector", "otel-collector:c1").unwrap();
        for s in &r2.streams {
            sub.add_events(&s.uid, &r.inst, &s.info, &s.events, &tag)
                .unwrap();
        }
        let trace = otlp_traces::unhex::<16>("4bf92f3577b34da6a3ce929d0e0e4736").unwrap();
        let t = materialise(&sub, &trace).unwrap();
        assert_eq!(t.units.len(), 3);
        assert_eq!(t.relations.len(), 2, "the root precedes both");
        assert_eq!(t.duplicates.len(), 1);
    }
}
