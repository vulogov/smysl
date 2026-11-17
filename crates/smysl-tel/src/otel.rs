//! One OTLP metrics export, recorded: samples for the substrate, and the reset and stale units
//! it shows (RFC SMYSL-3 §3, §4.1, §9.1, §20.1, TL-P3).
//!
//! The same facts a Prometheus scrape establishes ([`crate::scrape`]), told differently by the
//! source:
//!
//! - **`x.tel/reset`, `start-time`, `measured`.** A cumulative count (a monotonic sum, a
//!   histogram's or summary's `count`) states the start of its accumulation in every point. While
//!   it accumulates, the start stays where it was, before the previous point; a point whose start
//!   is at or after the previous point's time began accumulating afresh since then. The unit's
//!   `at_ms` is the new start, and every sample of the point (each bucket, the sum) carries
//!   flag 2.
//! - **`x.tel/reset`, `decrease`, `derived`.** A cumulative count with no start time (`0`, which
//!   OTLP allows) that fell below its previous value. Grounded on the series unit.
//! - **`x.tel/stale`, `no-recorded-value`, `measured`.** A point flagged
//!   `FLAG_NO_RECORDED_VALUE`: the source states the series ended there. Its samples are stale.
//!
//! A delta sum's start moves with every point by definition, so delta counts never reset. A
//! gauge never does. Unlike a scrape, an export is pushed, not polled: a series absent from one
//! export is merely not in it, so absence is never staleness here.
//!
//! An export can hold several points of one series (a batch). They are taken in time order,
//! each against the one before it, the first against the substrate.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Decimal, Uid};
use smysl_series::{flags, Sample, SeriesInfo};

use crate::readers::otlp::{Export, Point};
use crate::scrape::{numeric_cmp, reset, stale, History, ScrapeError, Scraped, SeriesSamples};

/// The scheme every OTLP series has.
pub const SCHEME: &str = "otel";

/// Nanoseconds as the substrate's `(ts_ms, ns)`; `ns` only when it is not zero.
fn instant(t_ns: u64) -> (u64, Option<u32>) {
    let ns = (t_ns % 1_000_000) as u32;
    (t_ns / 1_000_000, (ns != 0).then_some(ns))
}

fn as_ns(s: &Sample) -> u128 {
    let (ms, ns) = s.instant();
    u128::from(ms) * 1_000_000 + u128::from(ns)
}

/// Record `export`, carried to us by the instrument `(role, instrument_key)`, after `history`.
pub fn record(
    export: &Export,
    role: &str,
    instrument_key: &str,
    history: &impl History,
) -> Result<Scraped, ScrapeError> {
    let unit_err = |e: smysl_core::schemas::tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument(role, instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);

    let points = export.points();
    let mut series: BTreeMap<Uid, SeriesSamples> = BTreeMap::new();
    let mut uids = Vec::with_capacity(points.len());
    for p in &points {
        let unit = tel::series(SCHEME, &p.key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
        series.entry(uid).or_insert_with(|| SeriesSamples {
            uid,
            unit,
            info: SeriesInfo {
                scheme: SCHEME.into(),
                key: p.key.clone(),
                class: p.class.into(),
            },
            samples: Vec::new(),
        });
        uids.push(uid);
    }

    // Each data point: its points, and its headline's index.
    let mut groups: BTreeMap<usize, (Vec<usize>, Option<usize>)> = BTreeMap::new();
    for (i, p) in points.iter().enumerate() {
        let g = groups.entry(p.group).or_default();
        g.0.push(i);
        if p.headline && g.1.is_none() {
            g.1 = Some(i);
        }
    }
    // In time order per headline series, so each point is judged against the one before it.
    let mut order: Vec<(Uid, u64, usize)> = groups
        .iter()
        .filter_map(|(g, (_, h))| h.map(|h| (uids[h], points[h].time_ns, *g)))
        .collect();
    order.sort_unstable();

    let mut units = Vec::new();
    let mut last: BTreeMap<Uid, Sample> = BTreeMap::new();
    for (huid, _, g) in order {
        let (members, h) = &groups[&g];
        let h = h.expect("ordered groups have a headline");
        let head: &Point = &points[h];
        let (ts_ms, _) = instant(head.time_ns);
        let prev = match last.get(&huid) {
            Some(s) => Some(*s),
            None => history.before(&huid, ts_ms)?,
        };
        let key = &series[&huid].info.key;

        let mut extra = 0;
        if head.stale {
            units.push(stale(SCHEME, key, &huid, ts_ms, "no-recorded-value"));
        } else if let (true, Some(now), Some(prev)) =
            (head.resets, head.value, prev.filter(Sample::is_counted))
        {
            if head.start_ns != 0 {
                if u128::from(head.start_ns) >= as_ns(&prev) {
                    extra = flags::RESET;
                    let at = head.start_ns / 1_000_000;
                    units.push(reset(SCHEME, key, &huid, at, ts_ms, "start-time"));
                }
            } else if numeric_cmp(&now, &prev.value_dec()) == Ordering::Less {
                units.push(reset(SCHEME, key, &huid, ts_ms, ts_ms, "decrease"));
            }
        }

        for &i in members {
            let p = &points[i];
            let (ts_ms, ns) = instant(p.time_ns);
            let (value, f) = match (p.stale, p.value) {
                (true, _) => (Decimal::ZERO, flags::STALE),
                (false, Some(v)) => (v, extra),
                (false, None) => (Decimal::ZERO, flags::NO_VALUE | extra),
            };
            let s = Sample::new(ts_ms, ns, value, f).expect("ns below 10^6 and known flags");
            if i == h {
                last.insert(huid, s);
            }
            series
                .get_mut(&uids[i])
                .expect("every point has its series")
                .samples
                .push(s);
        }
    }

    Ok(Scraped {
        instrument,
        inst,
        series: series.into_values().collect(),
        units,
        not_values: export.not_values(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readers::otlp::read_json;
    use crate::scrape::SubstrateHistory;
    use smysl_series::{MemBackend, Substrate};

    const MS: u64 = 1_000_000;
    const T0: u64 = 1_768_557_600_000 * MS;

    fn sum(points: &str) -> String {
        format!(
            r#"{{"resourceMetrics":[{{"scopeMetrics":[{{"metrics":[{{"name":"req","sum":{{"aggregationTemporality":2,"isMonotonic":true,"dataPoints":[{points}]}}}}]}}]}}]}}"#
        )
    }

    fn pt(start: u64, t: u64, v: i64) -> String {
        format!(r#"{{"startTimeUnixNano":"{start}","timeUnixNano":"{t}","asInt":"{v}"}}"#)
    }

    fn rec(sub: &mut Substrate<MemBackend>, text: &str) -> Scraped {
        let e = read_json(text).unwrap();
        let key = "otel-collector:c1";
        let inst = canonical_uid(&tel::instrument("collector", key).unwrap());
        let h = SubstrateHistory {
            substrate: sub,
            instrument: inst,
            scheme: SCHEME,
        };
        let r = record(&e, "collector", key, &h).unwrap();
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
    fn a_new_start_time_is_a_measured_reset_with_flag_2() {
        let mut sub = Substrate::new(MemBackend::default());
        let r = rec(&mut sub, &sum(&pt(T0 - 3_600_000 * MS, T0, 100)));
        assert!(r.units.is_empty(), "first seen is not a reset");
        let r = rec(
            &mut sub,
            &sum(&pt(T0 - 3_600_000 * MS, T0 + 15_000 * MS, 120)),
        );
        assert!(r.units.is_empty(), "same start: accumulating");
        let r = rec(&mut sub, &sum(&pt(T0 + 20_000 * MS, T0 + 30_000 * MS, 3)));
        assert_eq!(kinds(&r), [("x.tel/reset".into(), "measured".into())]);
        assert_eq!(r.series[0].samples[0].flags(), flags::RESET);
    }

    #[test]
    fn a_decrease_without_a_start_time_is_a_derived_reset() {
        let mut sub = Substrate::new(MemBackend::default());
        rec(&mut sub, &sum(&pt(0, T0, 50)));
        let r = rec(&mut sub, &sum(&pt(0, T0 + 15_000 * MS, 10)));
        assert_eq!(kinds(&r), [("x.tel/reset".into(), "derived".into())]);
        assert_eq!(r.series[0].samples[0].flags(), 0, "nobody stated a start");
        assert!(!r.units[0].grounds.is_empty());
    }

    #[test]
    fn points_in_one_export_are_judged_in_time_order() {
        let mut sub = Substrate::new(MemBackend::default());
        // Out of order in the file; the reset is between the second and third by time.
        let r = rec(
            &mut sub,
            &sum(&[pt(0, T0 + 2 * MS, 1), pt(0, T0, 5), pt(0, T0 + MS, 6)].join(",")),
        );
        assert_eq!(kinds(&r), [("x.tel/reset".into(), "derived".into())]);
    }

    #[test]
    fn no_recorded_value_is_a_measured_stale() {
        let mut sub = Substrate::new(MemBackend::default());
        rec(&mut sub, &sum(&pt(1, T0, 5)));
        let r = rec(
            &mut sub,
            &sum(&format!(
                r#"{{"startTimeUnixNano":"1","timeUnixNano":"{}","flags":1}}"#,
                T0 + 15_000 * MS
            )),
        );
        assert_eq!(kinds(&r), [("x.tel/stale".into(), "measured".into())]);
        assert_eq!(r.series[0].samples[0].flags(), flags::STALE);
        // Back after the stale: no reset, whatever the start says.
        let r = rec(&mut sub, &sum(&pt(T0 + 20_000 * MS, T0 + 30_000 * MS, 1)));
        assert!(r.units.is_empty());
    }

    #[test]
    fn delta_sums_and_gauges_never_reset() {
        let mut sub = Substrate::new(MemBackend::default());
        let delta = |s: u64, t: u64, v: i64| {
            sum(&pt(s, t, v)).replace(
                r#""aggregationTemporality":2"#,
                r#""aggregationTemporality":1"#,
            )
        };
        rec(&mut sub, &delta(T0 - MS, T0, 50));
        let r = rec(&mut sub, &delta(T0, T0 + MS, 1));
        assert!(r.units.is_empty());
    }

    #[test]
    fn sub_millisecond_time_is_kept() {
        let mut sub = Substrate::new(MemBackend::default());
        let r = rec(&mut sub, &sum(&pt(0, T0 + 123_456, 1)));
        assert_eq!(r.series[0].samples[0].instant(), (T0 / MS, 123_456));
    }
}
