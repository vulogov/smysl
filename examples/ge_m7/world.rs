//! A case as the agent's tools see it (RFC SMYSL-10 §4.2), and the ledger arms (b) and (d) write.
//!
//! The case goes into an in-memory substrate at load: each metric column a series, each log
//! container a journald stream (`logs::materialise` templates journald and CloudEvents streams
//! only), and each trace operation two series, its calls and its mean span duration per second
//! (R1-3). Every tool computes its units the same way in every arm, so the ids an agent sees in
//! (a) are the uids (b) records; only (b) and (d) commit them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use smysl::schemas::tel;
use smysl::series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl::stage::{prepare_attested, Attesting};
use smysl::tel::investigate as inv;
use smysl::tel::readers::journald::Entry;
use smysl::tel::{evidence, logs};
use smysl::text::library::Library;
use smysl::{
    canonical_uid, AgentId, Attestation, Decimal, Hlc, Op, RelKind, Relation, Rung, StoreRead, Uid,
    UnitCore,
};

use crate::case::{split_column, Anon, Raw};

/// The case's length, seconds. Every RE2 case is 24 minutes, the fault at minute 12.
pub const CASE_S: u64 = 1_440;
/// GE-I3's window: `compare` and BARO look six minutes each side.
pub const HALF_S: u64 = 360;
/// The ranks a sketched window reports (R1-1).
pub const QS: &[&str] = &["0.5", "0.9", "0.99"];
pub const ALPHA: &str = "0.01";
/// A tool result's cap, characters (Q-4).
/// The M7-P2 probe (`--digits N`): tools print a non-integer to `N` significant digits, so a
/// value the agent quotes is no longer the unit's canonical text. Unset: canonical, §4.2.
pub static DIGITS: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();

/// `v` to `n` significant digits, trailing zeros dropped.
pub fn significant(v: &str, n: usize) -> String {
    let Ok(x) = v.parse::<f64>() else {
        return v.to_string();
    };
    if x == 0.0 || !x.is_finite() {
        return v.to_string();
    }
    let mag = x.abs().log10().floor() as i64;
    let places = (n as i64 - 1 - mag).max(0) as usize;
    let s = format!("{x:.places$}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

pub const CAP: usize = 2_000;

/// A value's text as a decimal, or as binary64 rounded to nine places when it is not one.
pub fn decimal(raw: &str) -> Option<Decimal> {
    if let Ok(d) = Decimal::parse_token(raw) {
        return Some(d);
    }
    let x: f64 = raw.parse().ok()?;
    x.is_finite()
        .then(|| Decimal::parse_token(&format!("{x:.9}")).ok())
        .flatten()
}

/// A mean of whole microseconds, rounded half up to a microsecond, in milliseconds.
pub fn mean_ms(sum_us: u64, n: u64) -> String {
    let m = (2 * sum_us + n) / (2 * n);
    format!("{}.{:03}", m / 1000, m % 1000)
}

/// What a series is, in the case's own files: the oracle recomputes from this (§5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A column of `simple_metrics.csv`.
    Column(String),
    /// A trace operation's calls per second: (service as in traces.csv, operation).
    Calls(String, String),
    /// Its mean span duration per second, ms.
    Latency(String, String),
}

pub struct Series {
    pub uid: Uid,
    pub unit: UnitCore,
    pub origin: Origin,
}

/// Every unit a tool returned an id for, and what it is.
#[derive(Debug, Clone)]
pub enum Known {
    /// A window of `series` (the agent's name) over `[from, to)` seconds.
    Window {
        series: String,
        from: u64,
        to: u64,
        sketched: bool,
    },
    /// A template of `service`'s logs over `[from, to)`.
    Template {
        service: String,
        from: u64,
        to: u64,
        template: String,
    },
    /// A hypothesis the agent wrote.
    Hypothesis,
    /// One of BARO's.
    Ranked,
}

/// One quote as the agent wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub id: String,
    pub field: String,
    pub value: String,
}

impl Quote {
    pub fn text(&self) -> String {
        format!("{}#{}={}", self.id, self.field, self.value)
    }
}

/// A hypothesis as written, and what rule TQ said of each quote, in (b) and (d).
#[derive(Debug, Clone)]
pub struct Written {
    pub uid: Option<Uid>,
    pub quotes: Vec<Quote>,
    /// Per quote: `Some(why)` when TQ failed it.
    pub tq: Vec<Option<String>>,
    pub grounds: Vec<Uid>,
}

/// The ledger: a library per run.
pub struct Ledger {
    pub lib: Library,
    pub agent: AgentId,
    pub investigation: Uid,
    pub alert: Uid,
    committed: BTreeSet<Uid>,
    clock: u64,
}

struct By<'a> {
    tool: &'a AgentId,
    model: &'a AgentId,
    model_units: &'a BTreeSet<Uid>,
    tw: BTreeMap<Uid, (Op, Rung)>,
    now: u64,
}

impl Attesting for By<'_> {
    fn for_unit(&self, uid: Uid) -> Attestation {
        if self.model_units.contains(&uid) {
            return Attestation::new(
                uid,
                self.model.clone(),
                Op::Authored,
                Rung::Document,
                Hlc::new(self.now, 0, self.model.clone()),
            );
        }
        let (op, rung) = self
            .tw
            .get(&uid)
            .copied()
            .unwrap_or((Op::Imported, Rung::Computed));
        Attestation::new(
            uid,
            self.tool.clone(),
            op,
            rung,
            Hlc::new(self.now, 0, self.tool.clone()),
        )
    }
}

/// The case, its substrate, and, in (b) and (d), its ledger.
pub struct World {
    pub raw: Raw,
    pub anon: Anon,
    pub sub: Substrate<MemBackend>,
    pub metric_inst: Uid,
    pub metric_inst_unit: UnitCore,
    /// By the agent's name: `svc-07/cpu`, `svc-07/op3/calls`.
    pub series: BTreeMap<String, Series>,
    /// By `svc-NN`: its log stream and instrument.
    pub streams: BTreeMap<String, (Uid, Uid)>,
    /// The units the log streams came with: instrument, stream units, lineage.
    log_units: Vec<UnitCore>,
    log_relations: Vec<Relation>,
    /// By `svc-NN`: its trace operations, `opN` → anonymised name.
    pub ops: BTreeMap<String, BTreeMap<String, String>>,
    /// Every unit an id was printed for.
    pub known: BTreeMap<Uid, (Known, UnitCore)>,
    pub ledger: Option<Ledger>,
    pub written: Vec<Written>,
}

/// A payload of text fields.
fn payload(entries: &[(&str, String)]) -> Vec<u8> {
    use smysl::surface::hjson::{HObject, HValue, Spanned};
    let span = smysl::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(
            Spanned::new((*k).to_string(), span),
            Spanned::new(HValue::Str(v.clone()), span),
        );
    }
    smysl::surface::object_to_payload(&o).expect("a payload")
}

fn tool_agent() -> AgentId {
    AgentId::new("tool:smysl-tel").expect("an agent id")
}

impl World {
    pub fn new(raw: Raw) -> World {
        let anon = Anon::new(&raw.services(), &format!("{}/{}", raw.system, raw.rel));
        let mut sub = Substrate::new(MemBackend::default());
        let metric_inst_unit =
            tel::instrument("exporter", "rcaeval:simple_metrics").expect("an instrument");
        let metric_inst = canonical_uid(&metric_inst_unit);
        let mut series = BTreeMap::new();
        let mut add = |sub: &mut Substrate<MemBackend>,
                       name: String,
                       key: String,
                       origin: Origin,
                       samples: Vec<Sample>| {
            let unit = tel::series("prom", &key).expect("a series");
            let uid = canonical_uid(&unit);
            let info = SeriesInfo {
                scheme: "prom".into(),
                key,
                class: "gauge".into(),
            };
            sub.add(&uid, &metric_inst, &info, &samples)
                .expect("the samples");
            series.insert(name, Series { uid, unit, origin });
        };
        for (col, pts) in &raw.metrics {
            let Some((svc, metric)) = split_column(col) else {
                continue;
            };
            let Some(a) = anon.of(svc) else { continue };
            let samples: Vec<Sample> = pts
                .iter()
                .filter_map(|(ts, v)| Some(Sample::value(ts * 1000, decimal(v)?)))
                .collect();
            add(
                &mut sub,
                format!("{a}/{metric}"),
                format!("{metric}{{service=\"{a}\"}}"),
                Origin::Column(col.clone()),
                samples,
            );
        }
        let mut ops: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        if let Some(spans) = &raw.spans {
            // (service, operation) → second → (calls, sum µs)
            let mut per: BTreeMap<(String, String), BTreeMap<u64, (u64, u64)>> = BTreeMap::new();
            for (ms, svc, op, dur) in spans {
                let e = per
                    .entry((svc.clone(), op.clone()))
                    .or_default()
                    .entry(ms / 1000)
                    .or_default();
                e.0 += 1;
                e.1 += dur;
            }
            let mut n: BTreeMap<String, usize> = BTreeMap::new();
            for ((svc, op), secs) in per {
                let a = anon
                    .of(&svc)
                    .map(str::to_string)
                    .unwrap_or_else(|| anon.text(&svc));
                let k = n.entry(a.clone()).or_default();
                *k += 1;
                let opn = format!("op{k}");
                ops.entry(a.clone())
                    .or_default()
                    .insert(opn.clone(), anon.text(&op));
                let calls: Vec<Sample> = secs
                    .iter()
                    .map(|(s, (c, _))| Sample::value(s * 1000, Decimal::from_i64(*c as i64)))
                    .collect();
                let lat: Vec<Sample> = secs
                    .iter()
                    .filter_map(|(s, (c, sum))| {
                        Some(Sample::value(
                            s * 1000,
                            Decimal::parse_token(&mean_ms(*sum, *c)).ok()?,
                        ))
                    })
                    .collect();
                add(
                    &mut sub,
                    format!("{a}/{opn}/calls"),
                    format!("calls{{service=\"{a}\",op=\"{opn}\"}}"),
                    Origin::Calls(svc.clone(), op.clone()),
                    calls,
                );
                add(
                    &mut sub,
                    format!("{a}/{opn}/latency-ms"),
                    format!("latency_ms{{service=\"{a}\",op=\"{opn}\"}}"),
                    Origin::Latency(svc.clone(), op.clone()),
                    lat,
                );
            }
        }
        // Logs: one journald stream per container, its messages renamed.
        let entries: Vec<Entry> = raw
            .logs
            .iter()
            .filter_map(|(ms, who, msg)| {
                let a = anon.of(who)?;
                Some(Entry {
                    realtime_us: ms * 1000,
                    message: anon.text(msg),
                    priority: None,
                    host: "rcaeval".into(),
                    unit: a.to_string(),
                    fields: "{}".into(),
                })
            })
            .collect();
        let mut streams = BTreeMap::new();
        let (mut log_units, mut log_relations) = (Vec::new(), Vec::new());
        if !entries.is_empty() {
            let r = logs::record_journald(&entries, "collector", "rcaeval:logs")
                .expect("the log streams");
            for st in &r.streams {
                sub.add_events(
                    &st.uid,
                    &r.inst,
                    &st.info,
                    &st.events,
                    &smysl::tel::trace::tag,
                )
                .expect("the events");
                let unit = r.streams.iter().find(|s| s.uid == st.uid).and_then(|_| {
                    entries
                        .iter()
                        .find(|e| e.stream_key() == st.info.key)
                        .map(|e| e.unit.clone())
                });
                if let Some(u) = unit {
                    streams.insert(u, (st.uid, r.inst));
                }
                log_units.push(st.unit.clone());
            }
            log_units.push(r.instrument.clone());
            log_units.extend(r.lineage.origins.values().cloned());
            log_relations.extend(r.lineage.relations.iter().cloned());
        }
        World {
            raw,
            anon,
            sub,
            metric_inst,
            metric_inst_unit,
            series,
            streams,
            log_units,
            log_relations,
            ops,
            known: BTreeMap::new(),
            ledger: None,
            written: Vec::new(),
        }
    }

    /// The case's second `s` as unix ms.
    pub fn ms(&self, s: u64) -> u64 {
        (self.raw.t0 + s) * 1000
    }

    /// The alert instant, case seconds: the injection, rounded down to the minute (§5.3).
    pub fn alert_s(&self) -> u64 {
        (self.raw.inject.saturating_sub(self.raw.t0)) / 60 * 60
    }

    /// The services the agent can name, `svc-NN`, sorted.
    pub fn services(&self) -> Vec<String> {
        self.anon.from.keys().cloned().collect()
    }

    // ---- the ledger -------------------------------------------------------------------

    /// Open the ledger for (b) and (d): a library under `dir`, the case's series, streams and
    /// instruments, the alert, and the investigation, attested by `model`.
    pub fn open_ledger(&mut self, dir: &Path, model: &AgentId) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir).expect("the run directory");
        let lib = Library::create(dir).expect("a library");
        let at = self.ms(self.alert_s());
        let alert = smysl::UnitCoreBuilder::new(
            smysl::SchemaId::Extension(tel::ALERT.to_string()),
            tel::gist_of("something is wrong"),
            smysl::Status::Cited,
        )
        .source(
            smysl::SourceRef::new(smysl::SourceKind::Tool, "alert:ge-m7".to_string())
                .observed_at(at),
        )
        .payload(payload(&[
            ("tel:system", "ge-m7".to_string()),
            ("tel:key", format!("case-{}", self.alert_s())),
            ("tel:state", "firing".to_string()),
        ]))
        .build()
        .expect("an alert");
        let open = inv::open("what is the root cause of the alert?", at, None);
        let (alert_uid, inv_uid) = (canonical_uid(&alert), canonical_uid(&open));
        let mut ledger = Ledger {
            lib,
            agent: model.clone(),
            investigation: inv_uid,
            alert: alert_uid,
            committed: BTreeSet::new(),
            clock: at,
        };
        let mut units = vec![self.metric_inst_unit.clone()];
        units.extend(self.series.values().map(|s| s.unit.clone()));
        units.extend(self.log_units.iter().cloned());
        let rels = self.log_relations.clone();
        commit(&mut ledger, units, rels, &BTreeSet::new());
        let model_units: BTreeSet<Uid> = [alert_uid, inv_uid].into();
        commit(
            &mut ledger,
            vec![alert, open],
            vec![Relation::new(
                RelKind::Extension(tel::CONSULTED.to_string()),
                inv_uid,
                alert_uid,
            )],
            &model_units,
        );
        let view = smysl::Record::View(inv::view(inv_uid));
        ledger.lib.append_records(&[view]).expect("the view");
        self.ledger = Some(ledger);
    }

    /// Commit units a tool returned, with `consulted` to each, in (b) and (d).
    fn consult(&mut self, units: Vec<UnitCore>, extra: Vec<Relation>) {
        let Some(l) = self.ledger.as_mut() else {
            return;
        };
        let mut rels = extra;
        for u in &units {
            let uid = canonical_uid(u);
            if u.schema.as_str() == tel::WINDOW {
                if let Some(f) = u
                    .payload
                    .as_deref()
                    .and_then(|p| tel::window_fields(p).ok())
                {
                    if let Some(s) = f.series.and_then(|s| s.parse::<Uid>().ok()) {
                        rels.push(Relation::new(
                            RelKind::Extension(tel::OF_SERIES.to_string()),
                            uid,
                            s,
                        ));
                    }
                }
            }
            rels.push(inv::consulted(l.investigation, uid));
        }
        commit(l, units, rels, &BTreeSet::new());
    }

    // ---- the tools ----------------------------------------------------------------------

    fn print_value(u: &UnitCore, field: &str, i: Option<usize>) -> String {
        let v = u
            .payload
            .as_deref()
            .and_then(|p| tel::quoted_value(p, field, i))
            .unwrap_or_else(|| "-".into());
        match DIGITS.get().copied().flatten() {
            Some(n) if v.contains('.') => significant(&v, n),
            _ => v,
        }
    }

    /// Exact and sketched windows of `name` over `[from, to)` seconds.
    fn windows(
        &mut self,
        name: &str,
        from: u64,
        to: u64,
    ) -> Result<(String, Vec<UnitCore>), String> {
        let s = self
            .series
            .get(name)
            .ok_or_else(|| format!("no series `{name}`; `series(service)` lists them"))?;
        let to = to.min(CASE_S);
        if from >= to {
            return Err(format!("an empty window [{from}, {to})"));
        }
        let (uid, inst) = (s.uid, self.metric_inst);
        let (a, b) = (self.ms(from), self.ms(to));
        let exact =
            evidence::materialise(&self.sub, &uid, &inst, a, b).map_err(|e| e.to_string())?;
        let mut out = format!("{name} [{from}, {to}) s\n");
        let e = &exact.unit;
        let count = Self::print_value(e, "count", None);
        let eu = canonical_uid(e);
        out.push_str(&format!(
            "  exact  id={} count={count} min={} max={} sum={} mean={} first={} last={}\n",
            eu.canonical(),
            Self::print_value(e, "min", None),
            Self::print_value(e, "max", None),
            Self::print_value(e, "sum", None),
            Self::print_value(e, "mean", None),
            Self::print_value(e, "first", None),
            Self::print_value(e, "last", None),
        ));
        self.known.insert(
            eu,
            (
                Known::Window {
                    series: name.to_string(),
                    from,
                    to,
                    sketched: false,
                },
                e.clone(),
            ),
        );
        let mut units = vec![e.clone()];
        if count != "0" {
            let qs: Vec<Decimal> = QS
                .iter()
                .map(|q| Decimal::parse_token(q).unwrap())
                .collect();
            let alpha = Decimal::parse_token(ALPHA).unwrap();
            if let Ok(sk) = evidence::materialise_sketched(&self.sub, &uid, &inst, a, b, alpha, &qs)
            {
                let u = &sk.unit;
                let su = canonical_uid(u);
                out.push_str(&format!(
                    "  sketch id={} q[0]={} (p50) q[1]={} (p90) q[2]={} (p99) alpha={ALPHA}\n",
                    su.canonical(),
                    Self::print_value(u, "q", Some(0)),
                    Self::print_value(u, "q", Some(1)),
                    Self::print_value(u, "q", Some(2)),
                ));
                self.known.insert(
                    su,
                    (
                        Known::Window {
                            series: name.to_string(),
                            from,
                            to,
                            sketched: true,
                        },
                        u.clone(),
                    ),
                );
                units.push(u.clone());
            }
        }
        Ok((out, units))
    }

    pub fn tool_services(&self) -> String {
        let mut out = format!("{} services:\n", self.anon.from.len());
        for a in self.services() {
            let n = self
                .series
                .keys()
                .filter(|k| k.split('/').next() == Some(a.as_str()) && k.matches('/').count() == 1)
                .count();
            let ops = self.ops.get(&a).map_or(0, BTreeMap::len);
            out.push_str(&format!(
                "  {a}: {n} metric series, logs {}, traced operations {ops}\n",
                if self.streams.contains_key(&a) {
                    "yes"
                } else {
                    "no"
                }
            ));
        }
        out
    }

    pub fn tool_series(&self, service: &str) -> Result<String, String> {
        if !self.anon.from.contains_key(service) {
            return Err(format!("no service `{service}`"));
        }
        let names: Vec<&str> = self
            .series
            .keys()
            .filter(|k| k.split('/').next() == Some(service) && k.matches('/').count() == 1)
            .map(String::as_str)
            .collect();
        let mut out = format!("{service}: {}\n", names.join(", "));
        if let Some(ops) = self.ops.get(service) {
            out.push_str(&format!(
                "  and {} traced operations; `traces` summarises them, and `window` reads \
                 {service}/opN/calls and {service}/opN/latency-ms\n",
                ops.len()
            ));
        }
        Ok(out)
    }

    pub fn tool_window(&mut self, series: &str, from: u64, to: u64) -> Result<String, String> {
        let (out, units) = self.windows(series, from, to)?;
        self.consult(units, Vec::new());
        Ok(out)
    }

    pub fn tool_compare(&mut self, series: &str, at: u64) -> Result<String, String> {
        let (a, ua) = self.windows(series, at.saturating_sub(HALF_S), at)?;
        let (b, ub) = self.windows(series, at, at + HALF_S)?;
        let mut units = ua;
        units.extend(ub);
        self.consult(units, Vec::new());
        Ok(format!("before:\n{a}after:\n{b}"))
    }

    pub fn tool_logs(&mut self, service: &str, from: u64, to: u64) -> Result<String, String> {
        let &(stream, inst) = self
            .streams
            .get(service)
            .ok_or_else(|| format!("{service} has no logs"))?;
        let to = to.min(CASE_S);
        if from >= to {
            return Err(format!("an empty window [{from}, {to})"));
        }
        let mut ts = logs::materialise(&self.sub, &stream, &inst, self.ms(from), self.ms(to))
            .map_err(|e| e.to_string())?;
        let lines: u64 = ts.iter().map(|t| t.count).sum();
        ts.sort_by(|a, b| b.count.cmp(&a.count).then(a.template.cmp(&b.template)));
        let mut out = format!(
            "{service} logs [{from}, {to}) s: {lines} lines, {} templates; top {} by count:\n",
            ts.len(),
            ts.len().min(20)
        );
        let mut units = Vec::new();
        for t in ts.iter().take(20) {
            let u = t.unit();
            let uid = canonical_uid(&u);
            let shown: String = t.template.chars().take(160).collect();
            out.push_str(&format!(
                "  id={} count={} {shown}{}\n",
                uid.canonical(),
                Self::print_value(&u, "count", None),
                if shown.len() < t.template.len() {
                    " …"
                } else {
                    ""
                }
            ));
            self.known.insert(
                uid,
                (
                    Known::Template {
                        service: service.to_string(),
                        from,
                        to,
                        template: t.template.clone(),
                    },
                    u.clone(),
                ),
            );
            units.push(u);
        }
        self.consult(units, Vec::new());
        Ok(out)
    }

    pub fn tool_traces(&mut self, service: &str, from: u64, to: u64) -> Result<String, String> {
        if self.raw.spans.is_none() {
            return Err("this system has no traces".into());
        }
        let ops = self
            .ops
            .get(service)
            .cloned()
            .ok_or_else(|| format!("{service} has no traced operations"))?;
        let mut rows = Vec::new();
        for (opn, name) in &ops {
            let (c, cu) = self.windows(&format!("{service}/{opn}/calls"), from, to)?;
            let calls: u64 = cu
                .first()
                .and_then(|u| u.payload.as_deref())
                .and_then(|p| tel::quoted_value(p, "sum", None))
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            rows.push((calls, opn.clone(), name.clone(), c, cu));
        }
        rows.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out = format!(
            "{service} traces [{from}, {to}) s: {} operations; top {} by calls:\n",
            rows.len(),
            rows.len().min(10)
        );
        let mut units = Vec::new();
        for (_, opn, name, c, cu) in rows.into_iter().take(10) {
            let (l, lu) = self.windows(&format!("{service}/{opn}/latency-ms"), from, to)?;
            out.push_str(&format!("  {opn} {name}\n"));
            for line in c.lines().chain(l.lines()).filter(|x| x.starts_with("  ")) {
                out.push_str(&format!("  {line}\n"));
            }
            units.extend(cu);
            units.extend(lu);
        }
        self.consult(units, Vec::new());
        Ok(out)
    }

    /// `hypothesise`: in (b) and (d) the unit, committed, and what rule TQ and kind 6 say of it;
    /// in (a) a note.
    pub fn tool_hypothesise(
        &mut self,
        service: &str,
        claim: &str,
        quotes: &[Quote],
        grounds: &[String],
    ) -> String {
        let mut ground_uids: Vec<Uid> = Vec::new();
        for id in quotes
            .iter()
            .map(|q| q.id.as_str())
            .chain(grounds.iter().map(String::as_str))
        {
            if let Ok(u) = id.parse::<Uid>() {
                if !ground_uids.contains(&u) {
                    ground_uids.push(u);
                }
            }
        }
        let n = self.written.len() + 1;
        let Some(ledger) = self.ledger.as_ref() else {
            self.written.push(Written {
                uid: None,
                quotes: quotes.to_vec(),
                tq: vec![None; quotes.len()],
                grounds: ground_uids,
            });
            return format!("noted as hypothesis {n}");
        };
        let alert = ledger.alert;
        let investigation = ledger.investigation;
        // Grounds the agent named that no tool returned are not units: they cannot be grounds.
        let grounds_ok: Vec<Uid> = ground_uids
            .iter()
            .copied()
            .filter(|u| self.known.contains_key(u))
            .collect();
        let qtext: Vec<String> = quotes.iter().map(Quote::text).collect();
        let text = format!("{service} is the root cause: {claim}");
        let u = inv::hypothesis(&text, &qtext, None, grounds_ok.clone());
        let hu = canonical_uid(&u);
        let model: BTreeSet<Uid> = [hu].into();
        let l = self.ledger.as_mut().expect("open");
        commit(
            l,
            vec![u.clone()],
            vec![inv::during(hu, investigation), inv::explains(hu, alert)],
            &model,
        );
        self.known.insert(hu, (Known::Hypothesis, u.clone()));
        // Rule TQ, per quote, as `smysl check` reads it.
        let store = l.lib.store();
        let payload_of =
            |x: &Uid| StoreRead::get(store, x).map(|c| c.core.payload.clone().unwrap_or_default());
        let failed = tel::rule_tq_violations(&u, payload_of);
        let tq: Vec<Option<String>> = qtext
            .iter()
            .map(|q| failed.iter().find(|(f, _)| f == q).map(|(_, w)| w.clone()))
            .collect();
        let status = tel::tq_capped(u.status, failed.len());
        // Kind 6: rival explanations of the alert that name this hypothesis.
        let units: Vec<(Uid, UnitCore)> = StoreRead::units(store)
            .map(|(x, v)| (*x, v.core.clone()))
            .collect();
        let rels: Vec<Relation> = store.relations().cloned().collect();
        let rivals = tel::rival_explanations(
            units.iter().map(|(a, b)| (a, b)),
            rels.iter(),
            |x| {
                units
                    .iter()
                    .find(|(a, _)| a == x)
                    .map_or(smysl::Status::Speculative, |(_, c)| c.status)
            },
            payload_of,
            Hlc::new(l.clock, 0, l.agent.clone()),
        );
        let named = rivals.iter().filter(|c| c.positions.contains(&hu)).count();
        let mut out = format!("hypothesis {n} id={} status={status}\n", hu.canonical());
        for (q, why) in qtext.iter().zip(&tq) {
            if let Some(w) = why {
                out.push_str(&format!("  SMY-W606 quote `{q}` does not verify: {w}\n"));
            }
        }
        if named > 0 {
            out.push_str(
                "  SMY-W053 another live hypothesis explains the same alert (kind 6, rival \
                 explanations)\n",
            );
        }
        if failed.is_empty() && !quotes.is_empty() {
            out.push_str("  every quote verifies\n");
        }
        self.written.push(Written {
            uid: Some(hu),
            quotes: quotes.to_vec(),
            tq,
            grounds: ground_uids,
        });
        out
    }

    /// Commit BARO's hypotheses in (d), before the agent's first step.
    pub fn commit_ranking(&mut self, units: &[UnitCore]) {
        for u in units {
            self.known
                .insert(canonical_uid(u), (Known::Ranked, u.clone()));
        }
        let Some(l) = self.ledger.as_mut() else {
            return;
        };
        let rels: Vec<Relation> = units
            .iter()
            .map(|u| inv::explains(canonical_uid(u), l.alert))
            .collect();
        commit(l, units.to_vec(), rels, &BTreeSet::new());
    }

    /// Units consulted (ids printed), and those any hypothesis cited: the unused-evidence rate's
    /// two sides, the same in every arm.
    pub fn evidence_use(&self) -> (usize, usize) {
        let consulted: BTreeSet<&Uid> = self
            .known
            .iter()
            .filter(|(_, (k, _))| matches!(k, Known::Window { .. } | Known::Template { .. }))
            .map(|(u, _)| u)
            .collect();
        let cited: BTreeSet<&Uid> = self
            .written
            .iter()
            .flat_map(|w| w.grounds.iter())
            .filter(|u| consulted.contains(u))
            .collect();
        (consulted.len(), cited.len())
    }
}

fn commit(l: &mut Ledger, units: Vec<UnitCore>, relations: Vec<Relation>, model: &BTreeSet<Uid>) {
    let units: Vec<UnitCore> = units
        .into_iter()
        .filter(|u| !l.committed.contains(&canonical_uid(u)))
        .collect();
    let mut tw = BTreeMap::new();
    for u in &units {
        let uid = canonical_uid(u);
        if u.schema.as_str() == tel::WINDOW {
            if let Some((_, op, rung)) = u
                .payload
                .as_deref()
                .and_then(|p| tel::window_fields(p).ok())
                .and_then(|f| tel::rule_tw(&f))
            {
                tw.insert(uid, (op, rung));
            }
        } else if u.schema.as_str() == tel::LOG_TEMPLATE || u.status == smysl::Status::Derived {
            tw.insert(uid, (Op::Transformed, Rung::Computed));
        }
    }
    l.clock += 1;
    let tool = tool_agent();
    let by = By {
        tool: &tool,
        model: &l.agent,
        model_units: model,
        tw,
        now: l.clock,
    };
    let uids: Vec<Uid> = units.iter().map(canonical_uid).collect();
    let staged = prepare_attested(
        l.lib.store(),
        units,
        relations,
        BTreeMap::new(),
        vec![tel::decl()],
        &by,
    );
    assert!(
        !staged.has_errors(),
        "a ledger batch did not check clean: {:?}",
        staged.report.diagnostics
    );
    l.lib.append_records(&staged.records()).expect("the append");
    l.committed.extend(uids);
}
