//! A project's signature policy, `trust:` in `.smysl/config.hjson` (RFC SMYSL-8 §5, R1-8).
//!
//! ```hjson
//! trust: {
//!   signatures: "selective"
//!   agent_prefixes: ["model:", "tool:"]
//!   status_at_least: "measured"
//!   keys: "keys/team.hjson"
//! }
//! ```
//!
//! Read from the configuration's text beside the provider settings, which `smysl-provider`
//! reads by key; the rest of the file is not this parser's business. Inside `trust:` it is
//! strict, as the key file is: an unknown or misplaced field is an error, because a typo in
//! `signatures` that dropped the policy would fail open.

use smysl_core::surface::hjson::{self, HObject, HValue};
use smysl_core::Status;

use crate::keyfile::KeyFileError;
use crate::verify::VerificationMode;

const FIELDS: [&str; 4] = ["signatures", "agent_prefixes", "status_at_least", "keys"];

/// A project's signature policy.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct TrustConfig {
    /// The mode trust decisions read signatures under.
    pub mode: VerificationMode,
    /// The key file, as written: relative to the project when relative. `None` for the default,
    /// `.smysl/keys.hjson`.
    pub keys: Option<String>,
}

impl TrustConfig {
    /// `trust:` from a configuration's text; `None` when the configuration has none.
    pub fn parse(text: &str) -> Result<Option<TrustConfig>, KeyFileError> {
        let err = |at: usize, message: String| KeyFileError {
            at,
            line: text[..at.min(text.len())].matches('\n').count() + 1,
            message,
        };
        let root = hjson::parse_object(text, 0).map_err(|e| err(e.at, e.message))?;
        let Some(t) = root.value.get("trust") else {
            return Ok(None);
        };
        let Some(o) = t.value.as_object() else {
            return Err(err(t.span.start, "`trust` is not an object".into()));
        };
        if let Some((k, _)) = o.iter().find(|(k, _)| !FIELDS.contains(&k.value.as_str())) {
            return Err(err(
                k.span.start,
                format!("unknown key `trust.{}`", k.value),
            ));
        }
        let field = |name: &str| -> Result<Option<(&str, usize)>, KeyFileError> {
            string(o, name).map_err(|(at, m)| err(at, m))
        };
        let (name, at) = field("signatures")?.ok_or_else(|| {
            err(
                t.span.start,
                "`trust` has no `signatures`: advisory, selective or strict".into(),
            )
        })?;
        let prefixes: Vec<String> = match o.get("agent_prefixes") {
            None => Vec::new(),
            Some(v) => {
                let Some(items) = v.value.as_array() else {
                    return Err(err(v.span.start, "`agent_prefixes` is not an array".into()));
                };
                let mut out = Vec::new();
                for i in items {
                    match &i.value {
                        HValue::Str(s) => out.push(s.clone()),
                        other => {
                            return Err(err(
                                i.span.start,
                                format!("an agent prefix is {}, not a string", other.type_name()),
                            ))
                        }
                    }
                }
                out
            }
        };
        let status = match field("status_at_least")? {
            None => None,
            Some((s, at)) => Some(
                Status::parse(s)
                    .ok_or_else(|| err(at, format!("`status_at_least`: no status `{s}`")))?,
            ),
        };
        let scoped = !prefixes.is_empty() || status.is_some();
        let mode = match name {
            "advisory" | "strict" if scoped => {
                return Err(err(
                    at,
                    format!("`agent_prefixes` and `status_at_least` scope selective, not {name}"),
                ))
            }
            "advisory" => VerificationMode::Advisory,
            "strict" => VerificationMode::Strict,
            "selective" if !scoped => {
                return Err(err(
                    at,
                    "selective needs `agent_prefixes`, `status_at_least`, or both".into(),
                ))
            }
            "selective" => VerificationMode::selective(prefixes, status),
            other => {
                return Err(err(
                    at,
                    format!("no mode `{other}`; the modes are advisory, selective and strict"),
                ))
            }
        };
        let keys = field("keys")?.map(|(k, _)| k.to_string());
        Ok(Some(TrustConfig { mode, keys }))
    }
}

fn string<'a>(o: &'a HObject, field: &str) -> Result<Option<(&'a str, usize)>, (usize, String)> {
    match o.get(field) {
        None => Ok(None),
        Some(v) => match &v.value {
            HValue::Str(s) => Ok(Some((s, v.span.start))),
            other => Err((
                v.span.start,
                format!("`{field}` is {}, not a string", other.type_name()),
            )),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_configuration_without_trust_has_no_policy() {
        let text = "{ providers: { local: { kind: \"ollama\" } } }";
        assert_eq!(TrustConfig::parse(text).unwrap(), None);
    }

    #[test]
    fn each_mode_reads() {
        let t = TrustConfig::parse("{ trust: { signatures: \"strict\" } }")
            .unwrap()
            .unwrap();
        assert_eq!(t.mode, VerificationMode::Strict);
        assert_eq!(t.keys, None);
        let t = TrustConfig::parse(
            "{ providers: {}, trust: { signatures: \"selective\", agent_prefixes: [\"model:\"], \
             status_at_least: \"measured\", keys: \"k.hjson\" } }",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            t.mode,
            VerificationMode::selective(["model:"], Some(Status::Measured))
        );
        assert_eq!(t.keys.as_deref(), Some("k.hjson"));
    }

    /// A typo must not drop the policy: it fails, with the line.
    #[test]
    fn a_misspelt_or_misplaced_field_is_refused() {
        for (text, needle) in [
            (
                "{ trust: { signature: \"strict\" } }",
                "unknown key `trust.signature`",
            ),
            ("{ trust: {} }", "no `signatures`"),
            ("{ trust: { signatures: \"strong\" } }", "no mode `strong`"),
            (
                "{ trust: { signatures: \"selective\" } }",
                "selective needs",
            ),
            (
                "{ trust: { signatures: \"strict\", agent_prefixes: [\"x:\"] } }",
                "scope selective",
            ),
            ("{\n trust: \"strict\" }", "not an object"),
        ] {
            let e = TrustConfig::parse(text).unwrap_err();
            assert!(e.message.contains(needle), "{text}: {}", e.message);
        }
        let e = TrustConfig::parse("{\n\n trust: { signatures: 3 } }").unwrap_err();
        assert_eq!(e.line, 3);
    }
}
