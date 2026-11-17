//! Reader profiles (RFC SMYSL-3 §20.2): what a snapshot's leaves are, and which are noise.
//!
//! A profile is an HJSON file, the configuration format smysl uses elsewhere:
//!
//! ```hjson
//! {
//!   profile: "k8s-deployments/1"
//!   scheme:  k8s
//!   select:  { apiVersion: "apps/v1", kind: "Deployment" }
//!   noise:   [ "/metadata/managedFields", "/metadata/resourceVersion" ]
//!   keys:    { "/spec/template/spec/containers": "name" }
//!   classes: { "/spec/replicas": "config", "/metadata/labels": "membership" }
//!   ints:    [ "/spec/replicas" ]
//!   route:   { gauge: series }
//! }
//! ```
//!
//! **`ints` is this module's, not the RFC's example's.** §5.5 makes a number `int` when the
//! source format types the field as an integer — a Kubernetes field whose OpenAPI type is
//! `integer` — and `dec` otherwise. A snapshot does not say which, and the RFC's profile example
//! has nowhere to; so a profile lists the pointers it types as integers, and every other number
//! is a `dec`. It is inside the profile's hash, so a profile that changed its mind is a
//! different profile (`SMY-W610`).
//!
//! **The hash** is `hash_bytes` over the canonical CBOR of the parsed profile, as a measure
//! recipe's is, and it is spelled `b3:` like one. It is a function of what the profile *means*,
//! not of how it was written: patterns are re-spelled canonically, the collections are sorted,
//! and comments, whitespace and key order are gone. Empty collections are omitted, so a field
//! added in a later revision does not move the hash of a profile that does not use it.
//!
//! **Classification** (§8.1 step 4) takes the longest class pattern that covers a pointer. The
//! RFC stops there, and two patterns of one length can both cover a pointer (`/a/*` and `/*/b`
//! cover `/a/b`), so the tie is broken here, deterministically: fewer `*` segments wins — the
//! more specific pattern — and then the pattern whose canonical text sorts first.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schemas::tel::{CLASSES, SCHEMES};
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::Uid;

use crate::json::Json;
use crate::pointer::{Pattern, Pointer, Seg};

/// Where a class's leaves go instead of becoming units (§20.1). `series` is the one
/// destination TL-P2 knows: the substrate (TL-P3).
pub const ROUTES: &[&str] = &["series"];

/// A parsed, validated profile.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
    /// Its name and revision, as `k8s-deployments/1`.
    pub name: String,
    pub scheme: String,
    /// Top-level fields an object must carry, with these string values, to be this profile's.
    pub select: BTreeMap<String, String>,
    pub noise: BTreeSet<Pattern>,
    /// Array pattern → the field that keys its elements (§8.1 step 2).
    pub keys: BTreeMap<Pattern, String>,
    pub classes: BTreeMap<Pattern, String>,
    pub ints: BTreeSet<Pattern>,
    pub route: BTreeMap<String, String>,
}

/// Why a profile was refused, naming the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileError {
    pub field: String,
    pub why: String,
}

impl core::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "profile `{}`: {}", self.field, self.why)
    }
}

impl std::error::Error for ProfileError {}

fn err(field: &str, why: impl Into<String>) -> ProfileError {
    ProfileError {
        field: field.to_string(),
        why: why.into(),
    }
}

fn string<'a>(field: &str, v: &'a HValue) -> Result<&'a str, ProfileError> {
    match v {
        HValue::Str(s) => Ok(s),
        other => Err(err(
            field,
            format!("expected text, found {}", other.type_name()),
        )),
    }
}

fn object<'a>(field: &str, v: &'a HValue) -> Result<&'a HObject, ProfileError> {
    match v {
        HValue::Object(o) => Ok(o),
        other => Err(err(
            field,
            format!("expected an object, found {}", other.type_name()),
        )),
    }
}

fn pattern(field: &str, text: &str) -> Result<Pattern, ProfileError> {
    Pattern::parse(text).map_err(|e| err(field, e.to_string()))
}

fn patterns(field: &str, v: &HValue) -> Result<BTreeSet<Pattern>, ProfileError> {
    let HValue::Array(items) = v else {
        return Err(err(
            field,
            format!("expected a list, found {}", v.type_name()),
        ));
    };
    items
        .iter()
        .map(|i| pattern(field, string(field, &i.value)?))
        .collect()
}

impl Profile {
    /// Parse a profile's HJSON text. Unknown fields are refused: a profile is a contract, and a
    /// misspelt `clases:` silently ignored would leave every leaf a `gauge`.
    pub fn parse(text: &str) -> Result<Profile, ProfileError> {
        let o = smysl_core::surface::parse_object(text, 0)
            .map_err(|e| err("(document)", e.to_string()))?
            .value;
        Profile::from_object(&o)
    }

    pub fn from_object(o: &HObject) -> Result<Profile, ProfileError> {
        let mut p = Profile {
            name: String::new(),
            scheme: String::new(),
            select: BTreeMap::new(),
            noise: BTreeSet::new(),
            keys: BTreeMap::new(),
            classes: BTreeMap::new(),
            ints: BTreeSet::new(),
            route: BTreeMap::new(),
        };
        for (k, v) in o.iter() {
            let field = k.value.as_str();
            let v = &v.value;
            match field {
                "profile" => p.name = string(field, v)?.to_string(),
                "scheme" => {
                    let s = string(field, v)?;
                    if !SCHEMES.contains(&s) {
                        return Err(err(field, format!("`{s}` is not a scheme: {SCHEMES:?}")));
                    }
                    p.scheme = s.to_string();
                }
                "select" => {
                    for (sk, sv) in object(field, v)?.iter() {
                        p.select
                            .insert(sk.value.clone(), string(field, &sv.value)?.to_string());
                    }
                }
                "noise" => p.noise = patterns(field, v)?,
                "ints" => p.ints = patterns(field, v)?,
                "keys" => {
                    for (pk, pv) in object(field, v)?.iter() {
                        p.keys.insert(
                            pattern(field, &pk.value)?,
                            string(field, &pv.value)?.to_string(),
                        );
                    }
                }
                "classes" => {
                    for (pk, pv) in object(field, v)?.iter() {
                        let class = string(field, &pv.value)?;
                        if !CLASSES.contains(&class) {
                            return Err(err(
                                field,
                                format!("`{class}` is not a leaf class: {CLASSES:?}"),
                            ));
                        }
                        p.classes
                            .insert(pattern(field, &pk.value)?, class.to_string());
                    }
                }
                "route" => {
                    for (rk, rv) in object(field, v)?.iter() {
                        let (class, to) = (rk.value.as_str(), string(field, &rv.value)?);
                        if !CLASSES.contains(&class) {
                            return Err(err(field, format!("`{class}` is not a leaf class")));
                        }
                        if !ROUTES.contains(&to) {
                            return Err(err(
                                field,
                                format!("`{to}` is not a destination: {ROUTES:?}"),
                            ));
                        }
                        p.route.insert(class.to_string(), to.to_string());
                    }
                }
                other => return Err(err(other, "not a profile field")),
            }
        }
        if p.name.is_empty() {
            return Err(err("profile", "a profile names itself"));
        }
        if p.scheme.is_empty() {
            return Err(err("scheme", "a profile names its scheme"));
        }
        Ok(p)
    }

    /// The profile as a canonical object: what it means, nothing about how it was written.
    pub fn to_object(&self) -> HObject {
        let span = smysl_core::Span::new(0, 0);
        let at = |v| Spanned::new(v, span);
        let text = |s: &str| at(HValue::Str(s.to_string()));
        let mut o = HObject::new();
        let mut put = |k: &str, v: Spanned<HValue>| o.insert(Spanned::new(k.to_string(), span), v);
        put("profile", text(&self.name));
        put("scheme", text(&self.scheme));
        let map = |m: Vec<(String, String)>| {
            let mut inner = HObject::new();
            for (k, v) in m {
                inner.insert(Spanned::new(k, span), Spanned::new(HValue::Str(v), span));
            }
            Spanned::new(HValue::Object(inner), span)
        };
        let list = |s: &BTreeSet<Pattern>| {
            let mut v: Vec<String> = s.iter().map(ToString::to_string).collect();
            v.sort();
            Spanned::new(
                HValue::Array(
                    v.into_iter()
                        .map(|t| Spanned::new(HValue::Str(t), span))
                        .collect(),
                ),
                span,
            )
        };
        if !self.select.is_empty() {
            put("select", map(self.select.clone().into_iter().collect()));
        }
        if !self.noise.is_empty() {
            put("noise", list(&self.noise));
        }
        if !self.ints.is_empty() {
            put("ints", list(&self.ints));
        }
        if !self.keys.is_empty() {
            put(
                "keys",
                map(self
                    .keys
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect()),
            );
        }
        if !self.classes.is_empty() {
            put(
                "classes",
                map(self
                    .classes
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.clone()))
                    .collect()),
            );
        }
        if !self.route.is_empty() {
            put("route", map(self.route.clone().into_iter().collect()));
        }
        o
    }

    /// The canonical CBOR of [`Profile::to_object`].
    pub fn to_cbor(&self) -> Vec<u8> {
        smysl_core::surface::object_to_payload(&self.to_object()).unwrap_or_default()
    }

    /// The profile's hash: what every recipe that uses it records (§20.2).
    pub fn hash(&self) -> [u8; 32] {
        smysl_core::hash_bytes(&self.to_cbor())
    }

    /// [`Profile::hash`] as `b3:` text, the form a recipe's hash takes.
    pub fn hash_text(&self) -> String {
        Uid::from_bytes(self.hash()).canonical()
    }

    /// Whether an object is this profile's: every `select` field present and, unless the wanted
    /// value is `*`, a string equal to it.
    ///
    /// `*` means "present, with any value". A Kubernetes object says what it is (`kind`), and a
    /// Zabbix one does not: a host is the object with a `host` field and an item the one with a
    /// `key_`, so `zabbix-items/1` selects `{ key_: "*" }`. A literal `*` value cannot be
    /// selected, which no object this reads would want.
    pub fn selects(&self, object: &Json) -> bool {
        self.select.iter().all(|(k, want)| match object.get(k) {
            None => false,
            Some(_) if want == "*" => true,
            Some(v) => v.as_str() == Some(want.as_str()),
        })
    }

    /// Whether a leaf is noise, removed before anything is stored (§8.3). A noise pattern on
    /// an object or array removes everything under it.
    pub fn is_noise(&self, p: &Pointer) -> bool {
        self.noise.iter().any(|pat| pat.covers(p))
    }

    /// Whether the source types this leaf as an integer (§5.5).
    pub fn is_int(&self, p: &Pointer) -> bool {
        self.ints.iter().any(|pat| pat.matches(p))
    }

    /// The field that keys the elements of the array at `p`, if the profile declares one.
    pub fn key_field(&self, p: &Pointer) -> Option<&str> {
        // The most specific declaration, by the classification tie-break, so that two
        // patterns over one array cannot leave the keying to map order.
        self.keys
            .iter()
            .filter(|(pat, _)| pat.matches(p))
            .min_by_key(|(pat, _)| specificity(pat))
            .map(|(_, field)| field.as_str())
    }

    /// The class of the leaf at `p` (§8.1 step 4): the longest covering class pattern, ties to
    /// the fewer `*`s and then the first in canonical order; `gauge` if none covers it.
    pub fn class_of(&self, p: &Pointer) -> &str {
        self.classes
            .iter()
            .filter(|(pat, _)| pat.covers(p))
            .min_by_key(|(pat, _)| specificity(pat))
            .map_or("gauge", |(_, class)| class.as_str())
    }

    /// Where a class's leaves go instead of becoming units, if anywhere.
    pub fn route_of(&self, class: &str) -> Option<&str> {
        self.route.get(class).map(String::as_str)
    }
}

/// The profiles that ship (RFC SMYSL-3 §20.2), by name, as HJSON in `profiles/`.
///
/// Compiled in, so a profile is a name a recipe can record and a reader can resolve with no
/// file beside it. ConfigMaps ("keys only by default") are not here: keeping a key while
/// hiding its value is something a profile cannot yet say.
pub const BUILTIN: &[(&str, &str)] = &[
    (
        "k8s-deployments/1",
        include_str!("../profiles/k8s-deployments.hjson"),
    ),
    (
        "k8s-statefulsets/1",
        include_str!("../profiles/k8s-statefulsets.hjson"),
    ),
    (
        "k8s-daemonsets/1",
        include_str!("../profiles/k8s-daemonsets.hjson"),
    ),
    ("k8s-pods/1", include_str!("../profiles/k8s-pods.hjson")),
    (
        "k8s-services/1",
        include_str!("../profiles/k8s-services.hjson"),
    ),
    ("k8s-nodes/1", include_str!("../profiles/k8s-nodes.hjson")),
    (
        "zabbix-hosts/1",
        include_str!("../profiles/zabbix-hosts.hjson"),
    ),
    (
        "zabbix-items/1",
        include_str!("../profiles/zabbix-items.hjson"),
    ),
];

/// A shipped profile by name.
pub fn builtin(name: &str) -> Option<Profile> {
    BUILTIN
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, text)| Profile::parse(text).expect("a shipped profile parses; a test says so"))
}

/// The classification order as a key to minimise: longest first, then fewest `*`, then
/// canonical text.
fn specificity(p: &Pattern) -> (core::cmp::Reverse<usize>, usize, String) {
    let stars = p.0.iter().filter(|s| matches!(s, Seg::Any)).count();
    (core::cmp::Reverse(p.len()), stars, p.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §20.2's example, with the `ints` this module adds.
    const DEPLOYMENTS: &str = r#"{
  profile: "k8s-deployments/1"
  scheme:  k8s
  select:  { apiVersion: "apps/v1", kind: "Deployment" }
  noise:   [ "/metadata/managedFields", "/metadata/resourceVersion" ]
  keys:    { "/spec/template/spec/containers": "name", "/spec/template/spec/containers/*/env": "name" }
  classes: {
    "/spec/replicas": config
    "/spec/template/spec/containers/*/image": config
    "/metadata/labels": membership
    "/metadata/uid": identity
    "/status/readyReplicas": gauge
  }
  ints:    [ "/spec/replicas", "/status/readyReplicas" ]
  // gauges go to the substrate, never to units
  route:   { gauge: series }
}"#;

    fn ptr(t: &str) -> Pointer {
        Pointer::parse(t).unwrap()
    }

    #[test]
    fn the_rfc_example_parses() {
        let p = Profile::parse(DEPLOYMENTS).expect("a profile");
        assert_eq!(p.name, "k8s-deployments/1");
        assert_eq!(p.scheme, "k8s");
        assert_eq!(p.class_of(&ptr("/spec/replicas")), "config");
        assert_eq!(p.class_of(&ptr("/metadata/labels/app")), "membership");
        assert_eq!(
            p.class_of(&ptr("/spec/template/spec/containers/web/image")),
            "config"
        );
        assert_eq!(
            p.class_of(&ptr("/status/availableReplicas")),
            "gauge",
            "undeclared"
        );
        assert!(p.is_noise(&ptr("/metadata/managedFields/0/manager")));
        assert!(!p.is_noise(&ptr("/metadata/name")));
        assert!(p.is_int(&ptr("/spec/replicas")));
        assert_eq!(
            p.key_field(&ptr("/spec/template/spec/containers")),
            Some("name")
        );
        assert_eq!(
            p.key_field(&ptr("/spec/template/spec/containers/web/env")),
            Some("name")
        );
        assert_eq!(p.route_of("gauge"), Some("series"));
        assert_eq!(p.route_of("config"), None);
    }

    #[test]
    fn selection_is_by_top_level_strings() {
        let p = Profile::parse(DEPLOYMENTS).unwrap();
        let yes = crate::json::parse(r#"{"apiVersion":"apps/v1","kind":"Deployment"}"#).unwrap();
        let no = crate::json::parse(r#"{"apiVersion":"apps/v1","kind":"StatefulSet"}"#).unwrap();
        assert!(p.selects(&yes));
        assert!(!p.selects(&no));
        let items =
            Profile::parse(r#"{ profile: "z/1", scheme: zabbix, select: { key_: "*" } }"#).unwrap();
        assert!(items.selects(&crate::json::parse(r#"{"key_": "system.cpu.load"}"#).unwrap()));
        assert!(
            items.selects(&crate::json::parse(r#"{"key_": 3}"#).unwrap()),
            "any value"
        );
        assert!(!items.selects(&crate::json::parse(r#"{"host": "web-1"}"#).unwrap()));
    }

    /// The hash is a function of meaning: comments, whitespace, key order and pattern spelling
    /// do not move it, and a changed class does.
    #[test]
    fn the_hash_is_what_the_profile_means() {
        let a = Profile::parse(DEPLOYMENTS).unwrap();
        let reordered = r#"{
  route: { gauge: series }, ints: ["/status/readyReplicas", "/spec/replicas"]
  scheme: k8s, profile: "k8s-deployments/1"
  classes: { "/metadata/uid": identity, "/status/readyReplicas": gauge, "/metadata/labels": membership,
             "/spec/template/spec/containers/*/image": config, "/spec/replicas": config }
  keys: { "/spec/template/spec/containers/*/env": "name", "/spec/template/spec/containers": "name" }
  noise: [ "/metadata/resourceVersion", "/metadata/managedFields" ]
  select: { kind: "Deployment", apiVersion: "apps/v1" }
}"#;
        let b = Profile::parse(reordered).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.hash(), b.hash());
        assert!(a.hash_text().starts_with("b3:"));
        let changed =
            DEPLOYMENTS.replace(r#""/metadata/uid": identity"#, r#""/metadata/uid": config"#);
        assert_ne!(Profile::parse(&changed).unwrap().hash(), a.hash());
    }

    #[test]
    fn a_tie_goes_to_the_more_specific_pattern_then_to_canonical_order() {
        let p = Profile::parse(
            r#"{ profile: "t/1", scheme: json, classes: { "/a/*": config, "/*/b": membership, "/a/b/c": event } }"#,
        )
        .unwrap();
        // Two patterns of length two, one star each: canonical order, `/*/b` before `/a/*`.
        assert_eq!(p.class_of(&ptr("/a/b")), "membership");
        // A longer pattern wins outright.
        assert_eq!(p.class_of(&ptr("/a/b/c")), "event");
        let q = Profile::parse(
            r#"{ profile: "t/1", scheme: json, classes: { "/a/*": config, "/a/b": membership } }"#,
        )
        .unwrap();
        assert_eq!(q.class_of(&ptr("/a/b")), "membership", "fewer stars");
    }

    /// Every shipped profile parses, is the name it is listed under, and is selective: a
    /// profile that selected everything would read every object as its kind.
    #[test]
    fn the_shipped_profiles_are_what_they_say() {
        for (name, text) in BUILTIN {
            let p = Profile::parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(p.name, *name);
            assert!(!p.select.is_empty(), "{name} selects everything");
            if p.scheme == "k8s" {
                // §8.3's MUST list, on every Kubernetes profile.
                for must in [
                    "/metadata/managedFields",
                    "/metadata/resourceVersion",
                    "/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration",
                    "/status/conditions/*/lastHeartbeatTime",
                    "/status/conditions/*/lastProbeTime",
                ] {
                    assert!(
                        p.noise.contains(&Pattern::parse(must).unwrap()),
                        "{name} does not filter {must}"
                    );
                }
            }
        }
        assert!(builtin("k8s-deployments/1").is_some());
        assert!(builtin("k8s-configmaps/1").is_none());
        // The root pattern classifies everything a Zabbix export holds.
        let hosts = builtin("zabbix-hosts/1").unwrap();
        assert_eq!(hosts.class_of(&ptr("/status")), "config");
        assert_eq!(hosts.class_of(&ptr("/host")), "identity");
    }

    #[test]
    fn a_profile_is_a_contract() {
        for (text, field) in [
            (r#"{ profile: "x/1", scheme: k8s, clases: {} }"#, "clases"),
            (r#"{ profile: "x/1", scheme: graphite }"#, "scheme"),
            (r#"{ scheme: k8s }"#, "profile"),
            (
                r#"{ profile: "x/1", scheme: k8s, classes: { "/a": hot } }"#,
                "classes",
            ),
            (
                r#"{ profile: "x/1", scheme: k8s, noise: [ "a" ] }"#,
                "noise",
            ),
            (
                r#"{ profile: "x/1", scheme: k8s, route: { gauge: kafka } }"#,
                "route",
            ),
        ] {
            let e = Profile::parse(text).expect_err(text);
            assert_eq!(e.field, field, "{text}: {e}");
        }
    }
}
