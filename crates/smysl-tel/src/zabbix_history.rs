//! Zabbix history, recorded: samples for the substrate, and each item's `x.tel/series-meta`
//! (RFC SMYSL-3 §6, §9.1, §20.1, TL-P3).
//!
//! What [`crate::readers::zabbix_history`] read becomes, per numeric item:
//! - its `x.tel/series` (scheme `zabbix`, key `host:<host>/item:<key_>`), class `gauge`;
//! - its rows as samples, a value outside `dec`'s range with flag 4;
//! - an `x.tel/series-meta`, `cited`: `tel:of` the series, `tel:native_id` the `itemid`,
//!   `tel:text` the item's name. The `itemid` is metadata and never identity (§6), so an item
//!   re-imported under a new `itemid` keeps its series and gains a second meta unit.
//!
//! No reset and no stale: Zabbix states neither (see the reader's module comment).
//!
//! The instrument is the Zabbix server the history was read from, role `collector`, keyed
//! `zabbix-server:<name>` (§7). The name is the caller's: an export does not carry it.

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, Decimal, SchemaId, SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder,
};
use smysl_series::{flags, Sample, SeriesInfo};

use crate::evidence::{Basis, Summary, Window};
use crate::readers::zabbix_history::{HistoryRead, Item, TrendsRead, TREND_MS};
use crate::scrape::{payload, Lineage, ScrapeError, Scraped, SeriesSamples};

/// The scheme every Zabbix series has.
pub const SCHEME: &str = "zabbix";

/// The instrument key of a Zabbix server (§7): `zabbix-server:<name>`.
pub fn instrument_key(server: &str) -> String {
    format!("zabbix-server:{server}")
}

/// The origin key of a Zabbix host (§7): `zabbix-agent:<host>`, role `agent`. The host's agent
/// measured the item; the server is how it reached us.
pub fn origin_key(host: &str) -> String {
    format!("zabbix-agent:{host}")
}

/// Record a history read, taken from the Zabbix server keyed `instrument_key`.
pub fn record(read: &HistoryRead, instrument_key: &str) -> Result<Scraped, ScrapeError> {
    let unit_err = |e: tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument("collector", instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);
    let mut series = Vec::with_capacity(read.items.len());
    let mut units = Vec::with_capacity(read.items.len());
    let mut lineage = Lineage::default();
    for ih in &read.items {
        let key = ih.item.key();
        let unit = tel::series(SCHEME, &key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
        lineage.add(uid, "agent", &origin_key(&ih.item.host), inst)?;
        units.push(meta(&key, &uid.canonical(), &ih.item));
        let samples = ih
            .rows
            .iter()
            .map(|r| {
                let (v, f) = match r.value {
                    Some(v) => (v, 0),
                    None => (Decimal::ZERO, flags::NO_VALUE),
                };
                Sample::new(r.ts_ms, r.ns, v, f).expect("ns below 10^6 and known flags")
            })
            .collect();
        series.push(SeriesSamples {
            uid,
            unit,
            info: SeriesInfo {
                scheme: SCHEME.into(),
                key,
                class: "gauge".into(),
            },
            samples,
        });
    }
    series.sort_by_key(|s| s.uid);
    Ok(Scraped {
        instrument,
        inst,
        series,
        units,
        not_values: read.not_values,
        lineage: {
            lineage.settle();
            lineage
        },
    })
}

/// The recipe a Zabbix trend window records: what computed it, which is Zabbix.
pub const TREND_RECIPE: &str = "zabbix/trend.get";

/// What a trends read became.
#[derive(Debug, Clone)]
pub struct Trended {
    pub instrument: UnitCore,
    pub inst: smysl_core::Uid,
    /// Each item's series unit and series-meta, in `itemid` order.
    pub series: Vec<UnitCore>,
    pub metas: Vec<UnitCore>,
    /// One `cited` platform window per trend row (§9.4, §20.1).
    pub windows: Vec<Window>,
    pub not_values: usize,
    /// Each item's series measured by its host's agent, via the server (§7).
    pub lineage: Lineage,
}

/// Record a trends read from the Zabbix server keyed `instrument_key` as platform windows.
///
/// Each hour is a `cited` `x.tel/window` with `basis: platform`: `count` is Zabbix's `num`,
/// `min`, `max` and `mean` its `value_min`, `value_max` and `value_avg`, and there is no digest
/// and no sum. Zabbix computed these, not us, from samples we may never have had; rule TW
/// (§9.4) makes them a document, transcribed.
pub fn record_trends(read: &TrendsRead, instrument_key: &str) -> Result<Trended, ScrapeError> {
    let unit_err = |e: tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument("collector", instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);
    let mut out = Trended {
        instrument,
        inst,
        series: Vec::new(),
        metas: Vec::new(),
        windows: Vec::new(),
        not_values: read.not_values,
        lineage: Lineage::default(),
    };
    for it in &read.items {
        let key = it.item.key();
        let unit = tel::series(SCHEME, &key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
        out.lineage
            .add(uid, "agent", &origin_key(&it.item.host), inst)?;
        out.metas.push(meta(&key, &uid.canonical(), &it.item));
        out.series.push(unit);
        for r in &it.rows {
            out.windows.push(Window {
                series: uid,
                scheme: SCHEME.into(),
                key: key.clone(),
                instrument: inst,
                from_ms: r.from_ms,
                to_ms: r.from_ms + TREND_MS,
                recipe: TREND_RECIPE.into(),
                basis: Basis::Platform,
                digest: None,
                summary: Summary {
                    count: r.num,
                    min: r.min,
                    max: r.max,
                    mean: r.avg,
                    ..Summary::default()
                },
                sketch: None,
            });
        }
    }
    out.lineage.settle();
    Ok(out)
}

fn meta(key: &str, series: &str, item: &Item) -> UnitCore {
    let mut fields = vec![
        ("tel:of", HValue::Str(series.to_string())),
        ("tel:native_id", HValue::Str(item.itemid.clone())),
    ];
    if !item.name.is_empty() {
        fields.push(("tel:text", HValue::Str(item.name.clone())));
    }
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::SERIES_META.to_string()),
        tel::gist_of(&format!("{key} is itemid {}", item.itemid)),
        Status::Cited,
    )
    .source(SourceRef::new(
        SourceKind::Metric,
        format!("{}{SCHEME}:{key}", tel::SERIES_REF),
    ))
    .payload(payload(&fields))
    .build()
    .expect("a series-meta unit is well formed by construction")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readers::zabbix_history;

    const ITEMS: &str = r#"[
      {"itemid":"42","key_":"cpu","name":"CPU","value_type":"0","hosts":[{"host":"web-01"}]}]"#;

    #[test]
    fn an_itemid_is_metadata_and_never_identity() {
        let h = r#"[{"itemid":"42","clock":"1","value":"1"}]"#;
        let a = record(
            &zabbix_history::read(h, ITEMS, "7.0").unwrap(),
            "zabbix-server:z",
        )
        .unwrap();
        let reimported = ITEMS.replace("\"42\"", "\"77\"");
        let b = record(
            &zabbix_history::read(&h.replace("\"42\"", "\"77\""), &reimported, "7.0").unwrap(),
            "zabbix-server:z",
        )
        .unwrap();
        assert_eq!(a.series[0].uid, b.series[0].uid, "one series");
        assert_ne!(
            canonical_uid(&a.units[0]),
            canonical_uid(&b.units[0]),
            "two metas"
        );
        assert_eq!(a.units[0].status, Status::Cited);
        assert_eq!(a.series[0].samples[0].instant(), (1000, 0));
    }

    #[test]
    fn the_same_export_records_the_same() {
        let h = r#"[{"itemid":"42","clock":"1","value":"1"},{"itemid":"42","clock":"2","value":"1E+999"}]"#;
        let r = zabbix_history::read(h, ITEMS, "6.0").unwrap();
        let a = record(&r, "zabbix-server:z").unwrap();
        let b = record(&r, "zabbix-server:z").unwrap();
        assert_eq!(a.units, b.units);
        assert_eq!(a.series[0].samples, b.series[0].samples);
        assert_eq!(a.series[0].samples[1].flags(), flags::NO_VALUE);
        assert_eq!(a.not_values, 1);
    }

    #[test]
    fn a_trend_is_a_cited_platform_window_by_rule_tw() {
        use crate::readers::zabbix_history::read_trends;
        let trends = r#"[{"itemid":"42","clock":"3600","num":"60","value_min":"0.1",
            "value_avg":"0.4250","value_max":"1.5"}]"#;
        let r = read_trends(trends, ITEMS, "7.0").unwrap();
        let t = record_trends(&r, "zabbix-server:z").unwrap();
        assert_eq!(t.windows.len(), 1);
        let w = &t.windows[0];
        assert_eq!(
            (w.from_ms, w.to_ms, w.summary.count),
            (3_600_000, 7_200_000, 60)
        );
        assert_eq!(w.summary.mean, Some(Decimal::parse_token("0.425").unwrap()));
        assert_eq!(
            w.summary.sum, None,
            "Zabbix gives no sum, and avg x num is not one"
        );
        let u = w.unit();
        assert_eq!(u.status, Status::Cited);
        let fields = tel::window_fields(u.payload.as_deref().unwrap()).unwrap();
        assert_eq!(fields.basis.as_deref(), Some("platform"));
        assert!(fields.digest.is_none());
        let att = smysl_core::Attestation::new(
            canonical_uid(&u),
            smysl_core::AgentId::new("tool:smysl-tel").unwrap(),
            smysl_core::Op::Imported,
            smysl_core::Rung::Document,
            smysl_core::Hlc::new(1, 0, smysl_core::AgentId::new("tool:smysl-tel").unwrap()),
        );
        assert!(tel::rule_tw_violations(&u, &[att]).is_empty());

        // The series is the history's series: one item, one series, both kinds of evidence.
        let h = zabbix_history::read(
            r#"[{"itemid":"42","clock":"3600","value":"1"}]"#,
            ITEMS,
            "7.0",
        )
        .unwrap();
        let rec = record(&h, "zabbix-server:z").unwrap();
        assert_eq!(rec.series[0].uid, w.series);

        for bad in [
            r#"[{"itemid":"42","clock":"1","num":"1","value_min":"x","value_avg":"1","value_max":"1"}]"#,
            r#"[{"itemid":"42","clock":"1","value_min":"1","value_avg":"1","value_max":"1"}]"#,
            r#"[{"itemid":"42","clock":"1","num":"1","value_avg":"1","value_max":"1"}]"#,
        ] {
            assert!(read_trends(bad, ITEMS, "7.0").is_err(), "{bad}");
        }
    }
}
