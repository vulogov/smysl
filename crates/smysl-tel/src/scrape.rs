//! One scrape, recorded: samples for the substrate, and the reset and stale units the scrape
//! shows (RFC SMYSL-3 §3, §4.1, §9.1, §20.1, TL-P3).
//!
//! **Samples are not units** (§3, D-T1). Every sample goes to the substrate, and the units are
//! the few facts a scrape establishes about a series rather than a value of it:
//!
//! - **`x.tel/reset`**, a counter reset. Two ways of knowing it, two statuses:
//!   - **`start-time`**: the family's `_created` sample changed since the last scrape. The
//!     source states it, so the unit is `measured`, its `at_ms` is the new start time, and the
//!     counter's samples in this scrape carry flag 2.
//!   - **`decrease`**: no `_created`, and the counter is below its last value. We inferred it, so
//!     the unit is `derived`, grounded on the series unit, at the sample's instant. No flag:
//!     flag 2 is a start-time change (§9.1), which nobody stated.
//!
//!   A reset is one unit per counter, on its headline series (a counter's own samples, a
//!   histogram's or summary's `_count`), not one per bucket: the buckets reset with it, and the
//!   substrate holds them.
//! - **`x.tel/stale`**: a series this instrument served at its last scrape is absent from this
//!   one. That is exactly when Prometheus writes its stale NaN, so the marker is `stale-nan`,
//!   and the substrate gets a stale sample at the scrape instant.
//!
//! **What came before** is a [`History`]: the substrate as this instrument left it. A scrape is
//! therefore recorded against what the store holds, never against a cache of the last run.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::{
    canonical_uid, Decimal, RelKind, Relation, SchemaId, SourceKind, SourceRef, Status, Uid,
    UnitCore, UnitCoreBuilder,
};
use smysl_series::{flags, Backend, Sample, SeriesInfo, Substrate};

use crate::readers::prom::{PromSample, Scrape};

/// The scheme every `prom/1` series has.
pub const SCHEME: &str = "prom";

/// What the substrate held before this scrape, for one instrument.
pub trait History {
    /// The last sample of `series` before `t_ms`.
    fn before(&self, series: &Uid, t_ms: u64) -> Result<Option<Sample>, smysl_series::Error>;
    /// The series whose last sample before `t_ms` is counted (not stale), with their keys.
    fn live(&self, t_ms: u64) -> Result<Vec<(Uid, String)>, smysl_series::Error>;
}

/// Two histories read as one: what the store holds, and what this run has recorded and not yet
/// written. A review that writes nothing (`tel read` without `--yes`) still reads each file
/// after the ones before it, so its units are the ones the writing run will commit.
pub struct Layered<A, B> {
    pub base: Option<A>,
    pub over: B,
}

impl<A: History, B: History> History for Layered<A, B> {
    fn before(&self, series: &Uid, t_ms: u64) -> Result<Option<Sample>, smysl_series::Error> {
        let over = self.over.before(series, t_ms)?;
        let base = match &self.base {
            Some(b) => b.before(series, t_ms)?,
            None => None,
        };
        // The later sample is the one in force; at one instant the sample order decides.
        Ok(over.max(base))
    }

    fn live(&self, t_ms: u64) -> Result<Vec<(Uid, String)>, smysl_series::Error> {
        let mut keys: BTreeMap<Uid, String> = self.over.live(t_ms)?.into_iter().collect();
        if let Some(b) = &self.base {
            keys.extend(b.live(t_ms)?);
        }
        // A series live in one layer may have gone stale in the other since.
        let mut out = Vec::new();
        for (uid, key) in keys {
            if self.before(&uid, t_ms)?.is_some_and(|s| s.is_counted()) {
                out.push((uid, key));
            }
        }
        Ok(out)
    }
}

/// The `x.tel/of-series` relations (§4.2) of `units`: each reset and stale to its series, read
/// from the unit's own `tel:series`.
pub fn of_series(units: &[UnitCore]) -> Vec<Relation> {
    let mut out = Vec::new();
    for u in units {
        if ![tel::RESET, tel::STALE].contains(&u.schema.as_str()) {
            continue;
        }
        let series = u
            .payload
            .as_deref()
            .and_then(|p| smysl_core::surface::payload_to_object(p).ok())
            .and_then(|o| match o.get("tel:series").map(|v| &v.value) {
                Some(HValue::Str(s)) => s.parse::<Uid>().ok(),
                _ => None,
            });
        if let Some(series) = series {
            out.push(Relation::new(
                RelKind::Extension(tel::OF_SERIES.to_string()),
                canonical_uid(u),
                series,
            ));
        }
    }
    out
}

/// A substrate's history for one instrument and scheme.
pub struct SubstrateHistory<'a, B> {
    pub substrate: &'a Substrate<B>,
    pub instrument: Uid,
    pub scheme: &'a str,
}

impl<B: Backend> History for SubstrateHistory<'_, B> {
    fn before(&self, series: &Uid, t_ms: u64) -> Result<Option<Sample>, smysl_series::Error> {
        match t_ms.checked_sub(1) {
            Some(t) => self.substrate.at(series, &self.instrument, t),
            None => Ok(None),
        }
    }

    fn live(&self, t_ms: u64) -> Result<Vec<(Uid, String)>, smysl_series::Error> {
        let mut out = Vec::new();
        for (uid, row) in self.substrate.series(Some(self.scheme))? {
            if self.before(&uid, t_ms)?.is_some_and(|s| s.is_counted()) {
                out.push((uid, row.key));
            }
        }
        Ok(out)
    }
}

/// One series' samples from a scrape.
#[derive(Debug, Clone)]
pub struct SeriesSamples {
    pub uid: Uid,
    /// Its `x.tel/series` unit.
    pub unit: UnitCore,
    pub info: SeriesInfo,
    /// In the order the scrape had them.
    pub samples: Vec<Sample>,
}

/// What one scrape became.
#[derive(Debug, Clone)]
pub struct Scraped {
    /// The `x.tel/instrument` unit of the scraper, and its uid.
    pub instrument: UnitCore,
    pub inst: Uid,
    /// Every series the scrape had, and every series it went stale on, in uid order.
    pub series: Vec<SeriesSamples>,
    /// Resets and stales.
    pub units: Vec<UnitCore>,
    /// Samples whose value was `NaN` or `±Inf`, kept with flag 4.
    pub not_values: usize,
}

/// Why a scrape could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrapeError {
    Unit(String),
    Series(smysl_series::Error),
}

impl core::fmt::Display for ScrapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ScrapeError::Unit(why) => write!(f, "{why}"),
            ScrapeError::Series(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ScrapeError {}

impl From<smysl_series::Error> for ScrapeError {
    fn from(e: smysl_series::Error) -> ScrapeError {
        ScrapeError::Series(e)
    }
}

/// The instrument key of a Prometheus scrape (§7): `prom-scrape:<job>/<instance>`.
pub fn instrument_key(job: &str, instance: &str) -> String {
    format!("prom-scrape:{job}/{instance}")
}

/// Record `scrape`, taken at `at_ms` by the instrument `instrument_key`, after `history`.
pub fn record(
    scrape: &Scrape,
    at_ms: u64,
    instrument_key: &str,
    history: &impl History,
) -> Result<Scraped, ScrapeError> {
    let instrument =
        tel::instrument("scraper", instrument_key).map_err(|e| ScrapeError::Unit(e.to_string()))?;
    let inst = canonical_uid(&instrument);

    // The series and their samples.
    let mut series: BTreeMap<Uid, SeriesSamples> = BTreeMap::new();
    let mut of_sample: Vec<Uid> = Vec::with_capacity(scrape.samples.len());
    for s in &scrape.samples {
        let key = s.key();
        let unit = tel::series(SCHEME, &key).map_err(|e| ScrapeError::Unit(e.to_string()))?;
        let uid = canonical_uid(&unit);
        let (ts_ms, ns) = s.ts.unwrap_or((at_ms, None));
        let (value, f) = match s.value {
            Some(v) => (v, 0),
            None => (Decimal::ZERO, flags::NO_VALUE),
        };
        let sample = Sample::new(ts_ms, ns, value, f).expect("ns below 10^6 and known flags");
        series
            .entry(uid)
            .or_insert_with(|| SeriesSamples {
                uid,
                unit,
                info: SeriesInfo {
                    scheme: SCHEME.into(),
                    key: key.clone(),
                    class: s.ty.class().into(),
                },
                samples: Vec::new(),
            })
            .samples
            .push(sample);
        of_sample.push(uid);
    }

    let mut units = Vec::new();

    // Start-time resets: a group whose `_created` moved.
    let group = |s: &PromSample| (s.family.clone(), s.group_labels());
    let mut created: BTreeMap<(String, Vec<(String, String)>), bool> = BTreeMap::new();
    for (s, uid) in scrape.samples.iter().zip(&of_sample) {
        if s.suffix != "_created" {
            continue;
        }
        let Some(now) = s.value else { continue };
        let prev = history.before(uid, at_ms)?;
        let moved = prev.is_some_and(|p| p.is_counted() && p.value_dec() != now);
        let g = group(s);
        *created.entry(g.clone()).or_default() |= moved;
        if !moved {
            continue;
        }
        let start_ms = seconds_to_ms(now).unwrap_or(at_ms);
        for (t, tuid) in scrape.samples.iter().zip(&of_sample) {
            if !t.is_monotonic() || group(t) != g {
                continue;
            }
            let ss = series.get_mut(tuid).expect("every sample has its series");
            for sample in &mut ss.samples {
                *sample = Sample::new(
                    sample.ts_ms(),
                    sample.ns(),
                    sample.value_dec(),
                    sample.flags() | flags::RESET,
                )
                .expect("the flags it had, and one more");
            }
            if is_headline(t) {
                units.push(reset(
                    SCHEME,
                    &ss.info.key,
                    tuid,
                    start_ms,
                    at_ms,
                    "start-time",
                ));
            }
        }
    }

    // Decreases, where no `_created` speaks for the counter.
    for (s, uid) in scrape.samples.iter().zip(&of_sample) {
        if !s.is_monotonic() || !is_headline(s) || created.contains_key(&group(s)) {
            continue;
        }
        let Some(now) = s.value else { continue };
        let ts = s.ts.map_or(at_ms, |t| t.0);
        if let Some(prev) = history.before(uid, ts)? {
            if prev.is_counted() && numeric_cmp(&now, &prev.value_dec()) == Ordering::Less {
                units.push(reset(
                    SCHEME,
                    &series[uid].info.key,
                    uid,
                    ts,
                    at_ms,
                    "decrease",
                ));
            }
        }
    }

    // Stale: live before, absent now.
    let here: BTreeSet<Uid> = series.keys().copied().collect();
    for (uid, key) in history.live(at_ms)? {
        if here.contains(&uid) {
            continue;
        }
        let unit = tel::series(SCHEME, &key).map_err(|e| ScrapeError::Unit(e.to_string()))?;
        units.push(stale(SCHEME, &key, &uid, at_ms, "stale-nan"));
        series.insert(
            uid,
            SeriesSamples {
                uid,
                unit,
                info: SeriesInfo {
                    scheme: SCHEME.into(),
                    key,
                    class: String::new(),
                },
                samples: vec![Sample::stale(at_ms)],
            },
        );
    }

    Ok(Scraped {
        instrument,
        inst,
        series: series.into_values().collect(),
        units,
        not_values: scrape.not_values,
    })
}

/// The series a reset unit is written for: a counter's own samples, a histogram's or summary's
/// `_count`.
fn is_headline(s: &PromSample) -> bool {
    matches!(s.suffix, "" | "_total" | "_count") && s.is_monotonic()
}

/// Seconds (a `_created` value) as epoch ms, truncated; `None` if negative or beyond `u64`.
fn seconds_to_ms(v: Decimal) -> Option<u64> {
    if v.mantissa() < 0 {
        return None;
    }
    let mut m = v.mantissa();
    let e = i32::from(v.exponent()) + 3;
    if e >= 0 {
        for _ in 0..e {
            m = m.checked_mul(10)?;
        }
    } else {
        for _ in 0..-e {
            m /= 10;
        }
    }
    u64::try_from(m).ok()
}

/// Numeric order of two decimals, exactly: `Decimal`'s own `Ord` is not numeric.
pub fn numeric_cmp(a: &Decimal, b: &Decimal) -> Ordering {
    let sign = |d: &Decimal| d.mantissa().signum();
    match sign(a).cmp(&sign(b)) {
        Ordering::Equal => {}
        o => return o,
    }
    if sign(a) == 0 {
        return Ordering::Equal;
    }
    // Same sign: compare magnitudes by their leading digit's power, then digit by digit.
    let mag = |d: &Decimal| {
        let digits = d.mantissa().unsigned_abs().to_string();
        let lead = i32::from(d.exponent()) + digits.len() as i32 - 1;
        (lead, digits)
    };
    let ((la, da), (lb, db)) = (mag(a), mag(b));
    let by_mag = la.cmp(&lb).then_with(|| {
        let n = da.len().max(db.len());
        format!("{da:0<n$}").cmp(&format!("{db:0<n$}"))
    });
    if sign(a) < 0 {
        by_mag.reverse()
    } else {
        by_mag
    }
}

pub(crate) fn payload(entries: &[(&str, HValue)]) -> Vec<u8> {
    let span = smysl_core::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(
            Spanned::new((*k).to_string(), span),
            Spanned::new(v.clone(), span),
        );
    }
    smysl_core::surface::object_to_payload(&o).unwrap_or_default()
}

fn source(scheme: &str, key: &str, at_ms: u64) -> SourceRef {
    SourceRef::new(SourceKind::Metric, format!("series:{scheme}:{key}")).observed_at(at_ms)
}

pub(crate) fn ms(v: u64) -> HValue {
    // An epoch millisecond is far below i64::MAX for the next 292 million years.
    HValue::Int(i64::try_from(v).unwrap_or(i64::MAX))
}

/// An `x.tel/reset` of the series `key` of `scheme`, at `at`, seen at `observed`.
pub(crate) fn reset(
    scheme: &str,
    key: &str,
    series: &Uid,
    at: u64,
    observed: u64,
    basis: &str,
) -> UnitCore {
    let (status, grounds) = match basis {
        "start-time" => (Status::Measured, Vec::new()),
        _ => (Status::Derived, vec![*series]),
    };
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::RESET.to_string()),
        tel::gist_of(&format!("reset {key}")),
        status,
    )
    .source(source(scheme, key, observed))
    .grounds(grounds)
    .payload(payload(&[
        ("tel:series", HValue::Str(series.canonical())),
        ("tel:at_ms", ms(at)),
        ("tel:basis", HValue::Str(basis.into())),
    ]))
    .build()
    .expect("a reset unit is well formed by construction")
}

/// An `x.tel/stale` of the series `key` of `scheme` at `at`, as `marker` (§4.1) says.
pub(crate) fn stale(scheme: &str, key: &str, series: &Uid, at: u64, marker: &str) -> UnitCore {
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::STALE.to_string()),
        tel::gist_of(&format!("stale {key}")),
        Status::Measured,
    )
    .source(source(scheme, key, at))
    .payload(payload(&[
        ("tel:series", HValue::Str(series.canonical())),
        ("tel:at_ms", ms(at)),
        ("tel:marker", HValue::Str(marker.into())),
    ]))
    .build()
    .expect("a stale unit is well formed by construction")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readers::prom;
    use smysl_series::MemBackend;

    const T0: u64 = 1_768_557_600_000;
    const TARGET: &[(&str, &str)] = &[("job", "node"), ("instance", "h:9100")];

    fn d(s: &str) -> Decimal {
        Decimal::parse_token(s).unwrap()
    }

    /// Read, record against the substrate, and add the samples, as `tel read` will.
    fn scrape(sub: &mut Substrate<MemBackend>, text: &str, at: u64) -> Scraped {
        let s = prom::read(text, TARGET).unwrap();
        let key = instrument_key("node", "h:9100");
        let inst = canonical_uid(&tel::instrument("scraper", &key).unwrap());
        let h = SubstrateHistory {
            substrate: sub,
            instrument: inst,
            scheme: SCHEME,
        };
        let r = record(&s, at, &key, &h).unwrap();
        for ss in &r.series {
            sub.add(&ss.uid, &r.inst, &ss.info, &ss.samples).unwrap();
        }
        r
    }

    fn kinds(r: &Scraped) -> Vec<(String, String)> {
        r.units
            .iter()
            .map(|u| (u.schema.as_str().to_string(), u.status.as_str().to_string()))
            .collect()
    }

    #[test]
    fn numeric_order_is_value_order() {
        let cases = [
            ("1", "2"),
            ("0.5", "1"),
            ("-2", "-1"),
            ("-1", "0"),
            ("99", "100"),
            ("1.25", "1.3"),
            ("1e3", "1001"),
            ("-1e3", "-999"),
        ];
        for (a, b) in cases {
            assert_eq!(numeric_cmp(&d(a), &d(b)), Ordering::Less, "{a} < {b}");
            assert_eq!(numeric_cmp(&d(b), &d(a)), Ordering::Greater, "{b} > {a}");
        }
        assert_eq!(numeric_cmp(&d("41.0"), &d("41")), Ordering::Equal);
    }

    #[test]
    fn a_first_scrape_is_samples_and_no_units() {
        let mut sub = Substrate::new(MemBackend::default());
        let r = scrape(&mut sub, "# TYPE c counter\nc_total 5\nup 1\n", T0);
        assert!(r.units.is_empty());
        assert_eq!(r.series.len(), 2);
        let keys: BTreeSet<&str> = r.series.iter().map(|s| s.info.key.as_str()).collect();
        assert!(keys.contains(r#"c_total{instance="h:9100",job="node"}"#));
        assert_eq!(sub.series(Some("prom")).unwrap().len(), 2);
    }

    #[test]
    fn a_decrease_is_a_derived_reset_and_a_rise_is_nothing() {
        let mut sub = Substrate::new(MemBackend::default());
        scrape(&mut sub, "# TYPE c counter\nc_total 5\n", T0);
        assert!(
            scrape(&mut sub, "# TYPE c counter\nc_total 7\n", T0 + 15_000)
                .units
                .is_empty()
        );
        let r = scrape(&mut sub, "# TYPE c counter\nc_total 2\n", T0 + 30_000);
        assert_eq!(kinds(&r), [("x.tel/reset".into(), "derived".into())]);
        assert_eq!(r.units[0].grounds.len(), 1, "grounded on its series");
        assert!(r.units[0].grounds.contains(&r.series[0].uid));
        assert_eq!(
            r.series[0].samples[0].flags(),
            0,
            "flag 2 is a start-time's"
        );
        // A gauge falling is a gauge.
        scrape(&mut sub, "# TYPE g gauge\ng 5\n", T0 + 45_000);
        assert!(scrape(&mut sub, "# TYPE g gauge\ng 1\n", T0 + 60_000)
            .units
            .iter()
            .all(|u| u.schema.as_str() != "x.tel/reset"));
    }

    #[test]
    fn a_moved_created_is_a_measured_reset_with_its_start_time() {
        let om = |count: &str, created: &str| {
            format!(
                "# TYPE rq summary\nrq_count {count}\nrq_sum 1.5\nrq_created {created}\n# EOF\n"
            )
        };
        let mut sub = Substrate::new(MemBackend::default());
        scrape(&mut sub, &om("10", "1768557000"), T0);
        // Same start time, count rising: nothing.
        assert!(scrape(&mut sub, &om("12", "1768557000"), T0 + 15_000)
            .units
            .is_empty());
        let r = scrape(&mut sub, &om("1", "1768557620.5"), T0 + 30_000);
        assert_eq!(kinds(&r), [("x.tel/reset".into(), "measured".into())]);
        let p =
            smysl_core::surface::payload::payload_to_object(r.units[0].payload.as_ref().unwrap())
                .unwrap();
        assert_eq!(
            p.get("tel:at_ms").unwrap().value,
            HValue::Int(1_768_557_620_500)
        );
        assert_eq!(
            p.get("tel:basis").unwrap().value,
            HValue::Str("start-time".into())
        );
        let count = r
            .series
            .iter()
            .find(|s| s.info.key.starts_with("rq_count"))
            .unwrap();
        assert_eq!(count.samples[0].flags(), flags::RESET);
        let sum = r
            .series
            .iter()
            .find(|s| s.info.key.starts_with("rq_sum"))
            .unwrap();
        assert_eq!(sum.samples[0].flags(), 0, "_sum is not a counter");
    }

    #[test]
    fn an_absent_series_goes_stale_once() {
        let mut sub = Substrate::new(MemBackend::default());
        scrape(&mut sub, "a 1\nb 2\n", T0);
        let r = scrape(&mut sub, "a 1\n", T0 + 15_000);
        assert_eq!(kinds(&r), [("x.tel/stale".into(), "measured".into())]);
        let b = r
            .series
            .iter()
            .find(|s| s.info.key.starts_with("b{"))
            .unwrap();
        assert_eq!(b.samples, [Sample::stale(T0 + 15_000)]);
        // Stale now, so not live: no second stale unit.
        assert!(scrape(&mut sub, "a 1\n", T0 + 30_000).units.is_empty());
        // And back.
        assert!(scrape(&mut sub, "a 1\nb 3\n", T0 + 45_000).units.is_empty());
        let b_uid = b.uid;
        let inst = r.inst;
        assert_eq!(
            sub.at(&b_uid, &inst, T0 + 45_000).unwrap(),
            Some(Sample::value(T0 + 45_000, d("3")))
        );
    }

    #[test]
    fn not_a_value_is_flag_4() {
        let mut sub = Substrate::new(MemBackend::default());
        let r = scrape(&mut sub, "r NaN\n", T0);
        assert_eq!(r.not_values, 1);
        assert_eq!(r.series[0].samples[0].flags(), flags::NO_VALUE);
    }

    #[test]
    fn the_same_scrape_twice_is_the_same_units_and_no_new_samples() {
        let mut sub = Substrate::new(MemBackend::default());
        scrape(&mut sub, "# TYPE c counter\nc_total 5\nb 1\n", T0);
        let text = "# TYPE c counter\nc_total 2\n";
        let a = scrape(&mut sub, text, T0 + 15_000);
        let before = sub.window(&a.series[0].uid, &a.inst, 0, u64::MAX).unwrap();
        // Read again against a substrate that already holds it: the history now sees this
        // scrape's own samples, which are not *before* it, so it decides the same way.
        let b = scrape(&mut sub, text, T0 + 15_000);
        let ids = |r: &Scraped| r.units.iter().map(canonical_uid).collect::<Vec<_>>();
        assert_eq!(ids(&a), ids(&b));
        assert_eq!(
            sub.window(&a.series[0].uid, &a.inst, 0, u64::MAX).unwrap(),
            before
        );
    }

    #[test]
    fn a_layered_history_is_the_store_and_the_run_together() {
        // The store holds the first scrape; this run recorded the second and has not written it.
        let mut store = Substrate::new(MemBackend::default());
        scrape(&mut store, "# TYPE c counter\nc_total 5\ng 1\n", T0);
        let mut run = Substrate::new(MemBackend::default());
        let key = instrument_key("node", "h:9100");
        let inst = canonical_uid(&tel::instrument("scraper", &key).unwrap());
        let s2 = prom::read("# TYPE c counter\nc_total 7\n", TARGET).unwrap();
        let r2 = {
            let h = Layered {
                base: Some(SubstrateHistory {
                    substrate: &store,
                    instrument: inst,
                    scheme: SCHEME,
                }),
                over: SubstrateHistory {
                    substrate: &run,
                    instrument: inst,
                    scheme: SCHEME,
                },
            };
            record(&s2, T0 + 15_000, &key, &h).unwrap()
        };
        assert_eq!(
            kinds(&r2),
            [("x.tel/stale".into(), "measured".into())],
            "g went"
        );
        for ss in &r2.series {
            run.add(&ss.uid, &r2.inst, &ss.info, &ss.samples).unwrap();
        }
        // The third scrape sees the run's 7, not the store's 5, and g stale in the run.
        let s3 = prom::read("# TYPE c counter\nc_total 6\n", TARGET).unwrap();
        let h = Layered {
            base: Some(SubstrateHistory {
                substrate: &store,
                instrument: inst,
                scheme: SCHEME,
            }),
            over: SubstrateHistory {
                substrate: &run,
                instrument: inst,
                scheme: SCHEME,
            },
        };
        let r3 = record(&s3, T0 + 30_000, &key, &h).unwrap();
        assert_eq!(kinds(&r3), [("x.tel/reset".into(), "derived".into())]);
        let rels = of_series(&r3.units);
        assert_eq!(rels.len(), 1);
        assert_eq!(rels[0].from, canonical_uid(&r3.units[0]));
        assert_eq!(rels[0].to, r3.series[0].uid);
        assert_eq!(rels[0].kind.as_str(), tel::OF_SERIES);
    }
}
