//! The key file, `.smysl/keys.hjson` (RFC SMYSL-7 §5), parsed from its text (R1-6).
//!
//! ```hjson
//! {
//!   keys: [
//!     { agent: "human:alice", ed25519: "hex:d75a9801…", status: "active" }
//!     { agent: "tool:ci", ed25519: "base64:MCowBQYDK2VwAyEA…", max_status: "measured" }
//!     { agent: "tool:old-bot", ed25519: "hex:3d4017c3…", status: "revoked",
//!       revoked_after_ms: 1762000000000, reason: "rotated in Q4" }
//!   ]
//! }
//! ```
//!
//! The parser is strict: an unknown key, a field the entry's status does not take, a
//! duplicate key, or a key bound to two agents is an error, not something skipped. A typo in
//! `revoked_after_ms` that dropped a revocation would fail open.

use std::fmt;

use smysl_core::surface::hjson::{self, HObject, HValue, Spanned};
use smysl_core::{AgentId, Status};

use crate::key::{KeyId, KeyStatus, MemoryKeyResolver, ResolvedKey, SigningKey};

/// The DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410 §4), as
/// `openssl pkey -pubout -outform DER` writes it; the 32-byte key follows.
pub const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

/// The key file's reader and writer.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct KeyFile;

/// A key file that did not parse, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct KeyFileError {
    /// The byte offset in the text.
    pub at: usize,
    /// The 1-based line of `at`.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for KeyFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for KeyFileError {}

const FIELDS: &[&str] = &[
    "agent",
    "ed25519",
    "kid",
    "status",
    "max_status",
    "revoked_after_ms",
    "reason",
    "not_before_ms",
    "not_after_ms",
];

impl KeyFile {
    /// Parse a key file's text into a resolver.
    pub fn parse(text: &str) -> Result<MemoryKeyResolver, KeyFileError> {
        let err = |at: usize, message: String| KeyFileError {
            at,
            line: text[..at.min(text.len())].matches('\n').count() + 1,
            message,
        };
        let root = hjson::parse_object(text, 0).map_err(|e| err(e.at, e.message))?;
        if let Some((k, _)) = root.value.iter().find(|(k, _)| k.value != "keys") {
            return Err(err(k.span.start, format!("unknown key `{}`", k.value)));
        }
        let Some(keys) = root.value.get("keys") else {
            return Err(err(root.span.start, "no `keys` array".into()));
        };
        let Some(entries) = keys.value.as_array() else {
            return Err(err(keys.span.start, "`keys` is not an array".into()));
        };
        let mut out = MemoryKeyResolver::new();
        for e in entries {
            let key = entry(e).map_err(|(at, m)| err(at, m))?;
            if let Some(other) = out.keys().find(|k| k.kid == key.kid) {
                let message = if other.agent == key.agent {
                    format!("key {} is listed twice for {}", key.kid, key.agent)
                } else {
                    format!(
                        "key {} is bound to {} and to {}",
                        key.kid, other.agent, key.agent
                    )
                };
                return Err(err(e.span.start, message));
            }
            out.insert(key);
        }
        Ok(out)
    }

    /// One key's entry, on one line, as `parse` reads it.
    pub fn entry(key: &ResolvedKey) -> String {
        let mut s = format!(
            "{{ agent: {}, ed25519: \"hex:{}\", kid: \"{}\"",
            quote(key.agent.as_str()),
            hex(&key.public_key),
            key.kid
        );
        match &key.status {
            KeyStatus::Active => {}
            KeyStatus::Revoked { after_ms, reason } => {
                s.push_str(", status: \"revoked\"");
                if let Some(t) = after_ms {
                    s.push_str(&format!(", revoked_after_ms: {t}"));
                }
                if let Some(r) = reason {
                    s.push_str(&format!(", reason: {}", quote(r)));
                }
            }
            KeyStatus::Bounded {
                not_before_ms,
                not_after_ms,
            } => s.push_str(&format!(
                ", status: \"bounded\", not_before_ms: {not_before_ms}, not_after_ms: {not_after_ms}"
            )),
        }
        if let Some(m) = key.max_status {
            s.push_str(&format!(", max_status: \"{}\"", m.as_str()));
        }
        s.push_str(" }");
        s
    }

    /// A signing key's file: its agent, its secret, and its id, as `smysl keygen` writes it.
    /// The text holds the secret; it is wiped when the returned value is dropped.
    pub fn secret(key: &SigningKey) -> zeroize::Zeroizing<String> {
        let seed = key.seed();
        zeroize::Zeroizing::new(format!(
            "# A smysl signing key (RFC SMYSL-7). Keep it private: whoever reads it signs as \
             {agent}.\n{{ agent: {q}, ed25519_secret: \"hex:{secret}\", kid: \"{kid}\" }}\n",
            agent = key.agent(),
            q = quote(key.agent().as_str()),
            secret = zeroize::Zeroizing::new(hex(&seed[..])).as_str(),
            kid = key.kid(),
        ))
    }

    /// Read a signing key's file. `kid`, when present, must be the key's.
    pub fn parse_secret(text: &str) -> Result<SigningKey, KeyFileError> {
        let err = |at: usize, message: String| KeyFileError {
            at,
            line: text[..at.min(text.len())].matches('\n').count() + 1,
            message,
        };
        let o = hjson::parse_object(text, 0).map_err(|e| err(e.at, e.message))?;
        if let Some((k, _)) = o
            .value
            .iter()
            .find(|(k, _)| !["agent", "ed25519_secret", "kid"].contains(&k.value.as_str()))
        {
            return Err(err(k.span.start, format!("unknown key `{}`", k.value)));
        }
        let at = o.span.start;
        let (agent_s, a_at) = string(&o.value, "agent")
            .map_err(|(at, m)| err(at, m))?
            .ok_or_else(|| err(at, "no `agent`".into()))?;
        let agent: AgentId = agent_s
            .parse()
            .map_err(|e| err(a_at, format!("`agent`: {e}")))?;
        let (secret, s_at) = string(&o.value, "ed25519_secret")
            .map_err(|(at, m)| err(at, m))?
            .ok_or_else(|| err(at, "no `ed25519_secret`".into()))?;
        let seed: zeroize::Zeroizing<[u8; 32]> = secret
            .strip_prefix("hex:")
            .and_then(unhex)
            .and_then(|v| {
                let v = zeroize::Zeroizing::new(v);
                <[u8; 32]>::try_from(&v[..]).ok()
            })
            .map(zeroize::Zeroizing::new)
            .ok_or_else(|| {
                err(
                    s_at,
                    "`ed25519_secret` is not `hex:` and 64 hex digits".into(),
                )
            })?;
        let key = SigningKey::from_seed(agent, *seed);
        if let Some((k, k_at)) = string(&o.value, "kid").map_err(|(at, m)| err(at, m))? {
            if k.to_ascii_lowercase() != key.kid().to_string() {
                return Err(err(
                    k_at,
                    format!("`kid` is {k}; the key's id is {}", key.kid()),
                ));
            }
        }
        Ok(key)
    }
}

type Res<T> = Result<T, (usize, String)>;

fn entry(v: &Spanned<HValue>) -> Res<ResolvedKey> {
    let Some(o) = v.value.as_object() else {
        return Err((v.span.start, "a key entry is not an object".into()));
    };
    if let Some((k, _)) = o.iter().find(|(k, _)| !FIELDS.contains(&k.value.as_str())) {
        return Err((k.span.start, format!("unknown key `{}`", k.value)));
    }
    let (agent_s, at) = string(o, "agent")?.ok_or((v.span.start, "no `agent`".to_string()))?;
    let agent: AgentId = agent_s.parse().map_err(|e| (at, format!("`agent`: {e}")))?;
    let (pk, at) = string(o, "ed25519")?.ok_or((v.span.start, "no `ed25519`".to_string()))?;
    let public_key = parse_public_key(pk).map_err(|m| (at, format!("`ed25519`: {m}")))?;
    let kid = KeyId::of(&public_key);
    if let Some((k, at)) = string(o, "kid")? {
        if k.to_ascii_lowercase() != kid.to_string() {
            return Err((at, format!("`kid` is {k}; the key's id is {kid}")));
        }
    }
    let status_name = string(o, "status")?;
    let only = |field: &str, status: &str| -> Res<()> {
        match o.get(field) {
            Some(_) if status_name.map(|s| s.0) != Some(status) => Err((
                o.key_span(field).map_or(v.span.start, |s| s.start),
                format!("`{field}` needs status: \"{status}\""),
            )),
            _ => Ok(()),
        }
    };
    only("revoked_after_ms", "revoked")?;
    only("reason", "revoked")?;
    only("not_before_ms", "bounded")?;
    only("not_after_ms", "bounded")?;
    let status = match status_name {
        None | Some(("active", _)) => KeyStatus::Active,
        Some(("revoked", _)) => KeyStatus::Revoked {
            after_ms: millis(o, "revoked_after_ms")?.map(|x| x.0),
            reason: string(o, "reason")?.map(|r| r.0.to_string()),
        },
        Some(("bounded", at)) => {
            let need = |f: &str| millis(o, f)?.ok_or((at, format!("status bounded needs `{f}`")));
            let (not_before_ms, _) = need("not_before_ms")?;
            let (not_after_ms, at) = need("not_after_ms")?;
            if not_after_ms < not_before_ms {
                return Err((at, "`not_after_ms` is before `not_before_ms`".into()));
            }
            KeyStatus::Bounded {
                not_before_ms,
                not_after_ms,
            }
        }
        Some((s, at)) => {
            return Err((
                at,
                format!("status `{s}`: expected active, revoked or bounded"),
            ))
        }
    };
    let max_status = match string(o, "max_status")? {
        None => None,
        Some((s, at)) => {
            Some(Status::parse(s).ok_or((at, format!("`max_status`: no status `{s}`")))?)
        }
    };
    Ok(ResolvedKey {
        kid,
        agent,
        public_key,
        status,
        max_status,
    })
}

fn string<'a>(o: &'a HObject, field: &str) -> Res<Option<(&'a str, usize)>> {
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

fn millis(o: &HObject, field: &str) -> Res<Option<(u64, usize)>> {
    match o.get(field) {
        None => Ok(None),
        Some(v) => match v.value {
            HValue::Int(n) if n >= 0 => Ok(Some((n as u64, v.span.start))),
            _ => Err((
                v.span.start,
                format!("`{field}` is not a count of milliseconds"),
            )),
        },
    }
}

/// A public key as the key file writes it: `hex:` and 64 hex digits, or `base64:` and the raw
/// 32 bytes or their SubjectPublicKeyInfo DER. A point that is not on the curve, or one of
/// small order, is refused.
pub fn parse_public_key(s: &str) -> Result<[u8; 32], String> {
    let bytes = if let Some(h) = s.strip_prefix("hex:") {
        unhex(h).ok_or("not 64 hex digits")?
    } else if let Some(b) = s.strip_prefix("base64:") {
        let raw = unbase64(b).ok_or("not base64")?;
        match raw.len() {
            32 => raw,
            44 if raw[..12] == ED25519_SPKI_PREFIX => raw[12..].to_vec(),
            44 => return Err("44 bytes, and not an Ed25519 SubjectPublicKeyInfo".into()),
            n => return Err(format!("{n} bytes; a key is 32, or 44 as DER")),
        }
    } else {
        return Err("expected `hex:` or `base64:`".into());
    };
    let key: [u8; 32] = bytes.try_into().map_err(|_| "not 32 bytes".to_string())?;
    match ed25519_dalek::VerifyingKey::from_bytes(&key) {
        Ok(vk) if !vk.is_weak() => Ok(key),
        Ok(_) => Err("a point of small order".into()),
        Err(_) => Err("not a point on the curve".into()),
    }
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if s.len() != 64 || !s.bytes().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Standard base64 (RFC 4648 §4), padded, with the unused bits zero: one encoding per value.
fn unbase64(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    if b.is_empty() || b.len() % 4 != 0 {
        return None;
    }
    let val = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    };
    let pad = b.iter().rev().take_while(|&&c| c == b'=').count();
    if pad > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(b.len() / 4 * 3);
    for (i, chunk) in b.chunks(4).enumerate() {
        let last = i == b.len() / 4 - 1;
        let n = if last { 4 - pad } else { 4 };
        let mut acc = 0u32;
        for (j, &c) in chunk.iter().enumerate() {
            acc <<= 6;
            if j < n {
                acc |= val(c)?;
            }
        }
        let bytes = acc.to_be_bytes();
        match n {
            4 => out.extend(&bytes[1..4]),
            3 if acc & 0xff == 0 => out.extend(&bytes[1..3]),
            2 if acc & 0xffff == 0 => out.push(bytes[1]),
            _ => return None,
        }
    }
    Some(out)
}

/// A JSON string literal.
fn quote(s: &str) -> String {
    let mut q = String::from('"');
    for c in s.chars() {
        match c {
            '"' => q.push_str("\\\""),
            '\\' => q.push_str("\\\\"),
            '\n' => q.push_str("\\n"),
            '\r' => q.push_str("\\r"),
            '\t' => q.push_str("\\t"),
            c if (c as u32) < 0x20 => q.push_str(&format!("\\u{:04x}", c as u32)),
            c => q.push(c),
        }
    }
    q.push('"');
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_reads_one_encoding_per_value() {
        assert_eq!(unbase64("TWFu").unwrap(), b"Man");
        assert_eq!(unbase64("TWE=").unwrap(), b"Ma");
        assert_eq!(unbase64("TQ==").unwrap(), b"M");
        assert!(unbase64("TR==").is_none(), "unused bits set");
        assert!(unbase64("TWE").is_none(), "unpadded");
        assert!(unbase64("T===").is_none());
        assert!(unbase64("TW=u").is_none(), "padding inside");
        assert!(unbase64("").is_none());
    }
}
