//! `zabbix/1` — Zabbix API exports: hosts and items (RFC SMYSL-3 §20.1, TL-P2).
//!
//! The input is what the API returned to `host.get` or `item.get`, either the whole JSON-RPC
//! response (`{"jsonrpc": "2.0", "result": [...]}`) or its `result` array alone. History is
//! [`super::zabbix_history`]'s (TL-P3); trends, problems and events are later.
//!
//! **The API version is required.** §20.1: the caller records the version it saw
//! (`apiinfo.version`, 6.x, 7.x or 8.x) beside the export, and the reader refuses an export
//! without it. The field names and their meanings moved between majors, so an export whose
//! version is unknown is an export nobody can read twice the same way.
//!
//! **Identity.** A host is `host:<host>`; an item is `host:<host>/item:<key_>`, with the item
//! key verbatim (§6). The host's technical name, not its numeric id: `hostid` and `itemid`
//! change when templates are re-imported, so they are metadata, not identity. An item names its
//! host only if the export asked for it (`selectHosts: ["host"]`); one that did not is refused
//! with that remedy, because the alternative — the `hostid` — is the identity §6 rejects.

use crate::json::{parse_seq, Json};
use crate::profile::Profile;
use crate::readers::{alias_for, shape, text, Object, Read, ReadError};

/// The reader id a manifest records.
pub const ID: &str = "zabbix/1";

/// Whether `v` is an API version this reader reads: a 6, 7 or 8 major, then dotted numbers.
pub fn version_is_supported(v: &str) -> bool {
    let mut parts = v.split('.');
    matches!(parts.next(), Some("6" | "7" | "8"))
        && v.contains('.')
        && parts.all(|p| !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
}

/// Read every host or item in `input` that `profile` selects, from an export of API
/// `api_version`.
pub fn read(input: &str, profile: &Profile, api_version: &str) -> Result<Read, ReadError> {
    require_version(api_version)?;
    let mut out = Read::default();
    for obj in results(input)? {
        if !profile.selects(&obj) {
            out.skipped += 1;
            continue;
        }
        out.objects.push(object(obj)?);
    }
    Ok(out)
}

/// The objects of one or more API responses: each a whole JSON-RPC response or its `result`
/// array alone.
pub(crate) fn results(input: &str) -> Result<Vec<Json>, ReadError> {
    let mut out = Vec::new();
    for v in parse_seq(input)? {
        out.extend(match v {
            Json::Array(items) => items,
            Json::Object(members) => match members.into_iter().find(|(k, _)| k == "result") {
                Some((_, Json::Array(items))) => items,
                Some(_) => return Err(shape("a JSON-RPC `result` that is not an array")),
                None => return Err(shape("not a Zabbix API response: no `result`")),
            },
            _ => return Err(shape("not a Zabbix API export")),
        });
    }
    Ok(out)
}

/// The version check every `zabbix/1` read makes first.
pub(crate) fn require_version(api_version: &str) -> Result<(), ReadError> {
    if version_is_supported(api_version) {
        Ok(())
    } else {
        Err(shape(format!(
            "`{api_version}` is not a Zabbix API version this reader reads; record the \
             `apiinfo.version` the export was taken from: 6.x, 7.x or 8.x"
        )))
    }
}

/// An item's host and key, which every item export must carry (`selectHosts: ["host"]`).
pub(crate) fn item_host<'a>(doc: &'a Json, key_: &str) -> Result<&'a str, ReadError> {
    let host = doc
        .get("hosts")
        .and_then(|h| match h {
            Json::Array(hs) => hs.first(),
            _ => None,
        })
        .map(|h| text(h, "host"))
        .unwrap_or("");
    if host.is_empty() {
        return Err(shape(format!(
            "item `{key_}` does not name its host; export items with `selectHosts: [\"host\"]`"
        )));
    }
    Ok(host)
}

fn object(doc: Json) -> Result<Object, ReadError> {
    let key_ = text(&doc, "key_");
    if !key_.is_empty() {
        let host = item_host(&doc, key_)?;
        let key = format!("host:{host}/item:{key_}");
        let alias = alias_for("zabbix", &["item", host, key_], &key);
        return Ok(Object {
            alias,
            key,
            doc,
            deleted: false,
        });
    }
    let host = text(&doc, "host");
    if host.is_empty() {
        return Err(shape("neither a host (`host`) nor an item (`key_`)"));
    }
    let key = format!("host:{host}");
    let alias = alias_for("zabbix", &["host", host], &key);
    Ok(Object {
        alias,
        key,
        doc,
        deleted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Profile {
        Profile::parse(r#"{ profile: "t/1", scheme: zabbix, select: { key_: "*" } }"#).unwrap()
    }

    fn hosts() -> Profile {
        Profile::parse(r#"{ profile: "t/1", scheme: zabbix, select: { host: "*" } }"#).unwrap()
    }

    const HOSTS: &str = r#"{"jsonrpc": "2.0", "id": 1, "result": [
      {"hostid": "10084", "host": "web-01", "name": "Web 01", "status": "0"},
      {"hostid": "10085", "host": "DB 01", "name": "Database", "status": "0"}]}"#;

    const ITEMS: &str = r#"[
      {"itemid": "42", "key_": "system.cpu.load[percpu,avg1]", "delay": "1m",
       "hosts": [{"hostid": "10084", "host": "web-01"}]}]"#;

    #[test]
    fn hosts_and_items_by_their_technical_names() {
        let r = read(HOSTS, &hosts(), "7.0.3").unwrap();
        assert_eq!(r.objects.len(), 2);
        assert_eq!(r.objects[0].key, "host:web-01");
        assert_eq!(r.objects[0].alias, "zabbix/host/web-01");
        assert_eq!(r.objects[1].key, "host:DB 01");
        assert!(
            r.objects[1].alias.starts_with("zabbix/h/"),
            "not a valid alias as written"
        );

        let r = read(ITEMS, &items(), "6.0.30").unwrap();
        assert_eq!(
            r.objects[0].key,
            "host:web-01/item:system.cpu.load[percpu,avg1]"
        );
    }

    #[test]
    fn a_profile_selects_hosts_or_items() {
        let both = format!("{HOSTS}\n{ITEMS}");
        assert!(read(&both, &items(), "7.0").is_ok());
        let r = read(&both, &items(), "7.0").unwrap();
        assert_eq!((r.objects.len(), r.skipped), (1, 2));
    }

    #[test]
    fn an_export_without_its_version_is_refused() {
        for v in ["", "5.4", "7", "7.x", "latest", "9.0"] {
            assert!(read(HOSTS, &hosts(), v).is_err(), "{v:?}");
        }
        for v in ["6.0", "7.0.3", "8.0.0"] {
            assert!(version_is_supported(v), "{v}");
        }
    }

    #[test]
    fn an_item_must_name_its_host() {
        let bare = r#"[{"itemid": "42", "key_": "agent.ping", "hostid": "10084"}]"#;
        let e = read(bare, &items(), "7.0").unwrap_err();
        assert!(e.to_string().contains("selectHosts"), "{e}");
    }
}
