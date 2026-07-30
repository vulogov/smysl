//! Identifiers (§1.2, §2.1).
//!
//! Every identifier here is validated on construction and infallible thereafter, which is
//! what lets the encoder be infallible: an id that exists is an id that encodes.

use core::fmt;
use core::str::FromStr;

use crate::error::{IdError, IntegrityError};

/// RFC 4648 base32, lowercased. Not base32hex - the alphabet is the standard one.
const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn decode_char(c: u8) -> Option<u8> {
    ALPHABET.iter().position(|&a| a == c).map(|p| p as u8)
}

// ---------------------------------------------------------------------------
// Uid
// ---------------------------------------------------------------------------

/// `uid = BLAKE3-256(det_cbor(core))` (rule P1).
///
/// A `Uid` always carries all 256 bits. The 26-character short form is for interactive
/// display and prefix resolution only; a canonical record carrying a truncated uid is
/// `SMY-E071`, and comparison is always over the full 256 bits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uid([u8; 32]);

impl Uid {
    /// Characters in the short display form: the first 130 bits, exactly 26 base32 chars.
    pub const SHORT_CHARS: usize = 26;
    /// Characters in the canonical display form: all 256 bits, zero-padded to 260.
    pub const FULL_CHARS: usize = 52;
    /// The textual prefix that marks a uid as BLAKE3.
    pub const PREFIX: &'static str = "b3:";

    pub const fn from_bytes(b: [u8; 32]) -> Uid {
        Uid(b)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub const fn to_bytes(self) -> [u8; 32] {
        self.0
    }

    /// `b3:` + 26 base32 characters (the first 130 bits). Display form; not canonical.
    pub fn short(&self) -> String {
        self.encode(Uid::SHORT_CHARS)
    }

    /// `b3:` + 52 base32 characters (all 256 bits). The canonical text form.
    pub fn canonical(&self) -> String {
        self.encode(Uid::FULL_CHARS)
    }

    fn encode(&self, chars: usize) -> String {
        let mut s = String::with_capacity(Uid::PREFIX.len() + chars);
        s.push_str(Uid::PREFIX);
        for i in 0..chars {
            s.push(ALPHABET[five_bits_at(&self.0, i * 5)] as char);
        }
        s
    }

    /// Parse a canonical, full-width uid. A shorter form is `SMY-E071`: it is a display
    /// abbreviation, and accepting it in a record would silently weaken identity.
    pub fn parse(s: &str) -> Result<Uid, IntegrityError> {
        let body = s
            .strip_prefix(Uid::PREFIX)
            .ok_or_else(|| IntegrityError::TruncatedUid {
                found: s.to_string(),
            })?;
        if body.len() != Uid::FULL_CHARS {
            return Err(IntegrityError::TruncatedUid {
                found: s.to_string(),
            });
        }
        let mut bytes = [0u8; 32];
        for (i, c) in body.bytes().enumerate() {
            let v = decode_char(c).ok_or_else(|| IntegrityError::TruncatedUid {
                found: s.to_string(),
            })?;
            put_five_bits(&mut bytes, i * 5, v);
        }
        Ok(Uid(bytes))
    }
}

/// The 5 bits starting at bit offset `off`, MSB-first, zero-padded past the end.
fn five_bits_at(bytes: &[u8; 32], off: usize) -> usize {
    let mut v = 0usize;
    for k in 0..5 {
        let bit = off + k;
        let set = bit < 256 && (bytes[bit / 8] >> (7 - (bit % 8))) & 1 == 1;
        v = (v << 1) | usize::from(set);
    }
    v
}

/// Write 5 bits at bit offset `off`, MSB-first. Bits past 256 are discarded, which is
/// what makes the 52nd character's four padding bits inert.
fn put_five_bits(bytes: &mut [u8; 32], off: usize, v: u8) {
    for k in 0..5 {
        let bit = off + k;
        if bit >= 256 {
            return;
        }
        if (v >> (4 - k)) & 1 == 1 {
            bytes[bit / 8] |= 1 << (7 - (bit % 8));
        }
    }
}

/// Display is the short form - the one a human reads in a diagnostic.
impl fmt::Display for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.short())
    }
}

impl fmt::Debug for Uid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uid({})", self.canonical())
    }
}

impl FromStr for Uid {
    type Err = IntegrityError;
    fn from_str(s: &str) -> Result<Uid, IntegrityError> {
        Uid::parse(s)
    }
}

/// An abbreviated uid, for interactive resolution only.
///
/// A prefix is never an identity. Resolving one against a store MUST report ambiguity
/// (`SMY-E072`) rather than pick a winner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UidPrefix {
    bits: usize,
    bytes: [u8; 32],
}

impl UidPrefix {
    /// Parse `b3:` plus 26-52 base32 characters.
    pub fn parse(s: &str) -> Result<UidPrefix, IntegrityError> {
        let body = s
            .strip_prefix(Uid::PREFIX)
            .ok_or_else(|| IntegrityError::TruncatedUid {
                found: s.to_string(),
            })?;
        if !(Uid::SHORT_CHARS..=Uid::FULL_CHARS).contains(&body.len()) {
            return Err(IntegrityError::TruncatedUid {
                found: s.to_string(),
            });
        }
        let mut bytes = [0u8; 32];
        for (i, c) in body.bytes().enumerate() {
            let v = decode_char(c).ok_or_else(|| IntegrityError::TruncatedUid {
                found: s.to_string(),
            })?;
            put_five_bits(&mut bytes, i * 5, v);
        }
        Ok(UidPrefix {
            bits: (body.len() * 5).min(256),
            bytes,
        })
    }

    pub fn matches(&self, uid: &Uid) -> bool {
        let full = self.bits / 8;
        let rest = self.bits % 8;
        if uid.0[..full] != self.bytes[..full] {
            return false;
        }
        if rest == 0 {
            return true;
        }
        let mask = 0xFFu8 << (8 - rest);
        uid.0[full] & mask == self.bytes[full] & mask
    }

    pub const fn bits(&self) -> usize {
        self.bits
    }
}

// ---------------------------------------------------------------------------
// String-shaped identifiers
// ---------------------------------------------------------------------------

/// The trailing segment of an extension id.
///
/// Appendix A writes `ext-type = "x." , ident , "/" , ident`, but the RFC's own examples
/// use `x.sre/1` - a schema id whose second segment is a version. Versions are accepted
/// here: a leading digit and internal dots are allowed after the slash, and nowhere else.
pub(crate) fn is_ext_segment(s: &str) -> bool {
    let mut cs = s.bytes();
    match cs.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'-' | b'_' | b'.'))
}

/// `ident = ALPHA , { ALPHA | DIGIT | "-" | "_" }` (Appendix A), lowercase.
fn is_ident(s: &str) -> bool {
    let mut cs = s.bytes();
    match cs.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    cs.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
}

macro_rules! label_shaped {
    ($name:ident, $kind:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Result<$name, IdError> {
                let s = s.into();
                let valid = match s.split_once('/') {
                    Some((a, b)) => is_ident(a) && is_ident(b),
                    None => false,
                };
                if valid {
                    Ok($name(s))
                } else {
                    Err(IdError {
                        kind: $kind,
                        found: s,
                    })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }

            /// The namespace segment before the `/`.
            pub fn namespace(&self) -> &str {
                self.0.split_once('/').map(|(a, _)| a).unwrap_or(&self.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.pad(&self.0)
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<$name, IdError> {
                $name::new(s)
            }
        }
    };
}

label_shaped!(
    Label,
    "label",
    "A view-scoped alias such as `c/auth-p95`.\n\nLabels are not identity: they are not hashed, and the same label bound to different\nuids across views in scope is a contention (§5.4)."
);
label_shaped!(
    ThreadId,
    "thread id",
    "A thread identifier such as `t/brief`."
);
label_shaped!(ViewId, "view id", "A view identifier such as `v/incident`.");
label_shaped!(
    ContentionId,
    "contention id",
    "A contention identifier such as `k/pool-vs-index`."
);

/// `^(model|human|tool):[a-z0-9._-]+(/[a-z0-9._:-]+)?$` (§1.2).
///
/// Self-asserted until COSE signing lands (N9). Corroboration is gameable by fabricated
/// attestations until then, which is why low-trust deployments set `w_r = 0` (D-6).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId(String);

/// The kind an `AgentId` declares before the colon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum AgentKind {
    Model,
    Human,
    Tool,
}

impl AgentKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            AgentKind::Model => "model",
            AgentKind::Human => "human",
            AgentKind::Tool => "tool",
        }
    }
}

impl AgentId {
    pub fn new(s: impl Into<String>) -> Result<AgentId, IdError> {
        let s = s.into();
        if AgentId::is_valid(&s) {
            Ok(AgentId(s))
        } else {
            Err(IdError {
                kind: "agent id",
                found: s,
            })
        }
    }

    fn is_valid(s: &str) -> bool {
        let Some((kind, rest)) = s.split_once(':') else {
            return false;
        };
        if !matches!(kind, "model" | "human" | "tool") {
            return false;
        }
        let (head, tail) = match rest.split_once('/') {
            Some((h, t)) => (h, Some(t)),
            None => (rest, None),
        };
        let head_ok = !head.is_empty()
            && head.bytes().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
            });
        let tail_ok = match tail {
            None => true,
            Some(t) => {
                !t.is_empty()
                    && t.bytes().all(|c| {
                        c.is_ascii_lowercase()
                            || c.is_ascii_digit()
                            || matches!(c, b'.' | b'_' | b'-' | b':' | b'/')
                    })
            }
        };
        head_ok && tail_ok
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn kind(&self) -> AgentKind {
        match self.0.split_once(':').map(|(k, _)| k) {
            Some("human") => AgentKind::Human,
            Some("tool") => AgentKind::Tool,
            _ => AgentKind::Model,
        }
    }

    /// The provider segment of a model agent, e.g. `anthropic` in
    /// `model:anthropic/claude-opus-5`. Corroboration groups by this (§16.4).
    pub fn provider(&self) -> Option<&str> {
        let rest = self.0.split_once(':')?.1;
        rest.split_once('/').map(|(p, _)| p)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

impl FromStr for AgentId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<AgentId, IdError> {
        AgentId::new(s)
    }
}

// ---------------------------------------------------------------------------
// Schema ids and kernel types
// ---------------------------------------------------------------------------

/// The closed kernel type set of §2.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum KernelType {
    Claim,
    Evidence,
    Definition,
    Question,
    Hypothesis,
    Finding,
    Procedure,
    Decision,
    Constraint,
    Observation,
    Data,
    ArtifactRef,
    Prose,
    Contention,
    PackInfo,
}

impl KernelType {
    pub const ALL: &'static [KernelType] = &[
        KernelType::Claim,
        KernelType::Evidence,
        KernelType::Definition,
        KernelType::Question,
        KernelType::Hypothesis,
        KernelType::Finding,
        KernelType::Procedure,
        KernelType::Decision,
        KernelType::Constraint,
        KernelType::Observation,
        KernelType::Data,
        KernelType::ArtifactRef,
        KernelType::Prose,
        KernelType::Contention,
        KernelType::PackInfo,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            KernelType::Claim => "claim",
            KernelType::Evidence => "evidence",
            KernelType::Definition => "definition",
            KernelType::Question => "question",
            KernelType::Hypothesis => "hypothesis",
            KernelType::Finding => "finding",
            KernelType::Procedure => "procedure",
            KernelType::Decision => "decision",
            KernelType::Constraint => "constraint",
            KernelType::Observation => "observation",
            KernelType::Data => "data",
            KernelType::ArtifactRef => "artifact-ref",
            KernelType::Prose => "prose",
            KernelType::Contention => "contention",
            KernelType::PackInfo => "packinfo",
        }
    }

    pub fn parse(s: &str) -> Option<KernelType> {
        KernelType::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

impl fmt::Display for KernelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// What a unit is: a kernel type, or an extension type `x.<domain>/<type>` (§2.2).
///
/// This is `UnitCore.schema`, key 0 of the hashed record. Extension types carry their
/// structure in `payload` and MUST NOT alter kernel field semantics.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SchemaId {
    /// A kernel unit type: `claim`, `evidence`, and the rest of §2.1.
    Kernel(KernelType),
    /// The kernel schema itself, `smysl.kernel/MAJOR[.MINOR]`, as it appears in a view's
    /// `requires`. Not a unit type - §12.2 types `requires` as a `SchemaId` set, and the
    /// RFC's own example puts the kernel schema in it alongside extensions.
    KernelSchema(String),
    /// An extension type or schema, `x.<domain>/<segment>`.
    Extension(String),
    /// A bare type name this build does not know — a kernel type a later version added.
    ///
    /// Only ever produced by *decoding*, never by parsing surface text. The two need
    /// opposite behaviour: on the wire an unrecognised type is forward compatibility and
    /// must survive, but in a hand-written file `@clai` is a typo and must stay an error.
    /// `SchemaId::parse` therefore still rejects these, and `parse_forward` accepts them.
    ///
    /// Before this arm existed, a unit whose type a build did not recognise failed the
    /// whole record with `SMY-E004: malformed envelope` — corruption, not degradation. So
    /// adding one kernel type in a later 0.x made every store carrying it unreadable to an
    /// earlier build, while an unknown *record* type and an unknown *extension* type both
    /// degraded correctly. This closes that asymmetry.
    UnknownKernel(String),
}

impl SchemaId {
    pub fn parse(s: &str) -> Result<SchemaId, IdError> {
        if let Some(k) = KernelType::parse(s) {
            return Ok(SchemaId::Kernel(k));
        }
        let err = || IdError {
            kind: "schema id",
            found: s.to_string(),
        };
        if let Some(v) = s.strip_prefix("smysl.kernel/") {
            return if is_ext_segment(v) {
                Ok(SchemaId::KernelSchema(s.to_string()))
            } else {
                Err(err())
            };
        }
        let rest = s.strip_prefix("x.").ok_or_else(err)?;
        let (domain, ty) = rest.split_once('/').ok_or_else(err)?;
        if is_ident(domain) && is_ext_segment(ty) {
            Ok(SchemaId::Extension(s.to_string()))
        } else {
            Err(err())
        }
    }

    /// Parse for *decoding*, where an unrecognised bare type is preserved rather than
    /// refused. Surface parsing keeps using [`SchemaId::parse`], so a typo stays a typo.
    pub fn parse_forward(s: &str) -> Result<SchemaId, IdError> {
        match SchemaId::parse(s) {
            Ok(id) => Ok(id),
            // Only a well-formed *bare* identifier degrades. A string that claims a
            // namespace - `x.` for an extension, `smysl.kernel/` for the kernel schema -
            // is being measured against that namespace's grammar, so a malformed one is
            // malformed rather than merely unfamiliar and still fails. Empty strings,
            // mixed case and punctuation cannot name a type at all.
            Err(e) => {
                let claims_a_namespace =
                    s.starts_with("x.") || s.starts_with("smysl.kernel/") || s.contains('/');
                if !claims_a_namespace && is_ext_segment(s) {
                    Ok(SchemaId::UnknownKernel(s.to_string()))
                } else {
                    Err(e)
                }
            }
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            SchemaId::Kernel(k) => k.as_str(),
            SchemaId::KernelSchema(s) | SchemaId::Extension(s) | SchemaId::UnknownKernel(s) => s,
        }
    }

    /// Whether this names a kernel *unit type*.
    pub const fn is_kernel(&self) -> bool {
        matches!(self, SchemaId::Kernel(_))
    }

    /// Whether this names the kernel schema itself.
    pub const fn is_kernel_schema(&self) -> bool {
        matches!(self, SchemaId::KernelSchema(_))
    }

    /// The kernel major this id requires, if it is a kernel schema id.
    pub fn kernel_major(&self) -> Option<u32> {
        match self {
            SchemaId::KernelSchema(s) => crate::kernel_major(s),
            _ => None,
        }
    }

    pub const fn kernel(&self) -> Option<KernelType> {
        match self {
            SchemaId::Kernel(k) => Some(*k),
            _ => None,
        }
    }
}

impl From<KernelType> for SchemaId {
    fn from(k: KernelType) -> SchemaId {
        SchemaId::Kernel(k)
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

impl FromStr for SchemaId {
    type Err = IdError;
    fn from_str(s: &str) -> Result<SchemaId, IdError> {
        SchemaId::parse(s)
    }
}

// ---------------------------------------------------------------------------
// Language tags
// ---------------------------------------------------------------------------

/// A view's language (§4). Per-unit language is an extension concern (D-7).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LangTag(String);

impl LangTag {
    pub fn new(s: impl Into<String>) -> Result<LangTag, IdError> {
        let s = s.into();
        let ok = !s.is_empty()
            && s.len() <= 35
            && s.split('-')
                .all(|seg| !seg.is_empty() && seg.bytes().all(|c| c.is_ascii_alphanumeric()));
        if ok {
            Ok(LangTag(s))
        } else {
            Err(IdError {
                kind: "language tag",
                found: s,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for LangTag {
    fn default() -> LangTag {
        LangTag("en".to_string())
    }
}

impl fmt::Display for LangTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

impl FromStr for LangTag {
    type Err = IdError;
    fn from_str(s: &str) -> Result<LangTag, IdError> {
        LangTag::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Uid ---------------------------------------------------------------

    #[test]
    fn short_form_is_prefixed_and_26_chars() {
        let u = Uid::from_bytes([0xAB; 32]);
        let s = u.short();
        assert!(s.starts_with("b3:"));
        assert_eq!(s.len(), 3 + 26);
        assert!(s[3..].bytes().all(|b| ALPHABET.contains(&b)));
    }

    #[test]
    fn canonical_form_is_52_chars() {
        let u = Uid::from_bytes([0x5C; 32]);
        let s = u.canonical();
        assert_eq!(s.len(), 3 + 52);
        assert!(
            s.starts_with(&u.short()),
            "short form is a prefix of canonical"
        );
    }

    #[test]
    fn all_zero_and_all_one_encode_at_the_alphabet_extremes() {
        assert_eq!(
            Uid::from_bytes([0x00; 32]).short(),
            format!("b3:{}", "a".repeat(26))
        );
        assert_eq!(
            Uid::from_bytes([0xFF; 32]).short(),
            format!("b3:{}", "7".repeat(26))
        );
    }

    #[test]
    fn distinct_bytes_give_distinct_short_forms() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[0] = 0x01;
        b[0] = 0x02;
        assert_ne!(Uid::from_bytes(a).short(), Uid::from_bytes(b).short());
    }

    #[test]
    fn short_form_covers_exactly_the_first_130_bits() {
        let base = [0u8; 32];
        let mut flipped = base;
        flipped[17] |= 0b0100_0000; // bit 137, past the 130-bit boundary
        assert_eq!(
            Uid::from_bytes(base).short(),
            Uid::from_bytes(flipped).short()
        );
        assert_ne!(
            Uid::from_bytes(base).canonical(),
            Uid::from_bytes(flipped).canonical()
        );

        let mut inside = base;
        inside[16] |= 0b0100_0000; // bit 129, the last bit inside the boundary
        assert_ne!(
            Uid::from_bytes(base).short(),
            Uid::from_bytes(inside).short()
        );
    }

    #[test]
    fn ordering_is_over_raw_bytes() {
        let a = Uid::from_bytes([0x00; 32]);
        let mut hi = [0u8; 32];
        hi[0] = 0x01;
        assert!(a < Uid::from_bytes(hi));
    }

    #[test]
    fn round_trips_through_bytes() {
        let b = [0x3A; 32];
        assert_eq!(Uid::from_bytes(b).to_bytes(), b);
        assert_eq!(Uid::from_bytes(b).as_bytes(), &b);
    }

    #[test]
    fn canonical_text_round_trips() {
        for seed in [0x00u8, 0x01, 0x7F, 0x80, 0xFE, 0xFF] {
            let mut b = [seed; 32];
            b[5] = seed.wrapping_mul(3);
            b[31] = seed.wrapping_add(7);
            let u = Uid::from_bytes(b);
            assert_eq!(Uid::parse(&u.canonical()).unwrap(), u);
        }
    }

    /// A display abbreviation in a record is `SMY-E071`, not a shorter uid.
    #[test]
    fn short_form_is_rejected_as_a_canonical_uid() {
        let u = Uid::from_bytes([0x42; 32]);
        let e = Uid::parse(&u.short()).unwrap_err();
        assert_eq!(e.code(), crate::diag::Code::E071);
    }

    #[test]
    fn malformed_uids_are_rejected() {
        assert!(Uid::parse("").is_err());
        assert!(Uid::parse(&"a".repeat(52)).is_err(), "missing b3: prefix");
        assert!(
            Uid::parse(&format!("b3:{}", "1".repeat(52))).is_err(),
            "1 is not in the alphabet"
        );
        assert!(Uid::parse(&format!("b3:{}", "a".repeat(53))).is_err());
    }

    // --- UidPrefix ---------------------------------------------------------

    #[test]
    fn a_short_form_prefix_matches_its_own_uid() {
        let u = Uid::from_bytes([0x9C; 32]);
        let p = UidPrefix::parse(&u.short()).unwrap();
        assert_eq!(p.bits(), 130);
        assert!(p.matches(&u));
    }

    #[test]
    fn a_prefix_matches_more_than_one_uid() {
        let a = Uid::from_bytes([0x00; 32]);
        let mut bb = [0u8; 32];
        bb[31] = 0xFF; // differs only past the 130-bit boundary
        let b = Uid::from_bytes(bb);
        let p = UidPrefix::parse(&a.short()).unwrap();
        assert!(
            p.matches(&a) && p.matches(&b),
            "ambiguity must be observable"
        );
    }

    #[test]
    fn a_full_width_prefix_is_exact() {
        let a = Uid::from_bytes([0x11; 32]);
        let mut bb = [0x11u8; 32];
        bb[31] = 0x12;
        let p = UidPrefix::parse(&a.canonical()).unwrap();
        assert!(p.matches(&a));
        assert!(!p.matches(&Uid::from_bytes(bb)));
    }

    #[test]
    fn prefixes_shorter_than_the_display_form_are_rejected() {
        assert!(UidPrefix::parse("b3:abc").is_err());
        assert!(UidPrefix::parse(&format!("b3:{}", "a".repeat(25))).is_err());
        assert!(UidPrefix::parse(&format!("b3:{}", "a".repeat(26))).is_ok());
    }

    // --- label-shaped ids --------------------------------------------------

    #[test]
    fn labels_accept_the_documented_shape() {
        for s in ["c/auth-p95", "e/trace-jul", "t/brief", "x1/y_2"] {
            assert_eq!(Label::new(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn labels_reject_everything_else() {
        for s in ["", "c", "c/", "/x", "C/x", "1/x", "c/x/y", "c x", "c/A"] {
            assert!(Label::new(s).is_err(), "`{s}` must be rejected");
        }
    }

    #[test]
    fn label_namespace_is_the_leading_segment() {
        assert_eq!(Label::new("c/auth-p95").unwrap().namespace(), "c");
        assert_eq!(ThreadId::new("t/brief").unwrap().namespace(), "t");
    }

    #[test]
    fn label_shaped_ids_are_distinct_types() {
        assert!(ThreadId::from_str("t/brief").is_ok());
        assert!(ViewId::from_str("v/incident").is_ok());
        assert!(ContentionId::from_str("k/pool-vs-index").is_ok());
        assert!(ThreadId::from_str("bad").is_err());
    }

    // --- agent ids ---------------------------------------------------------

    #[test]
    fn agent_ids_accept_the_documented_pattern() {
        for s in [
            "model:anthropic/claude-opus-5",
            "human:vladimir",
            "tool:grafana/board.12",
            "model:ollama/llama3.1:8b",
        ] {
            assert_eq!(AgentId::new(s).unwrap().as_str(), s);
        }
    }

    #[test]
    fn agent_ids_reject_unknown_kinds_and_bad_shapes() {
        for s in [
            "", "agent:x", "model", "model:", ":x", "model:X", "MODEL:x", "model:x/",
        ] {
            assert!(AgentId::new(s).is_err(), "`{s}` must be rejected");
        }
    }

    #[test]
    fn agent_kind_and_provider_are_readable() {
        let a = AgentId::new("model:anthropic/claude-opus-5").unwrap();
        assert_eq!(a.kind(), AgentKind::Model);
        assert_eq!(a.provider(), Some("anthropic"));

        let h = AgentId::new("human:vladimir").unwrap();
        assert_eq!(h.kind(), AgentKind::Human);
        assert_eq!(
            h.provider(),
            None,
            "corroboration cannot group a human by provider"
        );

        assert_eq!(AgentId::new("tool:jq").unwrap().kind(), AgentKind::Tool);
    }

    // --- schema ids --------------------------------------------------------

    #[test]
    fn fifteen_kernel_types_with_stable_names() {
        assert_eq!(KernelType::ALL.len(), 15);
        for &k in KernelType::ALL {
            assert_eq!(KernelType::parse(k.as_str()), Some(k));
        }
        assert_eq!(KernelType::ArtifactRef.as_str(), "artifact-ref");
        assert_eq!(KernelType::PackInfo.as_str(), "packinfo");
    }

    #[test]
    fn kernel_type_names_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for &k in KernelType::ALL {
            assert!(seen.insert(k.as_str()), "duplicate kernel type {k}");
        }
    }

    #[test]
    fn schema_ids_prefer_the_kernel_type() {
        let s = SchemaId::parse("claim").unwrap();
        assert!(s.is_kernel());
        assert_eq!(s.kernel(), Some(KernelType::Claim));
        assert_eq!(s.as_str(), "claim");
    }

    #[test]
    fn extension_schema_ids_take_the_x_form() {
        let s = SchemaId::parse("x.sre/incident").unwrap();
        assert!(!s.is_kernel());
        assert_eq!(s.kernel(), None);
        assert_eq!(s.as_str(), "x.sre/incident");
    }

    /// The RFC's own `requires: ["x.sre/1"]` needs a version in the trailing segment,
    /// which Appendix A's grammar does not allow. The examples win.
    #[test]
    fn extension_schema_ids_accept_a_version_after_the_slash() {
        for s in ["x.sre/1", "x.sre/1.2", "x.finance/v2"] {
            assert_eq!(SchemaId::parse(s).unwrap().as_str(), s);
        }
        assert!(
            SchemaId::parse("x.1sre/t").is_err(),
            "the domain still needs a letter"
        );
    }

    #[test]
    fn unqualified_unknown_types_are_rejected() {
        for s in ["nonsense", "x.sre", "x./t", "x.sre/", "sre/incident", ""] {
            assert!(SchemaId::parse(s).is_err(), "`{s}` must be rejected");
        }
    }

    /// A view's `requires` names the kernel schema alongside extensions, so a schema id
    /// has three shapes, not two.
    #[test]
    fn the_kernel_schema_is_a_schema_id_but_not_a_unit_type() {
        let s = SchemaId::parse("smysl.kernel/0.1").unwrap();
        assert!(s.is_kernel_schema());
        assert!(!s.is_kernel(), "the kernel schema is not a unit type");
        assert_eq!(s.kernel(), None);
        assert_eq!(s.kernel_major(), Some(0));
        assert_eq!(s.as_str(), "smysl.kernel/0.1");

        assert_eq!(
            SchemaId::parse("smysl.kernel/9").unwrap().kernel_major(),
            Some(9)
        );
        assert!(SchemaId::parse("smysl.kernel/").is_err());
        assert_eq!(SchemaId::parse("claim").unwrap().kernel_major(), None);
    }

    #[test]
    fn kernel_types_convert_into_schema_ids() {
        assert_eq!(SchemaId::from(KernelType::Finding).as_str(), "finding");
    }

    // --- language tags -----------------------------------------------------

    #[test]
    fn language_tags_accept_bcp47_shapes() {
        for s in ["en", "en-GB", "zh-Hans-CN", "ru"] {
            assert_eq!(LangTag::new(s).unwrap().as_str(), s);
        }
        assert_eq!(LangTag::default().as_str(), "en");
    }

    #[test]
    fn language_tags_reject_empty_segments() {
        for s in ["", "-", "en-", "-en", "en--GB", "en_GB"] {
            assert!(LangTag::new(s).is_err(), "`{s}` must be rejected");
        }
    }
}
