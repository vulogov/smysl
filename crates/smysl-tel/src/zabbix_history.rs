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

use crate::readers::zabbix_history::{HistoryRead, Item};
use crate::scrape::{payload, ScrapeError, Scraped, SeriesSamples};

/// The scheme every Zabbix series has.
pub const SCHEME: &str = "zabbix";

/// The instrument key of a Zabbix server (§7): `zabbix-server:<name>`.
pub fn instrument_key(server: &str) -> String {
    format!("zabbix-server:{server}")
}

/// Record a history read, taken from the Zabbix server keyed `instrument_key`.
pub fn record(read: &HistoryRead, instrument_key: &str) -> Result<Scraped, ScrapeError> {
    let unit_err = |e: tel::TelError| ScrapeError::Unit(e.to_string());
    let instrument = tel::instrument("collector", instrument_key).map_err(unit_err)?;
    let inst = canonical_uid(&instrument);
    let mut series = Vec::with_capacity(read.items.len());
    let mut units = Vec::with_capacity(read.items.len());
    for ih in &read.items {
        let key = ih.item.key();
        let unit = tel::series(SCHEME, &key).map_err(unit_err)?;
        let uid = canonical_uid(&unit);
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
    })
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
}
