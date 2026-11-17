//! `zabbix/1` history: `history.get`, read against the `item.get` it belongs to (RFC SMYSL-3
//! §6, §9.1, §20.1, TL-P3).
//!
//! **Two exports, one read.** A `history.get` row is `{itemid, clock, value, ns}`: it names its
//! item by `itemid`, which is not identity (§6, it changes when templates are re-imported), and
//! it does not say what type its value is. The item export (`item.get` with
//! `selectHosts: ["host"]`) supplies both: the series key `host:<host>/item:<key_>`, and the
//! item's `value_type`. So history is read together with its items, and the `itemid` becomes
//! `x.tel/series-meta`'s `native_id`.
//!
//! **What becomes a sample.** Numeric items only: `value_type` 0 (float) and 3 (unsigned). Their
//! values are decimal strings, read exactly (§5.3). Character, log, text and binary items
//! (1, 2, 4, 5) are not series in the substrate's sense; their rows are counted in
//! [`HistoryRead::not_numeric`] and left. A row whose `itemid` the item export does not have is
//! counted in [`HistoryRead::unknown_items`], by item, rather than refusing the whole export:
//! history is usually exported for a time range, items for a selection.
//!
//! **Time** is `clock` seconds and `ns` nanoseconds, kept to the nanosecond.
//!
//! **No units but metadata.** Zabbix states neither a start time nor a stale marker, and it does
//! not say whether an item counts up (a raw counter is an unsigned item like any other), so a
//! history export establishes no reset and no stale. Its series are `gauge`s. What it does
//! state about a series is cited in its `series-meta`.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{DecError, Decimal};

use crate::json::Json;
use crate::readers::zabbix::{item_host, require_version, results};
use crate::readers::{shape, text, ReadError};

/// An item, as the item export gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub itemid: String,
    pub host: String,
    pub key_: String,
    pub value_type: u8,
    /// The item's name, as Zabbix shows it.
    pub name: String,
    pub units: String,
}

impl Item {
    /// Its series key (§6).
    pub fn key(&self) -> String {
        format!("host:{}/item:{}", self.host, self.key_)
    }

    /// `value_type` 0 (float) or 3 (unsigned).
    pub fn is_numeric(&self) -> bool {
        matches!(self.value_type, 0 | 3)
    }
}

/// One history row of a numeric item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub ts_ms: u64,
    /// The nanoseconds below the millisecond, when not zero.
    pub ns: Option<u32>,
    /// `None`: a value outside `dec`'s range, kept with flag 4.
    pub value: Option<Decimal>,
}

/// One item's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemHistory {
    pub item: Item,
    /// In the export's order.
    pub rows: Vec<Row>,
}

/// What a history read produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryRead {
    /// By `itemid`, numeric items with at least one row.
    pub items: Vec<ItemHistory>,
    /// Rows of character, log, text and binary items.
    pub not_numeric: usize,
    /// `itemid`s with rows but absent from the item export.
    pub unknown_items: BTreeSet<String>,
    /// Values outside `dec`'s range.
    pub not_values: usize,
}

/// Read `history` (one or more `history.get` responses) against `items` (`item.get` with
/// `selectHosts: ["host"]`), both from an API of `api_version`.
pub fn read(history: &str, items: &str, api_version: &str) -> Result<HistoryRead, ReadError> {
    require_version(api_version)?;
    let mut by_id: BTreeMap<String, Item> = BTreeMap::new();
    for doc in results(items)? {
        let key_ = text(&doc, "key_");
        if key_.is_empty() {
            return Err(shape(
                "the item export has an object that is not an item (`key_`)",
            ));
        }
        let itemid = text(&doc, "itemid");
        if itemid.is_empty() {
            return Err(shape(format!("item `{key_}` has no `itemid`")));
        }
        let value_type = match text(&doc, "value_type") {
            "" => {
                return Err(shape(format!(
                    "item `{key_}` has no `value_type`; export items with `output` including it"
                )))
            }
            v => v
                .parse::<u8>()
                .ok()
                .filter(|v| *v <= 5)
                .ok_or_else(|| shape(format!("item `{key_}`: unknown value_type `{v}`")))?,
        };
        let item = Item {
            itemid: itemid.to_string(),
            host: item_host(&doc, key_)?.to_string(),
            key_: key_.to_string(),
            value_type,
            name: text(&doc, "name").to_string(),
            units: text(&doc, "units").to_string(),
        };
        if let Some(was) = by_id.insert(item.itemid.clone(), item.clone()) {
            if was != item {
                return Err(shape(format!(
                    "itemid {} is two different items in the item export",
                    item.itemid
                )));
            }
        }
    }

    let mut out = HistoryRead::default();
    let mut rows: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for doc in results(history)? {
        let itemid = text(&doc, "itemid");
        if itemid.is_empty() {
            return Err(shape("a history row without `itemid`"));
        }
        let Some(item) = by_id.get(itemid) else {
            out.unknown_items.insert(itemid.to_string());
            continue;
        };
        if !item.is_numeric() {
            out.not_numeric += 1;
            continue;
        }
        let clock = integer(&doc, "clock")?;
        let ns = match doc.get("ns") {
            None => 0,
            Some(_) => integer(&doc, "ns")?,
        };
        if ns >= 1_000_000_000 {
            return Err(shape(format!(
                "a history row of item {itemid}: ns {ns} is a second or more"
            )));
        }
        let ts_ms = clock
            .checked_mul(1000)
            .and_then(|ms| ms.checked_add(ns / 1_000_000))
            .ok_or_else(|| {
                shape(format!(
                    "a history row of item {itemid}: clock {clock} is out of range"
                ))
            })?;
        let sub = (ns % 1_000_000) as u32;
        let token = match doc.get("value") {
            Some(Json::String(s)) => s.as_str(),
            Some(Json::Number(t)) => t.as_str(),
            _ => {
                return Err(shape(format!(
                    "a history row of item {itemid} has no value"
                )))
            }
        };
        let value = match Decimal::parse_token(token) {
            Ok(d) => Some(d),
            Err(DecError::OutOfRange) => {
                out.not_values += 1;
                None
            }
            Err(_) => {
                return Err(shape(format!(
                    "a history row of numeric item {itemid}: `{token}` is not a number"
                )))
            }
        };
        rows.entry(itemid.to_string()).or_default().push(Row {
            ts_ms,
            ns: (sub != 0).then_some(sub),
            value,
        });
    }
    out.items = rows
        .into_iter()
        .map(|(id, rows)| ItemHistory {
            item: by_id[&id].clone(),
            rows,
        })
        .collect();
    Ok(out)
}

/// An unsigned integer, as Zabbix writes one: a string of digits (or, leniently, a number).
fn integer(doc: &Json, name: &str) -> Result<u64, ReadError> {
    let t = match doc.get(name) {
        Some(Json::String(s)) => s.as_str(),
        Some(Json::Number(t)) => t.as_str(),
        _ => return Err(shape(format!("a history row without `{name}`"))),
    };
    if t.is_empty() || !t.bytes().all(|b| b.is_ascii_digit()) {
        return Err(shape(format!("`{name}` {t:?} is not an unsigned integer")));
    }
    t.parse()
        .map_err(|_| shape(format!("`{name}` {t} is out of range")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEMS: &str = r#"{"jsonrpc":"2.0","id":1,"result":[
      {"itemid":"42","key_":"system.cpu.load[percpu,avg1]","name":"Load","value_type":"0","units":"",
       "hosts":[{"hostid":"10084","host":"web-01"}]},
      {"itemid":"43","key_":"net.if.in[eth0]","name":"In","value_type":"3","units":"bps",
       "hosts":[{"hostid":"10084","host":"web-01"}]},
      {"itemid":"44","key_":"agent.version","name":"Version","value_type":"1",
       "hosts":[{"hostid":"10084","host":"web-01"}]}]}"#;

    const HISTORY: &str = r#"[
      {"itemid":"42","clock":"1768557600","value":"0.5300","ns":"123456789"},
      {"itemid":"43","clock":"1768557600","value":"18446744073709551615","ns":"0"},
      {"itemid":"44","clock":"1768557600","value":"7.0.3","ns":"0"},
      {"itemid":"99","clock":"1768557600","value":"1","ns":"0"},
      {"itemid":"42","clock":"1768557660","value":"1E+500","ns":"0"}]"#;

    #[test]
    fn numeric_history_by_item_key_exactly_and_to_the_nanosecond() {
        let r = read(HISTORY, ITEMS, "7.0.3").unwrap();
        assert_eq!(r.items.len(), 2);
        let load = &r.items[0];
        assert_eq!(
            load.item.key(),
            "host:web-01/item:system.cpu.load[percpu,avg1]"
        );
        assert_eq!(
            load.rows[0],
            Row {
                ts_ms: 1_768_557_600_123,
                ns: Some(456_789),
                value: Some(Decimal::parse_token("0.53").unwrap()),
            }
        );
        assert_eq!(load.rows[1].value, None, "out of range: kept, flag 4");
        assert_eq!(
            r.items[1].rows[0].value.unwrap().to_string(),
            "18446744073709551615",
            "an unsigned item's value is never a double"
        );
        assert_eq!(r.not_numeric, 1);
        assert_eq!(r.unknown_items, BTreeSet::from(["99".to_string()]));
        assert_eq!(r.not_values, 1);
    }

    #[test]
    fn malformed_history_is_refused() {
        for (h, i) in [
            (r#"[{"itemid":"42","clock":"-1","value":"1"}]"#, ITEMS),
            (r#"[{"itemid":"42","clock":"1","value":"abc"}]"#, ITEMS),
            (
                r#"[{"itemid":"42","clock":"1","value":"1","ns":"1000000000"}]"#,
                ITEMS,
            ),
            (r#"[{"clock":"1","value":"1"}]"#, ITEMS),
            (
                "[]",
                r#"[{"itemid":"1","key_":"k","hosts":[{"host":"h"}]}]"#,
            ),
            (
                "[]",
                r#"[{"itemid":"1","key_":"k","value_type":"9","hosts":[{"host":"h"}]}]"#,
            ),
            ("[]", r#"[{"itemid":"1","key_":"k","value_type":"0"}]"#),
            (
                "[]",
                r#"[{"itemid":"1","key_":"a","value_type":"0","hosts":[{"host":"h"}]},
                    {"itemid":"1","key_":"b","value_type":"0","hosts":[{"host":"h"}]}]"#,
            ),
        ] {
            assert!(read(h, i, "7.0").is_err(), "{h} / {i}");
        }
        assert!(read("[]", ITEMS, "5.4").is_err(), "the version is required");
    }
}
