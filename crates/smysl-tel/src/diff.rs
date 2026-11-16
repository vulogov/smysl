//! The canonical diff (RFC SMYSL-3 §8.1): two snapshots of one object, and what changed.
//!
//! A function of (S₀, S₁, P) and nothing else, so the same inputs give the same uids on every
//! implementation, which `fixtures/tel/diff/` holds them to. The steps, as §8.1 numbers them:
//!
//! 1. **Parse.** The snapshots arrive as [`Json`], numbers still tokens.
//! 2. **Key arrays.** An array whose pointer the profile keys becomes an object keyed by each
//!    element's key text — if every element is an object whose key field is a scalar and the
//!    key texts are pairwise distinct. Otherwise the whole array is one `json` leaf, and
//!    `SMY-W608` says which. An array the profile does not key is one `json` leaf too.
//! 3. **Filter.** A leaf, or a subtree, a noise pattern covers is gone. Noise patterns name
//!    keyed pointers, so filtering runs as the walk descends, after the keying above it.
//! 4. **Classify.** The profile's longest covering class pattern; `gauge` if none.
//! 5. **Diff per leaf.** `add`, `remove`, or `replace` when the `vtype` or the text differs.
//! 6. **Emit.** One `x.tel/change` per diffed `config` or `membership` leaf; an `identity` leaf
//!    that was replaced is an `x.tel/recreate`. Leaves of a class the profile routes (`gauge` to
//!    `series`) are handed back for the substrate (TL-P3), and every other leaf emits nothing.
//!
//! **What a change's source is.** `reference` is S₁'s tid with the pointer as its locator,
//! `manifest` is S₁'s mid, `observed` is S₁'s instant: the change is known from S₁, and S₁ is
//! where to look. Status is `measured`: a change is a deterministic transcription of two
//! measured snapshots. The *attestation* — `Op::Imported` at `Rung::Computed`, the only path to
//! `measured` under rule T — is the caller's, as every producer's attestation is.
//!
//! **Two choices §8.1 leaves open, made here:**
//!
//! - *An empty object is a leaf* (`{}`, `vtype json`). It has no leaves to diff, so without
//!   this an object emptied between snapshots would vanish without a change.
//! - *A replaced leaf's `tel:vtype` is the new value's.* §4.1 gives a change one `vtype`, so a
//!   leaf that went from `41` to `"41"` records `text`, and `tel:from` is `41` read as what it
//!   was; the old `vtype` is S₀'s to say. An `identity` leaf added or removed — a uid where there
//!   was none — is a `change` with class `identity`, because a recreate needs both identities.

use std::collections::BTreeMap;

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::{
    Code, Mid, SchemaId, SourceKind, SourceRef, Status, Tid, Uid, UnitCore, UnitCoreBuilder,
};

use crate::json::Json;
use crate::leaf::{Leaf, VType};
use crate::pointer::Pointer;
use crate::profile::Profile;

/// What a diff could not do cleanly, by pointer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Note {
    /// A declared keyed array that could not be keyed (`SMY-W608`): an element that is not an
    /// object, a key that is missing or not a scalar, or a key text that repeats.
    Unkeyable(Pointer, &'static str),
    /// A number outside `dec`'s range, kept as its token (`SMY-W613`).
    OutOfRange(Pointer),
}

impl Note {
    pub fn code(&self) -> Code {
        match self {
            Note::Unkeyable(..) => Code::W608,
            Note::OutOfRange(_) => Code::W613,
        }
    }

    pub fn pointer(&self) -> &Pointer {
        match self {
            Note::Unkeyable(p, _) | Note::OutOfRange(p) => p,
        }
    }
}

/// A snapshot's leaves, after keying and filtering, and what the walk noticed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Leaves {
    pub leaves: BTreeMap<Pointer, Leaf>,
    pub notes: Vec<Note>,
}

/// §8.1 steps 2 and 3 over one snapshot.
pub fn leaves(doc: &Json, profile: &Profile) -> Leaves {
    let mut out = Leaves::default();
    walk(doc, &Pointer::root(), profile, &mut out);
    out.notes.sort();
    out.notes.dedup();
    out
}

fn leaf_at(v: &Json, at: &Pointer, profile: &Profile, out: &mut Leaves) {
    let leaf = Leaf::of(v, profile.is_int(at));
    if leaf.out_of_range {
        out.notes.push(Note::OutOfRange(at.clone()));
    }
    out.leaves.insert(at.clone(), leaf);
}

fn walk(v: &Json, at: &Pointer, profile: &Profile, out: &mut Leaves) {
    if profile.is_noise(at) {
        return;
    }
    match v {
        Json::Object(members) if !members.is_empty() => {
            for (k, child) in members {
                walk(child, &at.child(k.clone()), profile, out);
            }
        }
        Json::Array(items) => match profile.key_field(at) {
            None => leaf_at(v, at, profile, out),
            Some(field) => match keyed(items, field) {
                Ok(members) if members.is_empty() => {
                    leaf_at(&Json::Object(Vec::new()), at, profile, out)
                }
                Ok(members) => {
                    for (k, child) in members {
                        walk(child, &at.child(k), profile, out);
                    }
                }
                Err(why) => {
                    out.notes.push(Note::Unkeyable(at.clone(), why));
                    leaf_at(v, at, profile, out);
                }
            },
        },
        _ => leaf_at(v, at, profile, out),
    }
}

/// §8.1 step 2: the array as `(key text, element)`, or why it cannot be.
fn keyed<'a>(items: &'a [Json], field: &str) -> Result<Vec<(String, &'a Json)>, &'static str> {
    let mut out: Vec<(String, &Json)> = Vec::with_capacity(items.len());
    for item in items {
        let Json::Object(_) = item else {
            return Err("an element is not an object");
        };
        let key = match item.get(field) {
            None => return Err("an element has no key"),
            Some(k @ (Json::String(_) | Json::Number(_))) => {
                let l = Leaf::of(k, false);
                if l.vtype == VType::Text && matches!(k, Json::Number(_)) {
                    return Err("a key is a number outside dec's range");
                }
                l.text
            }
            Some(_) => return Err("a key is not a text, int or dec scalar"),
        };
        if out.iter().any(|(k, _)| *k == key) {
            return Err("a key value repeats");
        }
        out.push((key, item));
    }
    Ok(out)
}

/// §8.3: what a snapshot's part stores — the **filtered, keyed** object as canonical JSON (JCS
/// with `dec` numbers), so that a snapshot changed only in noise has its predecessor's tid and
/// costs nothing. The unfiltered object is not kept: D-T1 applies to bytes as well as to units.
///
/// The keying and filtering are [`leaves`]' own, so a part and the leaves diffed from it cannot
/// disagree about what the object was. The second value says a number in it was out of `dec`'s
/// range and is written as its token.
pub fn part_text(doc: &Json, profile: &Profile) -> (String, bool) {
    let shaped = shape(doc, &Pointer::root(), profile).unwrap_or(Json::Null);
    crate::jcs::canonical(&shaped)
}

fn shape(v: &Json, at: &Pointer, profile: &Profile) -> Option<Json> {
    if profile.is_noise(at) {
        return None;
    }
    Some(match v {
        Json::Object(members) => Json::Object(
            members
                .iter()
                .filter_map(|(k, c)| Some((k.clone(), shape(c, &at.child(k.clone()), profile)?)))
                .collect(),
        ),
        Json::Array(items) => match profile.key_field(at).map(|f| keyed(items, f)) {
            Some(Ok(members)) => Json::Object(
                members
                    .into_iter()
                    .filter_map(|(k, c)| Some((k.clone(), shape(c, &at.child(k), profile)?)))
                    .collect(),
            ),
            _ => v.clone(),
        },
        other => other.clone(),
    })
}

/// One object's snapshot, with what its part and manifest are and when it was read.
#[derive(Debug, Clone, Copy)]
pub struct Snapshot<'a> {
    pub doc: &'a Json,
    /// The tid of the part that stores the filtered, keyed object (§8.3).
    pub tid: &'a Tid,
    /// The manifest the part is under.
    pub mid: &'a Mid,
    /// The instant of the reading, epoch milliseconds.
    pub at_ms: u64,
}

/// A leaf the profile routes elsewhere instead of to a unit (§20.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routed {
    pub pointer: Pointer,
    pub class: String,
    pub to: String,
    pub leaf: Leaf,
}

/// What §8.1 produces from two snapshots.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    /// `x.tel/change` and `x.tel/recreate` units, in pointer order.
    pub units: Vec<UnitCore>,
    /// S₁'s leaves of a routed class, changed or not: the substrate wants every reading.
    pub routed: Vec<Routed>,
    /// Both snapshots' notes, S₁'s last.
    pub notes: Vec<Note>,
}

/// §8.1 over S₀ and S₁ under one profile. `object` is the uid of the object's own series unit,
/// written as `tel:object` when given.
pub fn diff(s0: &Json, s1: &Snapshot<'_>, profile: &Profile, object: Option<&Uid>) -> Diff {
    let before = leaves(s0, profile);
    let after = leaves(s1.doc, profile);
    let mut out = Diff {
        notes: before.notes.iter().chain(&after.notes).cloned().collect(),
        ..Diff::default()
    };
    out.notes.sort();
    out.notes.dedup();

    for (p, l) in &after.leaves {
        let class = profile.class_of(p);
        if let Some(to) = profile.route_of(class) {
            out.routed.push(Routed {
                pointer: p.clone(),
                class: class.to_string(),
                to: to.to_string(),
                leaf: l.clone(),
            });
        }
    }

    let mut pointers: Vec<&Pointer> = before.leaves.keys().chain(after.leaves.keys()).collect();
    pointers.sort();
    pointers.dedup();
    for p in pointers {
        let (was, now) = (before.leaves.get(p), after.leaves.get(p));
        let op = match (was, now) {
            (None, Some(_)) => "add",
            (Some(_), None) => "remove",
            (Some(a), Some(b)) if a.vtype != b.vtype || a.text != b.text => "replace",
            _ => continue,
        };
        let class = profile.class_of(p);
        let unit = match (class, op) {
            ("identity", "replace") => recreate(p, was, now, s1, object),
            ("config" | "membership" | "identity", _) => change(p, op, class, was, now, s1, object),
            _ => continue,
        };
        out.units.push(unit);
    }
    out
}

fn source(p: &Pointer, s1: &Snapshot<'_>) -> SourceRef {
    SourceRef::new(SourceKind::Doc, format!("{}#{p}", s1.tid.canonical()))
        .of_manifest(*s1.mid)
        .observed_at(s1.at_ms)
}

fn payload(entries: Vec<(&str, String)>) -> Vec<u8> {
    let span = smysl_core::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(
            Spanned::new(k.to_string(), span),
            Spanned::new(HValue::Str(v), span),
        );
    }
    smysl_core::surface::object_to_payload(&o).unwrap_or_default()
}

fn build(ty: &str, gist: String, source: SourceRef, payload: Vec<u8>) -> UnitCore {
    // Cannot fail: the gist is non-empty, the status is `measured` and the source is present.
    UnitCoreBuilder::new(SchemaId::Extension(ty.to_string()), gist, Status::Measured)
        .source(source)
        .payload(payload)
        .build()
        .expect("a change unit is well formed by construction")
}

fn change(
    p: &Pointer,
    op: &str,
    class: &str,
    was: Option<&Leaf>,
    now: Option<&Leaf>,
    s1: &Snapshot<'_>,
    object: Option<&Uid>,
) -> UnitCore {
    let vtype = now.or(was).map_or(VType::Null, |l| l.vtype);
    let mut entries = vec![
        ("tel:op", op.to_string()),
        ("tel:pointer", p.to_string()),
        ("tel:class", class.to_string()),
        ("tel:vtype", vtype.as_str().to_string()),
    ];
    if let Some(l) = was {
        entries.push(("tel:from", l.text.clone()));
    }
    if let Some(l) = now {
        entries.push(("tel:to", l.text.clone()));
    }
    if let Some(u) = object {
        entries.push(("tel:object", u.canonical()));
    }
    build(
        tel::CHANGE,
        tel::gist_of(&format!("{op} {p}")),
        source(p, s1),
        payload(entries),
    )
}

fn recreate(
    p: &Pointer,
    was: Option<&Leaf>,
    now: Option<&Leaf>,
    s1: &Snapshot<'_>,
    object: Option<&Uid>,
) -> UnitCore {
    let text = |l: Option<&Leaf>| l.map(|l| l.text.clone()).unwrap_or_default();
    let mut entries = vec![("tel:old", text(was)), ("tel:new", text(now))];
    if let Some(u) = object {
        entries.push(("tel:object", u.canonical()));
    }
    build(
        tel::RECREATE,
        tel::gist_of(&format!("recreated {p}")),
        source(p, s1),
        payload(entries),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    const PROFILE: &str = r#"{
  profile: "k8s-deployments/1"
  scheme: k8s
  noise: [ "/metadata/managedFields", "/metadata/resourceVersion", "/status/conditions/*/lastHeartbeatTime" ]
  keys: { "/spec/containers": "name", "/spec/containers/*/env": "name", "/status/conditions": "type" }
  classes: { "/spec": config, "/metadata/labels": membership, "/metadata/uid": identity, "/status": gauge }
  ints: [ "/spec/replicas", "/status/readyReplicas" ]
  route: { gauge: series }
}"#;

    fn profile() -> Profile {
        Profile::parse(PROFILE).unwrap()
    }

    fn ids() -> (Tid, Mid) {
        (Tid::from_bytes([7; 32]), Mid::from_bytes([9; 32]))
    }

    fn run(a: &str, b: &str) -> Diff {
        let (tid, mid) = ids();
        let s1 = Snapshot {
            doc: &parse(b).unwrap(),
            tid: &tid,
            mid: &mid,
            at_ms: 1_705_400_000_000,
        };
        diff(&parse(a).unwrap(), &s1, &profile(), None)
    }

    fn field(u: &UnitCore, k: &str) -> Option<String> {
        let o = smysl_core::surface::payload_to_object(u.payload.as_ref()?).ok()?;
        match &o.get(k)?.value {
            HValue::Str(s) => Some(s.clone()),
            _ => None,
        }
    }

    const S0: &str = r#"{
  "metadata": {"uid": "u-1", "resourceVersion": "1", "labels": {"app": "web"},
               "managedFields": [{"manager": "kubectl"}]},
  "spec": {"replicas": 3,
           "containers": [{"name": "web", "image": "web:1.0", "env": [{"name": "A", "value": "1"}]}]},
  "status": {"readyReplicas": 3, "conditions": [{"type": "Ready", "status": "True", "lastHeartbeatTime": "t0"}]}
}"#;

    #[test]
    fn keyed_arrays_name_their_leaves_by_key_and_noise_is_gone() {
        let l = leaves(&parse(S0).unwrap(), &profile());
        let ptrs: Vec<String> = l.leaves.keys().map(ToString::to_string).collect();
        assert!(
            ptrs.contains(&"/spec/containers/web/image".to_string()),
            "{ptrs:?}"
        );
        assert!(
            ptrs.contains(&"/spec/containers/web/env/A/value".to_string()),
            "{ptrs:?}"
        );
        assert!(
            ptrs.contains(&"/status/conditions/Ready/status".to_string()),
            "{ptrs:?}"
        );
        assert!(!ptrs
            .iter()
            .any(|p| p.contains("managedFields") || p.contains("resourceVersion")));
        assert!(!ptrs.iter().any(|p| p.contains("lastHeartbeatTime")));
        assert!(l.notes.is_empty());
    }

    /// Only noise and gauges moved: no unit. `41` → `41.0` is no change either.
    #[test]
    fn a_heartbeat_is_not_a_change() {
        let s1 = S0
            .replace(r#""resourceVersion": "1""#, r#""resourceVersion": "2""#)
            .replace(
                r#""lastHeartbeatTime": "t0""#,
                r#""lastHeartbeatTime": "t1""#,
            )
            .replace(r#""readyReplicas": 3"#, r#""readyReplicas": 2"#);
        let d = run(S0, &s1);
        assert!(d.units.is_empty(), "{:?}", d.units);
        // The gauge went to the substrate, as every routed reading does.
        assert!(d
            .routed
            .iter()
            .any(|r| r.pointer.to_string() == "/status/readyReplicas" && r.leaf.text == "2"));
    }

    #[test]
    fn an_image_bump_is_one_replace_with_its_values() {
        let d = run(S0, &S0.replace("web:1.0", "web:1.1"));
        assert_eq!(d.units.len(), 1);
        let u = &d.units[0];
        assert_eq!(u.schema.as_str(), tel::CHANGE);
        assert_eq!(u.status, Status::Measured);
        assert_eq!(u.gist, "replace /spec/containers/web/image");
        assert_eq!(field(u, "tel:op").as_deref(), Some("replace"));
        assert_eq!(field(u, "tel:class").as_deref(), Some("config"));
        assert_eq!(field(u, "tel:vtype").as_deref(), Some("text"));
        assert_eq!(field(u, "tel:from").as_deref(), Some("web:1.0"));
        assert_eq!(field(u, "tel:to").as_deref(), Some("web:1.1"));
        let s = u.source.as_ref().unwrap();
        let (tid, mid) = ids();
        assert_eq!(
            s.reference,
            format!("{}#/spec/containers/web/image", tid.canonical())
        );
        assert_eq!(s.manifest(), Some(&mid));
        assert_eq!(s.observed, Some(1_705_400_000_000));
        assert!(s.span().is_none());
    }

    #[test]
    fn reordering_a_keyed_array_is_no_change_and_a_new_element_is_adds() {
        let two = r#"[{"name": "web", "image": "web:1.0"}, {"name": "side", "image": "s:1"}]"#;
        let swapped = r#"[{"name": "side", "image": "s:1"}, {"name": "web", "image": "web:1.0"}]"#;
        let a = format!(r#"{{"spec": {{"containers": {two}}}}}"#);
        let b = format!(r#"{{"spec": {{"containers": {swapped}}}}}"#);
        assert!(run(&a, &b).units.is_empty());
        let one = r#"{"spec": {"containers": [{"name": "web", "image": "web:1.0"}]}}"#;
        let ops: Vec<(String, String)> = run(one, &a)
            .units
            .iter()
            .map(|u| {
                (
                    field(u, "tel:op").unwrap(),
                    field(u, "tel:pointer").unwrap(),
                )
            })
            .collect();
        assert_eq!(
            ops,
            [
                ("add".to_string(), "/spec/containers/side/image".to_string()),
                ("add".to_string(), "/spec/containers/side/name".to_string()),
            ]
        );
    }

    #[test]
    fn numbers_are_values_not_spellings() {
        let a = r#"{"spec": {"replicas": 3, "ratio": 0.50}}"#;
        assert!(run(a, r#"{"spec": {"replicas": 3.0, "ratio": 0.5}}"#)
            .units
            .is_empty());
        let d = run(a, r#"{"spec": {"replicas": 5, "ratio": 0.5}}"#);
        assert_eq!(field(&d.units[0], "tel:vtype").as_deref(), Some("int"));
        assert_eq!(field(&d.units[0], "tel:to").as_deref(), Some("5"));
    }

    #[test]
    fn a_label_is_membership_and_a_new_uid_is_a_recreate() {
        let d = run(
            S0,
            &S0.replace(r#""app": "web""#, r#""app": "web", "tier": "front""#),
        );
        assert_eq!(d.units.len(), 1);
        assert_eq!(
            field(&d.units[0], "tel:class").as_deref(),
            Some("membership")
        );
        assert_eq!(field(&d.units[0], "tel:op").as_deref(), Some("add"));

        let d = run(S0, &S0.replace("u-1", "u-2"));
        assert_eq!(d.units.len(), 1);
        assert_eq!(d.units[0].schema.as_str(), tel::RECREATE);
        assert_eq!(field(&d.units[0], "tel:old").as_deref(), Some("u-1"));
        assert_eq!(field(&d.units[0], "tel:new").as_deref(), Some("u-2"));
    }

    /// `env` with a repeated name, which Kubernetes admits: the array is one `json` leaf, and
    /// `SMY-W608` names it.
    #[test]
    fn an_unkeyable_array_is_one_leaf_and_w608() {
        let dup = S0.replace(
            r#"[{"name": "A", "value": "1"}]"#,
            r#"[{"name": "A", "value": "1"}, {"name": "A", "value": "2"}]"#,
        );
        let d = run(S0, &dup);
        assert_eq!(d.notes.len(), 1);
        assert_eq!(d.notes[0].code(), Code::W608);
        assert_eq!(d.notes[0].pointer().to_string(), "/spec/containers/web/env");
        // S₀'s keyed leaves were removed and S₁'s array is one new leaf.
        let ops: Vec<String> = d
            .units
            .iter()
            .map(|u| field(u, "tel:op").unwrap())
            .collect();
        assert!(ops.contains(&"add".to_string()) && ops.contains(&"remove".to_string()));
        let added = d
            .units
            .iter()
            .find(|u| field(u, "tel:op").as_deref() == Some("add"))
            .unwrap();
        assert_eq!(field(added, "tel:vtype").as_deref(), Some("json"));
        assert_eq!(
            field(added, "tel:to").as_deref(),
            Some(r#"[{"name":"A","value":"1"},{"name":"A","value":"2"}]"#)
        );
    }

    #[test]
    fn an_out_of_range_number_is_text_and_w613() {
        let d = run(r#"{"spec": {"big": 1}}"#, r#"{"spec": {"big": 1e999}}"#);
        assert_eq!(
            d.notes,
            vec![Note::OutOfRange(Pointer::parse("/spec/big").unwrap())]
        );
        assert_eq!(field(&d.units[0], "tel:vtype").as_deref(), Some("text"));
        assert_eq!(field(&d.units[0], "tel:to").as_deref(), Some("1e999"));
    }

    #[test]
    fn an_emptied_object_is_a_change() {
        let d = run(
            r#"{"spec": {"selector": {"app": "web"}}}"#,
            r#"{"spec": {"selector": {}}}"#,
        );
        let ops: Vec<(String, String)> = d
            .units
            .iter()
            .map(|u| {
                (
                    field(u, "tel:op").unwrap(),
                    field(u, "tel:pointer").unwrap(),
                )
            })
            .collect();
        assert_eq!(
            ops,
            [
                ("add".to_string(), "/spec/selector".to_string()),
                ("remove".to_string(), "/spec/selector/app".to_string()),
            ]
        );
    }

    /// The point of the whole procedure: one input, one set of uids.
    /// A snapshot changed only in noise stores the same bytes, so the same tid.
    #[test]
    fn a_part_is_the_filtered_keyed_object() {
        let p = profile();
        let (a, _) = part_text(&parse(S0).unwrap(), &p);
        assert!(!a.contains("managedFields") && !a.contains("resourceVersion"));
        assert!(a.contains(r#""containers":{"web":{"#), "{a}");
        let noisy = S0
            .replace(r#""resourceVersion": "1""#, r#""resourceVersion": "9""#)
            .replace(
                r#""lastHeartbeatTime": "t0""#,
                r#""lastHeartbeatTime": "t9""#,
            );
        let (b, _) = part_text(&parse(&noisy).unwrap(), &p);
        assert_eq!(a, b);
        assert_eq!(
            smysl_core::PartText::new(a.into_bytes()).tid,
            smysl_core::PartText::new(b.into_bytes()).tid
        );
    }

    #[test]
    fn the_diff_is_a_function_of_its_inputs() {
        let b = S0
            .replace("web:1.0", "web:1.1")
            .replace(r#""replicas": 3"#, r#""replicas": 4"#);
        let uids = |d: Diff| {
            d.units
                .iter()
                .map(smysl_core::canonical_uid)
                .collect::<Vec<_>>()
        };
        assert_eq!(uids(run(S0, &b)), uids(run(S0, &b)));
        assert_eq!(uids(run(S0, &b)).len(), 2);
    }
}
