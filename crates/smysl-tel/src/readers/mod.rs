//! Object readers (RFC SMYSL-3 §20.1, TL-P2): a platform's export in, objects out.
//!
//! A reader reads **files, never the network**. Collecting is the caller's job (`kubectl`, a
//! script, OKO for Zabbix), which keeps this crate pure and every reader replayable from a
//! fixture. What a reader returns is each selected object with the two names it is known by:
//!
//! - its **series key** (§6), the identity tuple an `x.tel/series` unit is built from — for a
//!   Kubernetes object `apiVersion/kind/namespace/name#uid`, for a Zabbix item
//!   `host:<host>/item:<key_>`;
//! - its **alias**, the library name its manifests chain under, one per object across time.
//!
//! The two differ on purpose. An alias names an object by what a person calls it, so a
//! Deployment deleted and recreated under the same name continues one manifest chain, and the
//! diff across the gap sees `/metadata/uid` change: a recreate (§6). The series key has the uid
//! in it, so the recreated object is a new series, linked to the old by that recreate.
//!
//! **An alias is always valid.** A-3's grammar is lowercase ASCII, digits, `-`, `_`, `.`, and
//! segments split by `/` or `:`, at most 128 bytes. A name that fits is used as it is; one that
//! does not — a Zabbix host named `Web 01`, a 253-character Kubernetes name — becomes the
//! reader's prefix and 26 characters of the series key's hash. Lowercasing would have been
//! shorter and wrong: `Web-01` and `web-01` would be one alias for two hosts.

use smysl_core::ids::is_alias;

use crate::json::{Json, JsonError};

pub mod k8s;
pub mod zabbix;

/// One object a reader selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The library alias its manifests chain under.
    pub alias: String,
    /// Its series key (§6), without a pointer: the object's own series.
    pub key: String,
    /// The object, as read. Never the unfiltered object stored: that is `part_text`'s.
    pub doc: Json,
    /// A watch event said the object was deleted. Its snapshot is the last one.
    pub deleted: bool,
}

/// What a read produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    pub objects: Vec<Object>,
    /// Objects in the input the profile did not select.
    pub skipped: usize,
}

/// Why an input could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    Json(JsonError),
    /// The JSON is not what this reader reads, and why.
    Shape(String),
}

impl core::fmt::Display for ReadError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReadError::Json(e) => write!(f, "{e}"),
            ReadError::Shape(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<JsonError> for ReadError {
    fn from(e: JsonError) -> ReadError {
        ReadError::Json(e)
    }
}

pub(crate) fn shape(why: impl Into<String>) -> ReadError {
    ReadError::Shape(why.into())
}

/// The alias for an object: `prefix/segment/…` if that is a valid alias, else `prefix/h/`
/// and 26 characters of the BLAKE3 of `key` in the format's base32.
pub fn alias_for(prefix: &str, segments: &[&str], key: &str) -> String {
    let plain = std::iter::once(prefix)
        .chain(segments.iter().copied())
        .collect::<Vec<_>>()
        .join("/");
    if is_alias(&plain) {
        plain
    } else {
        let h = smysl_core::hash_bytes(key.as_bytes());
        format!("{prefix}/h/{}", smysl_core::ids::base32(&h, 26))
    }
}

/// A string member, or `""` when absent or not a string.
pub(crate) fn text<'a>(v: &'a Json, key: &str) -> &'a str {
    v.get(key).and_then(Json::as_str).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_alias_is_the_name_when_it_can_be_and_a_hash_when_it_cannot() {
        assert_eq!(
            alias_for(
                "k8s",
                &["apps", "v1", "deployment", "shop", "checkout"],
                "k"
            ),
            "k8s/apps/v1/deployment/shop/checkout"
        );
        let a = alias_for("zabbix", &["host", "Web 01"], "host:Web 01");
        let b = alias_for("zabbix", &["host", "web-01"], "host:web-01");
        assert!(a.starts_with("zabbix/h/") && is_alias(&a), "{a}");
        assert_eq!(b, "zabbix/host/web-01");
        let c = alias_for("zabbix", &["host", "Web-01"], "host:Web-01");
        assert_ne!(c, b, "case is identity: two hosts, two aliases");
        let long = "x".repeat(200);
        assert!(alias_for("k8s", &[&long], &long).len() <= 128);
    }
}
