//! Instruments and their lineage, as a person declares them (RFC SMYSL-3 §4.1, §4.2, §7, TL-P7).
//!
//! The readers record what they can see of a series' lineage ([`crate::scrape::Lineage`]): the
//! origin a source names, and the hop from it to the reader. What no export says is declared:
//!
//! - **`same-quantity`:** two series measure one physical quantity in one unit. Only a person,
//!   or a profile, knows it; no source says that a node_exporter's disk usage and an
//!   application's own disk metric are the same bytes.
//! - **`via`** beyond the reader's hop: a recording rule, a remote-write pipeline, a store.
//! - **`measured-by`** where the source does not say: an OTLP resource with no `service.name`.
//! - **`x.tel/instrument-meta`:** an instrument's version at an instant.
//!
//! These are the builders; `smysl tel declare` commits them, authored by `--as` at `--at`.

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

use crate::scrape::payload;

/// The key of an instrument unit, if `unit` is one.
pub fn key_of(unit: &UnitCore) -> Option<String> {
    if unit.schema.as_str() != tel::INSTRUMENT {
        return None;
    }
    let o = smysl_core::surface::payload_to_object(unit.payload.as_ref()?).ok()?;
    match &o.get("tel:key")?.value {
        HValue::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// The role of an instrument unit, if `unit` is one.
pub fn role_of(unit: &UnitCore) -> Option<String> {
    if unit.schema.as_str() != tel::INSTRUMENT {
        return None;
    }
    let o = smysl_core::surface::payload_to_object(unit.payload.as_ref()?).ok()?;
    match &o.get("tel:role")?.value {
        HValue::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// `x.tel/instrument-meta` (§4.1): `instrument` was at `version` when it was read, at `at_ms`.
///
/// Its source is the instrument's own reference, observed at `at_ms`: metadata is a reading of
/// the instrument, so two readings of one version at two instants are two units, and the later
/// one supersedes nothing by itself. `None` when `instrument` is not an instrument unit.
pub fn instrument_meta(instrument: &UnitCore, version: &str, at_ms: u64) -> Option<UnitCore> {
    let key = key_of(instrument)?;
    let of = canonical_uid(instrument);
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::INSTRUMENT_META.to_string()),
        tel::gist_of(&format!("{key} is version {version}")),
        Status::Cited,
    )
    .source(
        SourceRef::new(SourceKind::Tool, format!("{}{key}", tel::INSTRUMENT_REF))
            .observed_at(at_ms),
    )
    .payload(payload(&[
        ("tel:of", HValue::Str(of.canonical())),
        ("tel:version", HValue::Str(version.to_string())),
    ]))
    .build()
    .ok()
}

/// Find an instrument among `units` by uid or by key.
///
/// A key names one instrument unless two roles share it, which is refused by name: the role is
/// part of the uid, so the caller must say which one with the uid.
pub fn resolve<'a>(
    arg: &str,
    units: impl IntoIterator<Item = (&'a Uid, &'a UnitCore)>,
) -> Result<Uid, String> {
    let parsed = arg.parse::<Uid>().ok();
    let mut found: Vec<(Uid, String)> = Vec::new();
    for (uid, u) in units {
        if u.schema.as_str() != tel::INSTRUMENT {
            continue;
        }
        if parsed == Some(*uid) {
            return Ok(*uid);
        }
        if key_of(u).as_deref() == Some(arg) {
            found.push((*uid, role_of(u).unwrap_or_default()));
        }
    }
    match found.as_slice() {
        [(uid, _)] => Ok(*uid),
        [] if parsed.is_some() => Err(format!("{arg} is not an instrument here")),
        [] => Err(format!(
            "no instrument keyed `{arg}` here; declare it first (`tel declare instrument ROLE \
             KEY`)"
        )),
        many => Err(format!(
            "`{arg}` keys {} instruments ({}); name one by its uid",
            many.len(),
            many.iter()
                .map(|(u, r)| format!("{r} {}", u.canonical()))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_points_at_its_instrument_and_moves_nothing() {
        let i = tel::instrument("exporter", "prom-target:node/h:9100").unwrap();
        let a = instrument_meta(&i, "1.8.2", 10).unwrap();
        let b = instrument_meta(&i, "1.8.2", 20).unwrap();
        assert_ne!(canonical_uid(&a), canonical_uid(&b), "two readings");
        assert!(tel::entity_like_violations(&i).is_empty());
        let s = tel::series("prom", "up").unwrap();
        assert!(instrument_meta(&s, "1", 0).is_none());
    }

    #[test]
    fn an_instrument_by_uid_or_key_and_an_ambiguous_key_refused() {
        let a = tel::instrument("exporter", "k").unwrap();
        let b = tel::instrument("scraper", "k").unwrap();
        let c = tel::instrument("store", "m").unwrap();
        let units: Vec<(Uid, UnitCore)> = [a, b, c]
            .into_iter()
            .map(|u| (canonical_uid(&u), u))
            .collect();
        let it = || units.iter().map(|(u, c)| (u, c));
        assert_eq!(resolve("m", it()), Ok(units[2].0));
        assert_eq!(resolve(&units[0].0.canonical(), it()), Ok(units[0].0));
        assert!(resolve("k", it()).unwrap_err().contains("2 instruments"));
        assert!(resolve("nope", it())
            .unwrap_err()
            .contains("declare it first"));
    }
}
