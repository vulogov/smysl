//! Library records: manifest (14), part text (15), part reading (18) and redaction (19).
//!
//! SMYSL-2.3 A-5 defines them; this module is only the types and their identities. What
//! *interprets* them — normalising text, reading files, deriving structure, resolving a
//! locator — is `smysl-text`. The split is deliberate and stated in SMYSL-2.4 §3.1: the
//! envelope decodes these records, `Store` holds them, a second implementation computes
//! their identities, and none of that needs a reader for a single file format.
//!
//! **A log never holds 15 or 18.** Part texts and readings live in a library's object store
//! as their record envelopes, so a bundle can emit them verbatim and verification is
//! decode-then-hash. The types are here because the envelope has to encode and decode them;
//! where the bytes are allowed to come to rest is `Store`'s rule to enforce (`SMY-E452`,
//! OQ-39, answered in 1.10.0). Rewriting a log to honour a redaction was the alternative,
//! and it is the one operation an append-only log cannot survive as evidence: it resets the
//! same hash chain that would have shown an edit.

use std::collections::BTreeMap;

use crate::ids::{is_alias, AgentId, LangTag, Mid, Rdid, Tid, Uid};
use crate::types::epistemics::SourceRef;
use crate::types::provenance::Hlc;
use crate::types::unit::Extra;

/// What a manifest permits to travel with it (manifest key 5).
///
/// Not a licence judgement — key 4 records the licence. This is what the producer *decided*,
/// and `bundle` refuses to emit text for anything but a permissive licence regardless
/// (`SMY-E402`, SMYSL-2.4 §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Carry {
    /// Neither the text nor a reference to it.
    #[default]
    None = 0,
    /// A reference only: the manifest names parts the receiver must already hold.
    Ref = 1,
    /// The part texts themselves.
    Text = 2,
}

impl Carry {
    pub const ALL: &'static [Carry] = &[Carry::None, Carry::Ref, Carry::Text];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Carry> {
        match v {
            0 => Some(Carry::None),
            1 => Some(Carry::Ref),
            2 => Some(Carry::Text),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Carry::None => "none",
            Carry::Ref => "ref",
            Carry::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Carry> {
        Carry::ALL.iter().copied().find(|c| c.as_str() == s)
    }
}

impl std::fmt::Display for Carry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// How a manifest derives from its parent (manifest key 12).
///
/// Required whenever key 11 is present: a derivation nobody named is a claim with no content,
/// and the four kinds are what a reader needs to know whether the texts are comparable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ParentKind {
    Translation,
    Edition,
    Excerpt,
    Transcription,
}

impl ParentKind {
    pub const ALL: &'static [ParentKind] = &[
        ParentKind::Translation,
        ParentKind::Edition,
        ParentKind::Excerpt,
        ParentKind::Transcription,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ParentKind::Translation => "translation",
            ParentKind::Edition => "edition",
            ParentKind::Excerpt => "excerpt",
            ParentKind::Transcription => "transcription",
        }
    }

    pub fn parse(s: &str) -> Option<ParentKind> {
        ParentKind::ALL.iter().copied().find(|k| k.as_str() == s)
    }
}

impl std::fmt::Display for ParentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// The calendar a manifest's recorded publication date is in (manifest key 18).
///
/// One key with one meaning: absent is Gregorian. A `Julian` value says the EDTF in key 8 is
/// a Julian date as the source printed it, which is not convertible without a decision about
/// the changeover — a decision that belongs to whoever reads the date, not to the record that
/// carries it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Calendar {
    Julian,
}

impl Calendar {
    pub const fn as_str(self) -> &'static str {
        match self {
            Calendar::Julian => "julian",
        }
    }

    pub fn parse(s: &str) -> Option<Calendar> {
        match s {
            "julian" => Some(Calendar::Julian),
            _ => None,
        }
    }
}

impl std::fmt::Display for Calendar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// One part of an expression, as a manifest names it (A-5, part entry).
///
/// Four identities for one part, and each answers a different question: `tid` is the bytes,
/// `structure` is how the reader divided them, `rdid` is the whole reading record, `length`
/// is a cheap check that does not need the bytes. A re-read whose structure hash or rdid
/// disagrees with the entry is `SMY-E401` — the text is intact but the reading changed, which
/// is what happens when a reader is upgraded under a corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PartEntry {
    pub tid: Tid,
    /// The part's length in bytes, normalised.
    pub length: u64,
    /// BLAKE3-256 of the canonical CBOR of the structure table in the reading's segments.
    ///
    /// Raw bytes rather than an identity type: it is a hash of a table, not of a record, so
    /// it names nothing that can be fetched and has no domain byte and no text prefix.
    pub structure: [u8; 32],
    pub rdid: Rdid,
    /// Written only when this part's language differs from the manifest's (key 2).
    pub lang: Option<LangTag>,
    pub extra: Extra,
}

impl PartEntry {
    pub fn new(tid: Tid, length: u64, structure: [u8; 32], rdid: Rdid) -> PartEntry {
        PartEntry {
            tid,
            length,
            structure,
            rdid,
            lang: None,
            extra: Extra::new(),
        }
    }

    pub fn with_lang(mut self, lang: LangTag) -> PartEntry {
        self.lang = Some(lang);
        self
    }
}

/// A manifest (record 14): an expression, at one version, is these parts in this order, read
/// this way, recorded with this metadata.
///
/// The field order below is A-5's key order, which is also the encoding order, so a reader
/// comparing this type against the table in the amendment reads them down the page together.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Manifest {
    /// The expression alias (key 0), lowercase ASCII per [`is_alias`].
    pub alias: String,
    /// Parts in reading order (key 1). MAY be empty: an import manifest records metadata for
    /// an expression whose text was not carried.
    pub parts: Vec<PartEntry>,
    /// BCP-47; `mul` for mixed, `und` for unknown (key 2).
    pub lang: LangTag,
    /// Reader id and version, such as `osis/1` (key 3).
    pub reader: String,
    /// An SPDX id, `public-domain` or `unknown` (key 4).
    pub licence: String,
    pub carry: Carry,
    pub title: Option<String>,
    pub creators: Vec<String>,
    /// EDTF, as recorded (key 8). Validated when `smysl-core` gains its EDTF parser in TX-P3;
    /// until then it is carried as the source printed it, which is also what "as recorded"
    /// means — a date this build cannot parse is still what the title page says.
    pub published: Option<String>,
    /// `isbn`, `doi`, `url`, `chat`, `channel`, `post`, `original`, `records` (key 9).
    pub identifiers: BTreeMap<String, String>,
    /// Where the expression came from (key 10), without `captured` when it records an import.
    pub origin: Option<SourceRef>,
    /// The manifest this one derives from, and how (keys 11 and 12). One field, because key
    /// 12 is required with key 11 and the pair is meaningless apart.
    pub parent: Option<(Mid, ParentKind)>,
    /// The previous version of *this* expression (key 13). Two heads under one alias are
    /// `SMY-W418`, not an error: a fork is a fact about a corpus, and refusing to open it
    /// would make the fact unreportable.
    pub supersedes: Option<Mid>,
    pub versification: Option<String>,
    /// Written only when true (key 15), so a lossless manifest encodes to no key at all.
    pub lossy: bool,
    /// The reader's manifest-level metadata, verbatim, as canonical CBOR (key 16).
    ///
    /// Opaque at this layer. It is whatever the reader found and could not place in a named
    /// key, which is exactly the content a core type must not interpret.
    pub raw: Option<Vec<u8>>,
    /// The boundary rule and size targets the parts were cut by (key 17).
    ///
    /// Required, and that is the point: the default changes at the end of TX-P2 when GE-T14
    /// measures it, and a corpus built before the change stays valid under the policy it
    /// recorded rather than under whatever the current default happens to be.
    pub part_policy: String,
    pub calendar: Option<Calendar>,
    pub extra: Extra,
}

impl Manifest {
    /// A manifest with the five required keys and no parts.
    ///
    /// `alias` is not validated here, because `Manifest` is also what the decoder builds and a
    /// decode must not fail on an alias a later version permits: [`Manifest::alias_is_valid`] is the
    /// check, called by the surface parser and by `check`'s library pass.
    pub fn new(
        alias: impl Into<String>,
        lang: LangTag,
        reader: impl Into<String>,
        licence: impl Into<String>,
        part_policy: impl Into<String>,
    ) -> Manifest {
        Manifest {
            alias: alias.into(),
            parts: Vec::new(),
            lang,
            reader: reader.into(),
            licence: licence.into(),
            carry: Carry::None,
            title: None,
            creators: Vec::new(),
            published: None,
            identifiers: BTreeMap::new(),
            origin: None,
            parent: None,
            supersedes: None,
            versification: None,
            lossy: false,
            raw: None,
            part_policy: part_policy.into(),
            calendar: None,
            extra: Extra::new(),
        }
    }

    pub fn with_parts(mut self, parts: Vec<PartEntry>) -> Manifest {
        self.parts = parts;
        self
    }

    pub fn with_carry(mut self, carry: Carry) -> Manifest {
        self.carry = carry;
        self
    }

    pub fn with_title(mut self, title: impl Into<String>) -> Manifest {
        self.title = Some(title.into());
        self
    }

    pub fn with_parent(mut self, parent: Mid, kind: ParentKind) -> Manifest {
        self.parent = Some((parent, kind));
        self
    }

    pub fn with_supersedes(mut self, previous: Mid) -> Manifest {
        self.supersedes = Some(previous);
        self
    }

    /// Whether `alias` satisfies A-3's grammar.
    pub fn alias_is_valid(&self) -> bool {
        is_alias(&self.alias)
    }

    /// `BLAKE3-256(0x0E ‖ canonical CBOR of the body)` (A-3).
    pub fn mid(&self) -> Mid {
        Mid::of(&crate::cbor::envelope::manifest_bytes(self))
    }

    /// The total length of the parts, as the entries record it.
    ///
    /// From the entries rather than from the texts, so it answers without the object store.
    pub fn length(&self) -> u64 {
        self.parts.iter().map(|p| p.length).sum()
    }
}

/// A part text (record 15): `{0: tid, 1: normalised bytes}`.
///
/// No surface form. Text is not written in surface syntax — a `.smy` file carrying a
/// megabyte of someone else's prose inside a quoted string would be neither readable nor
/// diffable, and the object store already addresses it by content.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PartText {
    pub tid: Tid,
    /// The normalised text: UTF-8, NFC, LF line endings, no BOM.
    pub text: Vec<u8>,
    pub extra: Extra,
}

impl PartText {
    /// Build a part text, deriving the tid from the bytes.
    ///
    /// The caller promises the bytes are normalised; `smysl-text`'s `norm` module is what
    /// makes that promise keepable.
    pub fn new(text: Vec<u8>) -> PartText {
        PartText {
            tid: Tid::from_normalised_bytes(&text),
            text,
            extra: Extra::new(),
        }
    }

    /// Build a part text with the tid it claims, whatever the bytes hash to.
    ///
    /// For the decoder, and the reason the decoder needs it: a record whose tid does not match
    /// its bytes must decode, be reported and re-encode unchanged. Failing the decode would
    /// make one bad record stop `Store::open` — F-12's lesson, paid for once already.
    pub fn with_claimed_tid(tid: Tid, text: Vec<u8>) -> PartText {
        PartText {
            tid,
            text,
            extra: Extra::new(),
        }
    }

    /// Whether the bytes hash to the tid this record claims (`SMY-E446` when false).
    pub fn verify(&self) -> bool {
        Tid::from_normalised_bytes(&self.text) == self.tid
    }
}

/// A part reading (record 18): what one reader derived from one part.
///
/// `segments` is the canonical CBOR of the segment table, carried opaquely. That is not
/// laziness: a segment row holds a locator, a level, a speaker and a timezone offset, all of
/// which are `smysl-text`'s to interpret, and the structure hash in the manifest's part entry
/// is taken over exactly these bytes. Decoding and re-encoding the table here would put a
/// second encoder in the path of a hash that a different crate computes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PartReading {
    pub tid: Tid,
    /// Reader id and version (key 1).
    pub reader: String,
    /// The canonical CBOR of the segment table (key 2). MAY be empty.
    pub segments: Vec<u8>,
    /// The reader's raw metadata for this part, verbatim (key 3).
    pub raw: Option<Vec<u8>>,
    pub extra: Extra,
}

impl PartReading {
    /// The CBOR item meaning "no segments": an empty array.
    ///
    /// Not zero bytes, which is not an item at all. A table may be empty (A-5) and a caller
    /// saying so with `Vec::new()` is making an ordinary statement, so the empty case is
    /// spelled here, once, and every other part of this type can assume `segments` holds a
    /// complete item.
    ///
    /// Spelling it at construction rather than only in the encoder is the difference between
    /// a safe type and a trap. The encoder guarded the empty case from the start, so the
    /// record always went onto the wire correctly — but [`structure_hash`](Self::structure_hash)
    /// hashed the field as given, which for `Vec::new()` is a hash of *nothing*, while the
    /// same reading decoded back from the wire hashes `[0x80]`. A manifest entry written
    /// before the round trip and checked after it would then fail `SMY-E401` on a corpus with
    /// nothing wrong with it: the text intact, the reading unchanged, and the structure hash
    /// moved by a representation nobody chose.
    pub const EMPTY_SEGMENTS: &'static [u8] = &[0x80];

    pub fn new(tid: Tid, reader: impl Into<String>, segments: Vec<u8>) -> PartReading {
        PartReading {
            tid,
            reader: reader.into(),
            segments: if segments.is_empty() {
                PartReading::EMPTY_SEGMENTS.to_vec()
            } else {
                segments
            },
            raw: None,
            extra: Extra::new(),
        }
    }

    /// Attach the reader's raw metadata for this part.
    ///
    /// An empty map rather than empty bytes, for the reason `segments` has: the field carries a
    /// CBOR item, and zero bytes is not one.
    pub fn with_raw(mut self, raw: Vec<u8>) -> PartReading {
        self.raw = Some(if raw.is_empty() { vec![0xA0] } else { raw });
        self
    }

    /// `BLAKE3-256(0x12 ‖ canonical CBOR of the body)` (A-3).
    pub fn rdid(&self) -> Rdid {
        Rdid::of(&crate::cbor::envelope::part_reading_bytes(self))
    }

    /// BLAKE3-256 of the segment table, which is a manifest part entry's `structure`.
    ///
    /// Over the table alone, not over the record: A-5 says the structure hash covers "the
    /// canonical CBOR of the structure table carried in the reading's segments (record 18 key
    /// 2)", so a reading that gains raw metadata keeps its structure hash and changes its
    /// rdid. Both are in the part entry, and this is why they are not the same number.
    pub fn structure_hash(&self) -> [u8; 32] {
        // Over the same bytes the encoder writes, which is why the empty case is spelled. A
        // directly assigned empty field is still possible — `segments` is public — so the
        // substitution is made here too rather than trusted to the constructor alone.
        crate::hash::hash_bytes(if self.segments.is_empty() {
            PartReading::EMPTY_SEGMENTS
        } else {
            &self.segments
        })
    }
}

/// What a resolver found under an identity.
///
/// Three states and not two, because the middle one is where `SMY-E446` lives. A resolver
/// that answered `Option<PartText>` would have to fold "nothing is stored here" together
/// with "something is stored here and it is not a part", and those are opposite answers: the
/// first is ordinary — a manifest may name parts whose text this library does not hold — and
/// the second is the corruption the code exists to report. Folded together, the only way to
/// report it would be to report absence too, and then every `carry: ref` manifest would be an
/// error.
/// Closed, where most public enumerations in this crate are `#[non_exhaustive]`. The three
/// states partition one question — what is at this address — into nothing, a part, and
/// something that is not a part, and there is no fourth answer to add later. A `_` arm here
/// would have to decide what to do about a state nobody has thought of, and the only
/// available decisions are to report a defect that may not be one or to stay quiet about one
/// that is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// Nothing is stored under this identity. Not a defect: a manifest is a catalog entry,
    /// and a catalog may name what it does not hold.
    Absent,
    /// The record stored there, **exactly as stored and not verified**.
    Part(PartText),
    /// Something is stored there and it is not a decodable part record.
    Unreadable,
}

/// Somewhere a part's bytes can be fetched from, so a pass over manifests can check them.
///
/// The one method hands back what is stored **without verifying it**, and that is the
/// contract rather than an oversight. An object store verifies on read — it has to, or a
/// caller asking for text would get whatever was on the disk — so a resolver built out of
/// `get_part` could never hand a checker a bad object, and `SMY-E446` would be unreportable
/// by the one pass whose job is to report it. Resolving and verifying are separate jobs here:
/// this fetches, `smysl-check`'s `Library` pass decides.
///
/// Implemented by `smysl-text`'s `ObjectStore`. It lives in the pure core because it names no
/// I/O — a `Tid` in, a record out — and because `smysl-check` may not depend on `smysl-text`
/// (SMYSL-2.4 §4.3.3 reserves that for the `Time` pass) while the facade cannot implement a
/// foreign trait for a foreign type.
/// `Debug` is a supertrait so that a caller can keep one in a `Debug` options struct —
/// `CheckOptions` is `Debug` and is printed in test failures, and a field that silently
/// dropped out of that would be the one field nobody could see while debugging a pass.
pub trait PartResolver: core::fmt::Debug {
    /// What is stored under `tid`.
    fn part(&self, tid: &Tid) -> Resolved;
}

/// A redaction: this part's text is to be held no longer (record 19, rule Z).
///
/// The record says *that* a part was redacted and by whom; it does not carry the part. What
/// honouring it means is deletion — of an object, from an object store — and the record is
/// what makes the deletion auditable: a store that no longer holds a part can still say why,
/// and a peer that never saw the part learns not to accept one.
///
/// # Why this is a record and not a flag
///
/// A redaction has to survive a merge, and it has to survive it in one direction only. Rule Z
/// is enforced over the **union** of every participant's redactions, so a peer that still holds
/// the bytes cannot reintroduce them by merging into a store that has let them go — the
/// redaction set grows and the filter is applied to the union, whatever the order records
/// arrive in. That is what makes the filtered merge commutative, associative and idempotent
/// (`P-Z1`–`P-Z4`), and none of it is available to a mutable flag on a part that is no longer
/// there.
///
/// # What it does not do
///
/// It does not rewrite a log. A log never holds a 15 or an 18 (`SMY-E452`, OQ-39), so there is
/// nothing in a log to erase and no reason to reset the hash chain that would have shown an
/// erasure. The bytes live in an object store, and erasing them there is an unlink.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Redaction {
    /// The part whose text is redacted.
    pub tid: Tid,
    pub agent: AgentId,
    pub ts: Hlc,
    /// A unit saying why, as a withdrawal's `reason` is.
    ///
    /// Optional, and a redaction with none is still a redaction: a legal demand arrives before
    /// anybody writes it down, and a store that refused the record until the paperwork existed
    /// would hold the text for as long as the paperwork took.
    pub reason: Option<Uid>,
    pub extra: Extra,
}

impl Redaction {
    pub fn new(tid: Tid, agent: AgentId, ts: Hlc) -> Redaction {
        Redaction {
            tid,
            agent,
            ts,
            reason: None,
            extra: Extra::new(),
        }
    }

    pub fn with_reason(mut self, reason: Uid) -> Redaction {
        self.reason = Some(reason);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(s: &str) -> LangTag {
        LangTag::new(s).unwrap()
    }

    /// Wire codes are permanent, so they are pinned rather than derived from the order of the
    /// variants. Renumbering `Carry` would silently turn every `text` manifest into a `ref`.
    #[test]
    fn carry_codes_are_pinned_to_the_amendment() {
        assert_eq!(Carry::None.as_u8(), 0);
        assert_eq!(Carry::Ref.as_u8(), 1);
        assert_eq!(Carry::Text.as_u8(), 2);
        for c in Carry::ALL {
            assert_eq!(Carry::from_u8(c.as_u8()), Some(*c));
            assert_eq!(Carry::parse(c.as_str()), Some(*c));
        }
        assert_eq!(Carry::from_u8(3), None, "the enumeration is closed");
        assert_eq!(Carry::default(), Carry::None);
    }

    #[test]
    fn the_named_sets_round_trip_through_their_words() {
        for k in ParentKind::ALL {
            assert_eq!(ParentKind::parse(k.as_str()), Some(*k));
        }
        assert_eq!(ParentKind::parse("adaptation"), None);
        assert_eq!(Calendar::parse("julian"), Some(Calendar::Julian));
        assert_eq!(
            Calendar::parse("gregorian"),
            None,
            "absent means Gregorian, so the word has no code"
        );
    }

    /// A part text knows whether its bytes are what it claims.
    #[test]
    fn a_part_text_verifies_its_own_tid() {
        let p = PartText::new(b"some normalised bytes\n".to_vec());
        assert!(p.verify());

        let liar = PartText::with_claimed_tid(Tid::of(b"other bytes"), p.text.clone());
        assert!(!liar.verify());
        assert_ne!(liar.tid, p.tid);
    }

    /// The length a manifest reports comes from its entries, not from the texts.
    ///
    /// Deliberately: a catalog answers "how big is this expression" without the object store,
    /// and an import manifest has no texts to add up at all.
    #[test]
    fn a_manifest_sums_the_lengths_its_entries_record() {
        let r = PartReading::new(Tid::of(b"a"), "txt/1", Vec::new());
        let e = |n: u64| PartEntry::new(Tid::of(&[n as u8]), n, r.structure_hash(), r.rdid());
        let m = Manifest::new("x", tag("en"), "txt/1", "unknown", "none")
            .with_parts(vec![e(10), e(32)]);
        assert_eq!(m.length(), 42);
        assert_eq!(
            Manifest::new("x", tag("en"), "txt/1", "unknown", "none").length(),
            0
        );
    }

    /// The alias check is the manifest's to offer and nobody's to skip.
    #[test]
    fn a_manifest_reports_whether_its_alias_is_well_formed() {
        let ok = Manifest::new("kjv/1769", tag("en"), "osis/1", "unknown", "none");
        assert!(ok.alias_is_valid());
        let mut bad = ok.clone();
        bad.alias = "KJV".into();
        assert!(!bad.alias_is_valid());
    }
}
