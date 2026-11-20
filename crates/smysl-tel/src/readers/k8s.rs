//! `k8s/1` — Kubernetes objects, as `kubectl get -o json` writes them (RFC SMYSL-3 §20.1).
//!
//! Three shapes, in any mix, one after another in one file:
//!
//! - a **list**, `{"kind": "List" | "<Kind>List", "items": [...]}`, which `kubectl get -o json`
//!   writes for more than one object;
//! - a **single object**, which it writes for one;
//! - a **watch event**, `{"type": "ADDED" | "MODIFIED" | "DELETED", "object": {...}}`, which
//!   `kubectl get -w -o json --output-watch-events` writes, pretty-printed and back to back.
//!   `BOOKMARK` events carry no object state and are passed over; an `ERROR` event is the
//!   API server refusing the watch, and is refused here rather than read as an object.
//!
//! **Secrets.** §8.3: `k8s/1` MUST NOT read the `data` or `stringData` of a `Secret` at all.
//! They are removed the moment an object is recognised as a Secret, before selection, before
//! the profile, before anything could hash, store or diff them. A profile cannot opt back in:
//! there is no field for it, on purpose.
//!
//! **Identity.** The series key is `apiVersion/kind/namespace/name#uid` (§6), with an empty
//! namespace for a cluster-scoped object. The alias is `k8s/<group>/<version>/<kind>/<ns>/<name>`
//! in lowercase, `-` for no namespace — without the uid, so a recreated object continues its
//! manifest chain and the diff sees the uid change.

use crate::json::{parse_seq, Json};
use crate::profile::Profile;
use crate::readers::{alias_for, shape, text, Object, Read, ReadError};

/// The reader id a manifest records.
pub const ID: &str = "k8s/1";

/// The fields of a `Secret` this reader never reads.
pub const SECRET_FIELDS: &[&str] = &["data", "stringData"];

/// Read every object in `input` that `profile` selects.
pub fn read(input: &str, profile: &Profile) -> Result<Read, ReadError> {
    let mut out = Read::default();
    for v in parse_seq(input)? {
        for (obj, deleted) in objects(v)? {
            let obj = redact_secret(obj);
            if !profile.selects(&obj) {
                out.skipped += 1;
                continue;
            }
            out.objects.push(object(obj, deleted)?);
        }
    }
    Ok(out)
}

/// The objects one top-level value holds, each with whether a watch event deleted it.
fn objects(v: Json) -> Result<Vec<(Json, bool)>, ReadError> {
    if let (Some(Json::String(t)), Some(_)) = (v.get("type"), v.get("object")) {
        let deleted = match t.as_str() {
            "ADDED" | "MODIFIED" => false,
            "DELETED" => true,
            "BOOKMARK" => return Ok(Vec::new()),
            "ERROR" => return Err(shape("a watch ERROR event: the API server ended the watch")),
            other => return Err(shape(format!("an unknown watch event type `{other}`"))),
        };
        let Json::Object(members) = v else {
            unreachable!("`get` found members")
        };
        let obj = members
            .into_iter()
            .find(|(k, _)| k == "object")
            .map(|(_, o)| o)
            .expect("checked above");
        return Ok(vec![(obj, deleted)]);
    }
    let is_list = text(&v, "kind").ends_with("List") && v.get("items").is_some();
    if is_list {
        let Json::Object(members) = v else {
            unreachable!("a list has members")
        };
        let items = members
            .into_iter()
            .find(|(k, _)| k == "items")
            .map(|(_, i)| i)
            .expect("checked above");
        let Json::Array(items) = items else {
            return Err(shape("a list's `items` is not an array"));
        };
        return Ok(items.into_iter().map(|i| (i, false)).collect());
    }
    match v {
        Json::Object(_) => Ok(vec![(v, false)]),
        _ => Err(shape("not a Kubernetes object, list or watch event")),
    }
}

/// §8.3: a Secret's `data` and `stringData` are gone before anything else sees the object.
fn redact_secret(obj: Json) -> Json {
    if text(&obj, "kind") != "Secret" {
        return obj;
    }
    match obj {
        Json::Object(members) => Json::Object(
            members
                .into_iter()
                .filter(|(k, _)| !SECRET_FIELDS.contains(&k.as_str()))
                .collect(),
        ),
        other => other,
    }
}

fn object(doc: Json, deleted: bool) -> Result<Object, ReadError> {
    let api = text(&doc, "apiVersion");
    let kind = text(&doc, "kind");
    let meta = doc
        .get("metadata")
        .ok_or_else(|| shape(format!("a `{kind}` with no metadata")))?;
    let (ns, name, uid) = (
        text(meta, "namespace"),
        text(meta, "name"),
        text(meta, "uid"),
    );
    if api.is_empty() || kind.is_empty() || name.is_empty() || uid.is_empty() {
        return Err(shape(format!(
            "an object needs apiVersion, kind, metadata.name and metadata.uid \
             (found `{api}`, `{kind}`, `{name}`, `{uid}`)"
        )));
    }
    let key = format!("{api}/{kind}/{ns}/{name}#{uid}");
    let kind_lower = kind.to_ascii_lowercase();
    let mut segments: Vec<&str> = api.split('/').collect();
    segments.push(&kind_lower);
    segments.push(if ns.is_empty() { "-" } else { ns });
    segments.push(name);
    let alias = alias_for("k8s", &segments, &key);
    Ok(Object {
        alias,
        key,
        doc,
        deleted,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deployments() -> Profile {
        Profile::parse(r#"{ profile: "t/1", scheme: k8s, select: { kind: "Deployment" } }"#)
            .unwrap()
    }

    fn any() -> Profile {
        Profile::parse(r#"{ profile: "t/1", scheme: k8s }"#).unwrap()
    }

    const DEPLOY: &str = r#"{"apiVersion": "apps/v1", "kind": "Deployment",
      "metadata": {"name": "checkout", "namespace": "shop", "uid": "u-1"}, "spec": {"replicas": 3}}"#;

    #[test]
    fn a_single_object_a_list_and_a_watch_stream() {
        let r = read(DEPLOY, &deployments()).unwrap();
        assert_eq!(r.objects.len(), 1);
        let o = &r.objects[0];
        assert_eq!(o.key, "apps/v1/Deployment/shop/checkout#u-1");
        assert_eq!(o.alias, "k8s/apps/v1/deployment/shop/checkout");
        assert!(!o.deleted);

        let list = format!(
            r#"{{"apiVersion": "v1", "kind": "List", "items": [{DEPLOY}, {{"apiVersion": "v1", "kind": "Service",
              "metadata": {{"name": "checkout", "namespace": "shop", "uid": "u-2"}}}}]}}"#
        );
        let r = read(&list, &deployments()).unwrap();
        assert_eq!((r.objects.len(), r.skipped), (1, 1));

        let watch = format!(
            "{{\n \"type\": \"ADDED\",\n \"object\": {DEPLOY}\n}}\n{{\"type\": \"BOOKMARK\", \"object\": {{}}}}\n{{\n \"type\": \"DELETED\",\n \"object\": {DEPLOY}\n}}\n"
        );
        let r = read(&watch, &deployments()).unwrap();
        assert_eq!(r.objects.len(), 2);
        assert!(!r.objects[0].deleted && r.objects[1].deleted);
    }

    #[test]
    fn a_cluster_scoped_object_has_an_empty_namespace_in_its_key() {
        let node =
            r#"{"apiVersion": "v1", "kind": "Node", "metadata": {"name": "n1", "uid": "u-9"}}"#;
        let o = &read(node, &any()).unwrap().objects[0];
        assert_eq!(o.key, "v1/Node//n1#u-9");
        assert_eq!(o.alias, "k8s/v1/node/-/n1");
    }

    /// §8.3's MUST NOT, as a test: the values are not in what the reader returns, whatever the
    /// profile says.
    #[test]
    fn a_secret_is_read_without_its_data() {
        let secret = r#"{"apiVersion": "v1", "kind": "Secret", "type": "Opaque",
          "metadata": {"name": "db", "namespace": "shop", "uid": "u-3"},
          "data": {"password": "aHVudGVyMg=="}, "stringData": {"token": "t0p"}}"#;
        let o = &read(secret, &any()).unwrap().objects[0];
        assert!(o.doc.get("data").is_none());
        assert!(o.doc.get("stringData").is_none());
        assert_eq!(text(&o.doc, "type"), "Opaque", "the rest is read");
        let (part, _) = crate::diff::part_text(&o.doc, &any());
        assert!(!part.contains("aHVudGVyMg") && !part.contains("t0p"));
    }

    #[test]
    fn what_is_not_an_object_is_refused() {
        assert!(read(r#"{"type": "ERROR", "object": {"code": 410}}"#, &any()).is_err());
        assert!(read("[1, 2]", &any()).is_err());
        assert!(read(
            r#"{"apiVersion": "v1", "kind": "Pod", "metadata": {"name": "p"}}"#,
            &any()
        )
        .is_err());
        assert_eq!(read("", &any()).unwrap(), Read::default());
    }
}
