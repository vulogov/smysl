//! One object, read at one instant, recorded (RFC SMYSL-3 §3, §8.3, TL-P2).
//!
//! What a snapshot becomes is SMYSL-2 library records plus units:
//!
//! - a **part** (record 15): the filtered, keyed object as canonical JSON, [`crate::diff::part_text`];
//! - a **reading** (record 18) by the object's reader, with an empty segment table — a unit
//!   addresses a leaf by its pointer, so the part needs no table of its own;
//! - a **manifest** (record 14) under the object's alias, one part, `lang: zxx` (no linguistic
//!   content), part policy [`POLICY`], its origin naming the object and the instant, and
//!   `supersedes` the previous snapshot's manifest: the chain §3 calls "manifests chaining a
//!   source";
//! - the object's own **series** unit (§6);
//! - and, against the previous snapshot, the **diff** (§8.1).
//!
//! **An unchanged snapshot is nothing.** §8.3: a snapshot that changed only in filtered leaves
//! has its predecessor's tid and costs nothing, so when the part's tid is the previous part's,
//! [`Recorded::manifest`] is `None` and there is no diff. The part and reading are still
//! returned — they are the previous ones, and an object store keeps one copy.
//!
//! **The previous snapshot is its stored part.** A recorder does not keep raw objects: D-T1
//! applies to bytes. [`Prev::doc`] is the previous part's text parsed back, and the diff of a
//! stored part against a raw object is the diff of the two raw objects, because storing only
//! removes noise and keys arrays, both of which the diff does again and both of which are
//! idempotent. A test holds that.

use smysl_core::ids::LangTag;
use smysl_core::{
    Manifest, Mid, PartEntry, PartReading, PartText, SourceKind, SourceRef, Tid, UnitCore,
};

use crate::diff::{diff, part_text, Diff, Note, Snapshot};
use crate::json::Json;
use crate::profile::Profile;
use crate::readers::Object;

/// The part policy a snapshot manifest records (manifest key 17): one object, one part.
pub const POLICY: &str = "smysl/tel-snapshot/1";

/// The language of a snapshot: none. BCP-47's code for "no linguistic content".
pub const LANG: &str = "zxx";

/// The previous snapshot of the same object: its stored part, parsed, and its manifest.
#[derive(Debug, Clone, Copy)]
pub struct Prev<'a> {
    pub doc: &'a Json,
    pub mid: Mid,
}

/// What one snapshot became.
#[derive(Debug, Clone)]
pub struct Recorded {
    pub part: PartText,
    pub reading: PartReading,
    /// `None` when the part is the previous snapshot's: nothing changed that is stored.
    pub manifest: Option<Manifest>,
    /// The object's own series unit.
    pub series: UnitCore,
    /// Against the previous snapshot, when there was one and this one changed.
    pub diff: Option<Diff>,
    /// `SMY-W608` and `SMY-W613` from reading this snapshot alone, when there is no diff to
    /// carry them.
    pub notes: Vec<Note>,
}

/// Why a snapshot could not be recorded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The profile's scheme or the object's key cannot make a series unit.
    Series(String),
}

impl core::fmt::Display for RecordError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RecordError::Series(why) => write!(f, "no series unit: {why}"),
        }
    }
}

impl std::error::Error for RecordError {}

/// Record `obj`, read by `reader` at `at_ms`, under `profile`, after `prev`.
///
/// `licence` is the manifest's (key 4): what the snapshot's owner says may be done with it,
/// usually `unknown` for one's own systems. `raw` is the reader's metadata for the manifest
/// (key 16), as canonical CBOR — `zabbix/1` records the API version there.
pub fn record(
    obj: &Object,
    prev: Option<Prev<'_>>,
    profile: &Profile,
    reader: &str,
    licence: &str,
    raw: Option<Vec<u8>>,
    at_ms: u64,
) -> Result<Recorded, RecordError> {
    let (text, _) = part_text(&obj.doc, profile);
    let part = PartText::new(text.into_bytes());
    let reading = PartReading::new(part.tid, reader, Vec::new());
    let series = smysl_core::schemas::tel::series(&profile.scheme, &obj.key)
        .map_err(|e| RecordError::Series(e.to_string()))?;
    let series_uid = smysl_core::canonical_uid(&series);

    let unchanged = prev.is_some_and(|p| tid_of(p.doc, profile) == part.tid);
    if unchanged {
        return Ok(Recorded {
            part,
            reading,
            manifest: None,
            series,
            diff: None,
            notes: Vec::new(),
        });
    }

    let lang = LangTag::new(LANG).expect("`zxx` is a well-formed language tag");
    let entry = PartEntry::new(
        part.tid,
        part.text.len() as u64,
        reading.structure_hash(),
        reading.rdid(),
    );
    let mut manifest =
        Manifest::new(obj.alias.clone(), lang, reader, licence, POLICY).with_parts(vec![entry]);
    manifest.origin =
        Some(SourceRef::new(SourceKind::Tool, format!("{reader}:{}", obj.key)).observed_at(at_ms));
    manifest.raw = raw;
    if let Some(p) = prev {
        manifest = manifest.with_supersedes(p.mid);
    }
    let mid = manifest.mid();

    let (diff, notes) = match prev {
        Some(p) => {
            let s1 = Snapshot {
                doc: &obj.doc,
                tid: &part.tid,
                mid: &mid,
                at_ms,
            };
            let d = diff(p.doc, &s1, profile, Some(&series_uid));
            let notes = d.notes.clone();
            (Some(d), notes)
        }
        None => (None, crate::diff::leaves(&obj.doc, profile).notes),
    };
    Ok(Recorded {
        part,
        reading,
        manifest: Some(manifest),
        series,
        diff,
        notes,
    })
}

fn tid_of(doc: &Json, profile: &Profile) -> Tid {
    PartText::new(part_text(doc, profile).0.into_bytes()).tid
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;
    use crate::readers::k8s;

    const PROFILE: &str = r#"{
  profile: "t/1", scheme: k8s, select: { kind: "Deployment" }
  noise: [ "/metadata/resourceVersion" ]
  keys: { "/spec/containers": "name" }
  classes: { "/spec": config, "/metadata/uid": identity }
  ints: [ "/spec/replicas" ]
}"#;

    fn deploy(rv: u32, replicas: &str, image: &str, uid: &str) -> String {
        format!(
            r#"{{"apiVersion": "apps/v1", "kind": "Deployment",
  "metadata": {{"name": "web", "namespace": "a", "uid": "{uid}", "resourceVersion": "{rv}"}},
  "spec": {{"replicas": {replicas}, "containers": [{{"name": "web", "image": "{image}"}}, {{"name": "log", "image": "l:1"}}]}}}}"#
        )
    }

    fn read_one(text: &str) -> Object {
        k8s::read(text, &Profile::parse(PROFILE).unwrap())
            .unwrap()
            .objects
            .remove(0)
    }

    fn rec(obj: &Object, prev: Option<Prev<'_>>) -> Recorded {
        record(
            obj,
            prev,
            &Profile::parse(PROFILE).unwrap(),
            k8s::ID,
            "unknown",
            None,
            1_000,
        )
        .unwrap()
    }

    #[test]
    fn a_first_snapshot_is_a_part_a_manifest_and_a_series() {
        let o = read_one(&deploy(1, "3", "web:1", "u-1"));
        let r = rec(&o, None);
        let m = r.manifest.as_ref().expect("a manifest");
        assert_eq!(m.alias, "k8s/apps/v1/deployment/a/web");
        assert_eq!(m.reader, "k8s/1");
        assert_eq!(m.part_policy, POLICY);
        assert_eq!(m.lang.as_str(), "zxx");
        assert_eq!(m.parts.len(), 1);
        assert_eq!(m.parts[0].tid, r.part.tid);
        assert_eq!(m.parts[0].rdid, r.reading.rdid());
        assert!(m.supersedes.is_none());
        assert_eq!(m.origin.as_ref().and_then(|s| s.observed), Some(1_000));
        assert_eq!(r.series.schema.as_str(), smysl_core::schemas::tel::SERIES);
        assert!(r.diff.is_none());
        assert!(r.part.verify());
    }

    #[test]
    fn a_noise_only_snapshot_costs_nothing() {
        let a = read_one(&deploy(1, "3", "web:1", "u-1"));
        let first = rec(&a, None);
        let stored = parse(std::str::from_utf8(&first.part.text).unwrap()).unwrap();
        let mid = first.manifest.as_ref().unwrap().mid();
        let b = read_one(&deploy(2, "3.0", "web:1", "u-1"));
        let r = rec(&b, Some(Prev { doc: &stored, mid }));
        assert!(r.manifest.is_none() && r.diff.is_none());
        assert_eq!(r.part.tid, first.part.tid);
    }

    /// The previous snapshot is its stored part, and the diff against it is the diff against
    /// the raw object it was stored from.
    #[test]
    fn diffing_a_stored_part_is_diffing_the_raw_object() {
        let profile = Profile::parse(PROFILE).unwrap();
        let raw_a = parse(&deploy(1, "3", "web:1", "u-1")).unwrap();
        let first = rec(&read_one(&deploy(1, "3", "web:1", "u-1")), None);
        let stored = parse(std::str::from_utf8(&first.part.text).unwrap()).unwrap();
        let mid = first.manifest.as_ref().unwrap().mid();

        let b = read_one(&deploy(5, "4", "web:2", "u-1"));
        let r = rec(&b, Some(Prev { doc: &stored, mid }));
        let m = r.manifest.as_ref().unwrap();
        assert_eq!(m.supersedes, Some(mid), "the chain");
        let d = r.diff.as_ref().unwrap();
        assert_eq!(d.units.len(), 2, "replicas and the image");

        let s1 = Snapshot {
            doc: &b.doc,
            tid: &r.part.tid,
            mid: &m.mid(),
            at_ms: 1_000,
        };
        let series = smysl_core::canonical_uid(&r.series);
        let from_raw = diff(&raw_a, &s1, &profile, Some(&series));
        let uids = |d: &Diff| {
            d.units
                .iter()
                .map(smysl_core::canonical_uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(uids(d), uids(&from_raw));
    }

    #[test]
    fn a_recreated_object_continues_its_chain_and_is_a_new_series() {
        let first = rec(&read_one(&deploy(1, "3", "web:1", "u-1")), None);
        let stored = parse(std::str::from_utf8(&first.part.text).unwrap()).unwrap();
        let mid = first.manifest.as_ref().unwrap().mid();
        let again = read_one(&deploy(1, "3", "web:1", "u-2"));
        let r = rec(&again, Some(Prev { doc: &stored, mid }));
        assert_eq!(
            r.manifest.as_ref().unwrap().alias,
            first.manifest.as_ref().unwrap().alias
        );
        assert_ne!(
            smysl_core::canonical_uid(&r.series),
            smysl_core::canonical_uid(&first.series)
        );
        let d = r.diff.unwrap();
        assert_eq!(d.units.len(), 1);
        assert_eq!(
            d.units[0].schema.as_str(),
            smysl_core::schemas::tel::RECREATE
        );
    }
}
