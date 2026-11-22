//! Logs and events (RFC SMYSL-3 §9.7, §19.6, TL-P9): journald entries and CloudEvents read into
//! the event substrate, and template evidence materialised from it.
//!
//! - [`record_journald`] and [`record_cloudevents`] turn a read into events per stream, with each
//!   stream's series unit and its lineage, in the shape [`crate::trace::record`] gives spans.
//! - [`materialise`] runs Drain ([`crate::drain`]) over the messages of one stream and
//!   instrument in `[from, to)`, in event order, and makes one `x.tel/log-template` per template:
//!   `derived`, grounded on the stream, its digest the window's event digest.
//! - [`verify`] recomputes a template: `SMY-W605` when the window's events are not all here,
//!   `SMY-E604` when they are and the window or its templates differ.

use std::collections::BTreeMap;

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::{
    canonical_uid, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_series::{Backend, Event, StreamInfo, Substrate};

use crate::drain;
use crate::readers::cloudevents::{self, CloudEvent};
use crate::readers::journald::{self, Entry};
use crate::scrape::{Lineage, ScrapeError};
use crate::trace::{Recorded, Stream};

/// The scheme of a journald stream.
pub const JOURNALD: &str = "journald";
/// The scheme of a CloudEvents stream.
pub const CLOUDEVENTS: &str = "cloudevents";

/// One read item: its stream's scheme, key and kind, its origin's key, and its event.
struct Item {
    scheme: &'static str,
    kind: &'static str,
    key: String,
    origin: Option<String>,
    event: Event,
}

fn record(items: Vec<Item>, role: &str, instrument_key: &str) -> Result<Recorded, ScrapeError> {
    let unit_err = |e: tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument(role, instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);
    let mut streams: BTreeMap<Uid, Stream> = BTreeMap::new();
    let mut lineage = Lineage::default();
    for it in items {
        let unit = tel::series(it.scheme, &it.key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
        if let Some(o) = &it.origin {
            lineage.add(uid, "agent", o, inst)?;
        }
        streams
            .entry(uid)
            .or_insert_with(|| Stream {
                uid,
                unit,
                info: StreamInfo {
                    scheme: it.scheme.into(),
                    key: it.key.clone(),
                    kind: it.kind.into(),
                },
                events: Vec::new(),
            })
            .events
            .push(it.event);
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

/// Record journal entries, carried by the instrument `(role, instrument_key)`. Each host's
/// journal is the origin of its streams (`journald:<host>`).
pub fn record_journald(
    entries: &[Entry],
    role: &str,
    instrument_key: &str,
) -> Result<Recorded, ScrapeError> {
    let items = entries
        .iter()
        .map(|e| Item {
            scheme: JOURNALD,
            kind: "log",
            key: e.stream_key(),
            origin: (!e.host.is_empty()).then(|| format!("journald:{}", e.host)),
            event: Event::at_nanos(e.unix_nano(), e.body()),
        })
        .collect();
    record(items, role, instrument_key)
}

/// Record CloudEvents, carried by the instrument `(role, instrument_key)`. Each event's
/// `source` is the origin of its stream (`cloudevents:<source>`).
pub fn record_cloudevents(
    events: &[CloudEvent],
    role: &str,
    instrument_key: &str,
) -> Result<Recorded, ScrapeError> {
    let items = events
        .iter()
        .map(|e| Item {
            scheme: CLOUDEVENTS,
            kind: "event",
            key: e.stream_key(),
            origin: Some(format!("cloudevents:{}", e.source)),
            event: Event::at_nanos(e.unix_nano, e.body()),
        })
        .collect();
    record(items, role, instrument_key)
}

/// The message Drain reads from an event of a stream of `scheme` (§21).
pub fn message(scheme: &str, body: &[u8]) -> Option<String> {
    match scheme {
        JOURNALD => journald::message_of(body),
        CLOUDEVENTS => cloudevents::message_of(body),
        _ => None,
    }
}

/// One template of a window: the unit's fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub stream: Uid,
    pub scheme: String,
    pub key: String,
    pub inst: Uid,
    pub from_ms: u64,
    pub to_ms: u64,
    pub template: String,
    pub count: u64,
    pub digest: [u8; 32],
}

impl Template {
    /// The `x.tel/log-template` payload (§4.1), text keys in canonical order.
    pub fn payload(&self) -> Vec<u8> {
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
        let mut digest = Enc::new();
        digest.bytes(&self.digest);
        let mut kv: Vec<(Vec<u8>, Vec<u8>)> = vec![
            (text("tel:series"), text(&self.stream.canonical())),
            (text("tel:instrument"), text(&self.inst.canonical())),
            (text("tel:from_ms"), uint(self.from_ms)),
            (text("tel:to_ms"), uint(self.to_ms)),
            (text("tel:recipe"), text(drain::RECIPE)),
            (text("tel:template"), text(&self.template)),
            (text("tel:count"), uint(self.count)),
            (text("tel:digest"), digest.into_bytes()),
        ];
        kv.sort();
        let mut e = Enc::new();
        e.head(major::MAP, kv.len() as u64);
        for (k, v) in kv {
            e.raw(&k);
            e.raw(&v);
        }
        e.into_bytes()
    }

    /// The unit: `derived`, grounded on its stream, observed at the window's end.
    pub fn unit(&self) -> UnitCore {
        UnitCoreBuilder::new(
            SchemaId::Extension(tel::LOG_TEMPLATE.to_string()),
            tel::gist_of(&format!("template {}", self.template)),
            Status::Derived,
        )
        .source(
            SourceRef::new(
                SourceKind::Metric,
                format!("{}{}:{}", tel::SERIES_REF, self.scheme, self.key),
            )
            .observed_at(self.to_ms),
        )
        .grounds([self.stream])
        .payload(self.payload())
        .build()
        .expect("a template unit is well formed by construction")
    }
}

/// Why a window's templates could not be made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogsError {
    Series(smysl_series::Error),
    /// The substrate holds no events of the stream.
    NoStream,
    /// The stream's scheme has no message Drain reads.
    NotLogs(String),
}

impl core::fmt::Display for LogsError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            LogsError::Series(e) => write!(f, "{e}"),
            LogsError::NoStream => f.write_str("the substrate holds no event of this stream"),
            LogsError::NotLogs(s) => write!(f, "a `{s}` stream has no message to template"),
        }
    }
}

impl std::error::Error for LogsError {}

impl From<smysl_series::Error> for LogsError {
    fn from(e: smysl_series::Error) -> LogsError {
        LogsError::Series(e)
    }
}

/// Drain over the messages of `(stream, inst)` in `[from, to)` (§9.7).
pub fn materialise<B: Backend>(
    sub: &Substrate<B>,
    stream: &Uid,
    inst: &Uid,
    from: u64,
    to: u64,
) -> Result<Vec<Template>, LogsError> {
    let row = sub.stream(stream)?.ok_or(LogsError::NoStream)?;
    if !matches!(row.scheme.as_str(), JOURNALD | CLOUDEVENTS) {
        return Err(LogsError::NotLogs(row.scheme));
    }
    let read = sub.events(stream, inst, from, to)?;
    let msgs: Vec<String> = read
        .events
        .iter()
        .map(|e| message(&row.scheme, &e.body).unwrap_or_default())
        .collect();
    Ok(drain::run(msgs.iter().map(String::as_str))
        .into_iter()
        .map(|(template, count)| Template {
            stream: *stream,
            scheme: row.scheme.clone(),
            key: row.key.clone(),
            inst: *inst,
            from_ms: from,
            to_ms: to,
            template,
            count,
            digest: read.digest,
        })
        .collect())
}

/// The fields of an `x.tel/log-template` payload: what [`verify`] recomputes and `tent/1`
/// counts (RFC SMYSL-5 §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateFields {
    pub stream: Uid,
    pub inst: Uid,
    pub from: u64,
    pub to: u64,
    pub recipe: String,
    pub template: String,
    pub count: u64,
    pub digest: [u8; 32],
}

/// Read a template payload; `None` when it is not one.
pub fn template_fields(payload: &[u8]) -> Option<TemplateFields> {
    use smysl_core::cbor::reader::Dec;
    let mut d = Dec::new(payload);
    let n = d.map_head().ok()?;
    let mut f = (None, None, None, None, None, None, None, None);
    for _ in 0..n {
        match d.text().ok()? {
            "tel:series" => f.0 = d.text().ok()?.parse::<Uid>().ok(),
            "tel:instrument" => f.1 = d.text().ok()?.parse::<Uid>().ok(),
            "tel:from_ms" => f.2 = Some(d.uint().ok()?),
            "tel:to_ms" => f.3 = Some(d.uint().ok()?),
            "tel:recipe" => f.4 = Some(d.text().ok()?.to_string()),
            "tel:template" => f.5 = Some(d.text().ok()?.to_string()),
            "tel:count" => f.6 = Some(d.uint().ok()?),
            "tel:digest" => f.7 = d.bytes().ok()?.try_into().ok(),
            _ => {
                d.skip_item().ok()?;
            }
        }
    }
    Some(TemplateFields {
        stream: f.0?,
        inst: f.1?,
        from: f.2?,
        to: f.3?,
        recipe: f.4?,
        template: f.5?,
        count: f.6?,
        digest: f.7?,
    })
}

/// (stream, instrument, from, to): one window of one stream.
type WindowKey = (Uid, Uid, u64, u64);

/// The template half of the `tel-evidence` pass (§9.8). A recipe this build does not run is
/// not recomputed, and not reported.
pub fn verify<'a, B: Backend>(
    units: impl IntoIterator<Item = &'a UnitCore>,
    sub: &Substrate<B>,
) -> Vec<smysl_core::Diagnostic> {
    use smysl_core::{Code, Diagnostic};
    let mut out = Vec::new();
    // One Drain run per window, however many of its templates are checked.
    let mut runs: BTreeMap<WindowKey, Result<Vec<Template>, String>> = BTreeMap::new();
    for unit in units {
        if unit.schema.as_str() != tel::LOG_TEMPLATE {
            continue;
        }
        let uid = canonical_uid(unit);
        let Some(f) = unit.payload.as_deref().and_then(template_fields) else {
            continue;
        };
        if f.recipe != drain::RECIPE {
            continue;
        }
        let coverage = match sub.events_present(&f.stream, &f.inst, f.from, f.to) {
            Ok(c) => c,
            Err(e) => {
                out.push(
                    Diagnostic::on(Code::W605, uid)
                        .with_message(format!("the substrate could not be read: {e}")),
                );
                continue;
            }
        };
        if coverage.is_absent() || !coverage.evicted.is_empty() {
            out.push(Diagnostic::on(Code::W605, uid).with_message(format!(
                "the substrate does not hold every event of this window [{}, {}): not \
                 re-derivable",
                f.from, f.to
            )));
            continue;
        }
        let run = runs
            .entry((f.stream, f.inst, f.from, f.to))
            .or_insert_with(|| {
                materialise(sub, &f.stream, &f.inst, f.from, f.to).map_err(|e| e.to_string())
            });
        let why = match run {
            Err(e) => Some(format!("the window could not be recomputed: {e}")),
            Ok(ts) if ts.first().is_some_and(|t| t.digest != f.digest) => {
                Some("recomputed from the substrate, the window's digest differs".to_string())
            }
            Ok(ts) => match ts.iter().find(|t| t.template == f.template) {
                None => Some("recomputed, Drain makes no such template".to_string()),
                Some(t) if t.count != f.count => Some(format!(
                    "recomputed, the template's count is {}, not {}",
                    t.count, f.count
                )),
                Some(_) => None,
            },
        };
        if let Some(why) = why {
            out.push(Diagnostic::on(Code::E604, uid).with_message(why));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_series::MemBackend;

    const JOURNAL: &str = r#"{"__REALTIME_TIMESTAMP":"1768557600000000","_HOSTNAME":"h1","_SYSTEMD_UNIT":"cart.service","PRIORITY":"6","MESSAGE":"connection from 10.0.0.7 closed after 41 ms"}
{"__REALTIME_TIMESTAMP":"1768557601000000","_HOSTNAME":"h1","_SYSTEMD_UNIT":"cart.service","PRIORITY":"6","MESSAGE":"connection from 10.0.0.9 closed after 7 ms"}
{"__REALTIME_TIMESTAMP":"1768557602000000","_HOSTNAME":"h1","_SYSTEMD_UNIT":"cart.service","PRIORITY":"3","MESSAGE":"disk sda full"}
"#;

    #[test]
    fn a_window_of_a_journal_is_its_templates_and_they_verify() {
        let entries = journald::read(JOURNAL).unwrap();
        let r = record_journald(&entries, "collector", "journald:h1").unwrap();
        assert_eq!(r.streams.len(), 1);
        assert_eq!(r.lineage.origins.len(), 1, "journald:h1 is the reader too");
        let st = &r.streams[0];
        let mut sub = Substrate::new(MemBackend::default());
        sub.add_events(&st.uid, &r.inst, &st.info, &st.events, &crate::trace::tag)
            .unwrap();
        let (from, to) = (1_768_557_600_000, 1_768_557_660_000);
        let ts = materialise(&sub, &st.uid, &r.inst, from, to).unwrap();
        let got: Vec<(&str, u64)> = ts.iter().map(|t| (t.template.as_str(), t.count)).collect();
        assert_eq!(
            got,
            vec![
                ("connection from <IP> closed after <NUM> ms", 2),
                ("disk sda full", 1)
            ]
        );
        let units: Vec<UnitCore> = ts.iter().map(Template::unit).collect();
        assert!(verify(&units, &sub).is_empty());
        for u in &units {
            let a = smysl_core::AgentId::new("tool:smysl-tel").unwrap();
            let att = smysl_core::Attestation::new(
                canonical_uid(u),
                a.clone(),
                smysl_core::Op::Transformed,
                smysl_core::Rung::Computed,
                smysl_core::Hlc::new(1, 0, a),
            );
            assert!(tel::rule_tw_violations(u, &[att]).is_empty());
        }
        // A late record: the window is another window now.
        let late = journald::read(
            r#"{"__REALTIME_TIMESTAMP":"1768557603000000","_HOSTNAME":"h1","_SYSTEMD_UNIT":"cart.service","MESSAGE":"disk sdb full"}"#,
        )
        .unwrap();
        let r2 = record_journald(&late, "collector", "journald:h1").unwrap();
        sub.add_events(
            &st.uid,
            &r.inst,
            &st.info,
            &r2.streams[0].events,
            &crate::trace::tag,
        )
        .unwrap();
        let v = verify(&units, &sub);
        assert_eq!(v.len(), 2);
        assert!(v.iter().all(|d| d.code == smysl_core::Code::E604));
        // Evicted: not re-derivable.
        sub.evict(u64::MAX / 2, 1).unwrap();
        let v = verify(&units, &sub);
        assert!(v.iter().all(|d| d.code == smysl_core::Code::W605) && v.len() == 2);
    }
}
