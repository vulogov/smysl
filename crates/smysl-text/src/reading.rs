//! The segment table: record 18 key 2, and the hash a manifest keeps over it.
//!
//! A part reading is what *one* reader derived from *one* part. `smysl-core` carries its
//! `segments` field as opaque canonical CBOR, deliberately — the structure hash in a
//! manifest's part entry is taken over exactly those bytes, and a second encoder in the path
//! of a hash computed by another crate is how two implementations come to disagree about an
//! identity they both compute correctly. This module is that one encoder.
//!
//! # One table, not two
//!
//! SMYSL-2.3 A-5 says the structure hash covers "the canonical CBOR of the structure table
//! carried in the reading's segments (record 18 key 2)". So the segment table *is* the
//! structure table: one row per node, in document order, each with its byte range, its level
//! and its locator. [`crate::structure`] derives the tree from the rows by containment rather
//! than storing a second copy of it, which is what makes the hash and the tree the same
//! statement. (SMYSL-2.4 §3.2's sketch gave a node a `meta: Option<u32>` pointing at a row in
//! a separate segment table; with one table the row index *is* the node index, so the field
//! is nothing. The RFC records that.)
//!
//! # Row keys (A-5)
//!
//! `{0: start, 1: end, 2: level, 3: locator, 4: lang?, 5: speaker?, 6: observed?, 7: ids?,
//! 8: tz offset in minutes?, 9: reported-speech marks?}`. Unknown keys are kept verbatim, as
//! everywhere else in this format (rule X): a reader from a later release may write an
//! eleventh key, and a corpus that round-trips through this build must come out as it went in
//! — including the hash, which is why the keys are kept rather than merely tolerated.
//!
//! # Key 9, and why a reader needs it (TX-P5 step 3b)
//!
//! §3.4 writes `text:mode` from structure, "reported-speech spans the reader marks" — and
//! until this key there was nowhere for a reader to mark one. The two markers real sources
//! carry are USFM's `\wj … \wj*` and OSIS's `<q who="…">`, both of which name a *span inside
//! a node* rather than a node, so neither the row's `speaker` nor a row of its own would say
//! what they say. Key 9 is therefore a list of [`Mark`]s on the row that contains them, in
//! part coordinates, sorted and non-overlapping.
//!
//! A mark may carry a `who`, because OSIS has one and USFM does not. Nothing reads it yet:
//! §3.4 is explicit that a mark establishes `reported` **and no holder**, since the mark says
//! that someone else is speaking and not that the edition's attribution is a fact. Carrying it
//! anyway is the same decision the chat readers' `ids` recorded — the source said it, dropping
//! it is irreversible, and the field is where a later phase will find it.
//!
//! It is **not** pseudonymised, and that is a decision rather than an omission. A `who` is a
//! label an editor put on a public text — `#jesus`, `#peter` — and not a platform's identifier
//! for a living person, which is what [`crate::speaker`] exists to hide; keying it would
//! destroy the only use it has while protecting nobody. A transcript whose speakers *are*
//! people belongs in a chat reader, where the `speaker` field is keyed.

use std::collections::BTreeMap;

use smysl_core::cbor::writer::{enc, MapBuilder};
use smysl_core::cbor::{major, Dec, Enc};
use smysl_core::error::{CodecError, LibError};
use smysl_core::ids::{LangTag, Rdid, Tid};
use smysl_core::types::library::{PartEntry, PartReading};
use smysl_core::types::Extra;

use crate::locator::{self, Locator};

/// Segment row keys (A-5, record 18 key 2).
pub mod keys {
    pub const START: u16 = 0;
    pub const END: u16 = 1;
    pub const LEVEL: u16 = 2;
    pub const LOCATOR: u16 = 3;
    pub const LANG: u16 = 4;
    pub const SPEAKER: u16 = 5;
    pub const OBSERVED: u16 = 6;
    pub const IDS: u16 = 7;
    pub const TZ: u16 = 8;
    /// Reported-speech marks: an array of `{0: start, 1: end, 2: who?}` (TX-P5 step 3b).
    pub const REPORTED: u16 = 9;

    /// The keys this build understands, for the contiguity test. The table lives beside the
    /// constants rather than being derived from them, for the reason `smysl-core`'s key tables
    /// do: a derived list cannot notice a gap.
    pub const ALL: &[u16] = &[
        START, END, LEVEL, LOCATOR, LANG, SPEAKER, OBSERVED, IDS, TZ, REPORTED,
    ];
}

/// What a reader calls the kind of a node: `book`, `chapter`, `verse`, `message`, `line`.
///
/// Open, and a plain string on the wire, because the vocabulary belongs to the reader. A
/// closed enumeration here would mean a new reader could not name what it found without a
/// format change, and the level is used for grouping and windowing, not for deciding meaning.
/// The grammar is narrow so that the values are comparable: lowercase ASCII, starting with a
/// letter.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Level(String);

/// The levels the TX-P1 readers emit, as a reference for anyone writing another one.
///
/// Not a constraint: [`Level::new`] accepts any value matching the grammar. The list is here
/// so that two readers of the same kind of text have a reason to pick the same word.
pub const LEVELS: &[&str] = &[
    "book",
    "chapter",
    "verse",
    "section",
    "paragraph",
    "line",
    "message",
    "day",
    "page",
    // TX-P2 step 2: a Slack export holds many conversations in one archive, so a chat reading
    // can have a level above the days. Telegram and WhatsApp export one conversation per file
    // and have none.
    "channel",
];

impl Level {
    pub fn new(s: &str) -> Option<Level> {
        let mut chars = s.chars();
        let first = chars.next()?;
        if !first.is_ascii_lowercase() {
            return None;
        }
        if !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
            return None;
        }
        Some(Level(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(&self.0)
    }
}

/// One row of the table: one node of the structure, with whatever the reader knew about it.
///
/// `start` and `end` are byte offsets into the part's normalised text, half-open. They are
/// the authoritative thing in a row — the locator beside them is informative (A-5), and
/// `SMY-W405` is what reports a disagreement.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Segment {
    pub start: u64,
    pub end: u64,
    pub level: Level,
    pub locator: Locator,
    /// Only when this segment's language differs from the part's.
    pub lang: Option<LangTag>,
    /// A speaker, as a pseudonym (`spk:` + 26 base32 characters) or a plain identifier.
    pub speaker: Option<String>,
    /// When the segment was observed: milliseconds since the Unix epoch, as recorded.
    pub observed: Option<u64>,
    /// The reader's own identifiers for this segment — a message id, a reply id, a thread id.
    /// Substrate for the edges TX-P2 builds; text to text, because that is what the sources
    /// give.
    pub ids: BTreeMap<String, String>,
    /// The timezone the segment's own timestamp was written in, in minutes east of UTC.
    pub tz_offset: Option<i32>,
    /// Spans of this segment that the source marks as **reported speech**, in part
    /// coordinates, sorted and non-overlapping.
    ///
    /// Empty is the honest default and means *nothing is marked* — not that nothing is
    /// reported. A 1769 Bible marks Psalm 14:1's reported clause with a comma and a capital
    /// letter, and no reader can see that.
    pub reported: Vec<Mark>,
    pub extra: Extra,
}

/// One span a source marks as reported speech, and whom it attributes it to.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Mark {
    /// Part coordinates, as the row's own range is.
    pub start: u64,
    pub end: u64,
    /// The source's own label for the speaker — OSIS's `who`, with its leading `#` kept as
    /// written — or `None`, which is every `\wj` in USFM.
    ///
    /// Not a [`crate::speaker`] pseudonym and not an agent: a string the edition chose. See
    /// the module docs for why it is carried and why it is not keyed.
    pub who: Option<String>,
    /// Mark keys this build does not know, kept verbatim.
    ///
    /// Rule X reaches inside a mark for the reason it reaches everywhere else: a producer from
    /// a later release may write a fourth key, and a corpus that round-trips through this
    /// build has to come out as it went in — including the structure hash, which is taken over
    /// these bytes. Refusing the row instead would make a forward-compatible addition a
    /// corpus this build cannot open.
    pub extra: Extra,
}

impl Mark {
    pub fn new(start: u64, end: u64) -> Mark {
        Mark {
            start,
            end,
            who: None,
            extra: Extra::new(),
        }
    }

    pub fn with_who(mut self, who: impl Into<String>) -> Mark {
        self.who = Some(who.into());
        self
    }

    /// Whether `span` lies wholly inside this mark.
    ///
    /// Containment and not intersection, for the reason
    /// `smysl_ingest::text::attribution` gives: a span that merely touches a reported clause
    /// is partly about the reporting.
    pub fn holds(&self, start: u64, end: u64) -> bool {
        self.start <= start && end <= self.end
    }
}

impl Segment {
    pub fn new(start: u64, end: u64, level: Level, locator: Locator) -> Segment {
        Segment {
            start,
            end,
            level,
            locator,
            lang: None,
            speaker: None,
            observed: None,
            ids: BTreeMap::new(),
            tz_offset: None,
            reported: Vec::new(),
            extra: Extra::new(),
        }
    }

    pub fn with_lang(mut self, lang: LangTag) -> Segment {
        self.lang = Some(lang);
        self
    }

    pub fn with_speaker(mut self, speaker: impl Into<String>) -> Segment {
        self.speaker = Some(speaker.into());
        self
    }

    pub fn with_observed(mut self, ms: u64, tz_offset: Option<i32>) -> Segment {
        self.observed = Some(ms);
        self.tz_offset = tz_offset;
        self
    }

    /// Add a reported-speech mark, keeping the list sorted.
    ///
    /// Sorted here rather than at encoding time: the wire order is the list order, and a
    /// caller that built the list in source order already has it right. A reader that marks
    /// two spans out of order is a reader whose spans do not describe its own text.
    pub fn with_mark(mut self, mark: Mark) -> Segment {
        self.reported.push(mark);
        self.reported.sort();
        self
    }

    pub fn with_id(mut self, key: impl Into<String>, value: impl Into<String>) -> Segment {
        self.ids.insert(key.into(), value.into());
        self
    }

    pub fn len(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn range(&self) -> std::ops::Range<u64> {
        self.start..self.end
    }
}

/// A signed integer, in the shortest form (CBOR major 0 for non-negative, 1 for negative).
fn enc_int(e: &mut Enc, v: i64) {
    if v < 0 {
        // CBOR encodes -1 - n, so -1 is argument 0. `-(v + 1)` is computed before the
        // negation rather than after, which is what keeps `i64::MIN` from overflowing.
        e.head(major::NEGINT, (-(v + 1)) as u64);
    } else {
        e.uint(v as u64);
    }
}

fn dec_int(d: &mut Dec<'_>) -> Result<i64, CodecError> {
    let at = d.position();
    let (m, arg) = d.head()?;
    let bad = || CodecError::MalformedEnvelope { at };
    match m {
        major::UINT => i64::try_from(arg).map_err(|_| bad()),
        major::NEGINT => i64::try_from(arg).map(|n| -1 - n).map_err(|_| bad()),
        _ => Err(bad()),
    }
}

/// A text-keyed map in **encoded**-key order.
///
/// Not string order. Deterministic CBOR orders map keys by their encoded bytes, and a text
/// key's encoding starts with its length, so `"zz"` sorts before `"aaa"`. `smysl-core`'s
/// envelope does the same thing for the manifest's `identifiers`; the rule is restated here
/// because this table is hashed and a second opinion about ordering would move the hash.
fn enc_text_map<'a>(e: &mut Enc, entries: impl Iterator<Item = (&'a str, &'a str)>) {
    let mut rows: Vec<(Vec<u8>, Vec<u8>)> = entries
        .map(|(k, v)| (enc(|e| e.text(k)), enc(|e| e.text(v))))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    e.head(major::MAP, rows.len() as u64);
    for (k, v) in rows {
        e.raw(&k);
        e.raw(&v);
    }
}

/// Encode a row's marks: an array of `{0: start, 1: end, 2: who?}`, in the list's order.
///
/// The order is the wire's, so it has to be the sorted one — [`Segment::with_mark`] keeps it
/// that way and [`dec_marks`] refuses anything else. An array whose order carried no meaning
/// would be two encodings of one table and therefore two structure hashes.
fn enc_marks(e: &mut Enc, marks: &[Mark]) {
    e.array(
        marks
            .iter()
            .map(|m| {
                let mut row = MapBuilder::new();
                row.put(mark_keys::START, |e| e.uint(m.start));
                row.put(mark_keys::END, |e| e.uint(m.end));
                if let Some(who) = &m.who {
                    row.put(mark_keys::WHO, |e| e.text(who));
                }
                row.put_extra(&m.extra);
                row.into_bytes()
            })
            .collect(),
    );
}

/// Mark keys, inside a row's key 9.
pub mod mark_keys {
    pub const START: u16 = 0;
    pub const END: u16 = 1;
    pub const WHO: u16 = 2;

    pub const ALL: &[u16] = &[START, END, WHO];
}

fn dec_marks(d: &mut Dec<'_>) -> Result<Vec<Mark>, CodecError> {
    let at = d.position();
    let bad = || CodecError::MalformedEnvelope { at };
    let marks = d.array(|d| {
        let n = d.map_head()?;
        let mut start = None;
        let mut end = None;
        let mut who = None;
        let mut extra = Extra::new();
        let mut prev = None;
        for _ in 0..n {
            let k = d.map_key(prev)?;
            prev = Some(k);
            d.reject_null()?;
            match k {
                mark_keys::START => start = Some(d.uint()?),
                mark_keys::END => end = Some(d.uint()?),
                mark_keys::WHO => who = Some(d.text()?.to_string()),
                // Kept verbatim, as a row's own unknown keys are (rule X): the structure
                // hash is taken over these bytes, so a mark key this build dropped would be a
                // corpus that changed identity by being read.
                _ => {
                    extra.insert(k, d.skip_item()?.to_vec());
                }
            }
        }
        let (start, end) = (start.ok_or_else(bad)?, end.ok_or_else(bad)?);
        if end < start {
            return Err(CodecError::MalformedEnvelope { at });
        }
        Ok(Mark {
            start,
            end,
            who,
            extra,
        })
    })?;
    // Sorted and non-overlapping, checked rather than assumed. Two marks that overlap are two
    // statements about one clause, and `Mark::holds` would answer for whichever came first.
    for w in marks.windows(2) {
        if w[0] >= w[1] || w[0].end > w[1].start {
            return Err(CodecError::NonDeterministic {
                at,
                reason: smysl_core::error::NonDetReason::UnsortedSet,
            });
        }
    }
    Ok(marks)
}

fn dec_text_map(d: &mut Dec<'_>) -> Result<BTreeMap<String, String>, CodecError> {
    let at = d.position();
    let n = d.map_head()?;
    let mut out = BTreeMap::new();
    let mut prev: Option<Vec<u8>> = None;
    for _ in 0..n {
        // The key is read as raw bytes and then decoded, so the order check compares what
        // the wire holds rather than what the strings look like.
        let kbytes = d.skip_item()?.to_vec();
        let key = Dec::new(&kbytes).text()?.to_string();
        if let Some(p) = &prev {
            if *p >= kbytes {
                return Err(CodecError::NonDeterministic {
                    at,
                    reason: smysl_core::error::NonDetReason::UnsortedMapKeys,
                });
            }
        }
        prev = Some(kbytes);
        out.insert(key, d.text()?.to_string());
    }
    Ok(out)
}

/// Encode one row.
fn enc_segment(s: &Segment) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::START, |e| e.uint(s.start));
    m.put(keys::END, |e| e.uint(s.end));
    m.put(keys::LEVEL, |e| e.text(s.level.as_str()));
    m.put(keys::LOCATOR, |e| e.text(&s.locator.to_string()));
    if let Some(l) = &s.lang {
        m.put(keys::LANG, |e| e.text(&l.to_string()));
    }
    if let Some(sp) = &s.speaker {
        m.put(keys::SPEAKER, |e| e.text(sp));
    }
    if let Some(o) = s.observed {
        m.put(keys::OBSERVED, |e| e.uint(o));
    }
    if !s.ids.is_empty() {
        m.put(keys::IDS, |e| {
            enc_text_map(e, s.ids.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        });
    }
    if let Some(tz) = s.tz_offset {
        m.put(keys::TZ, |e| enc_int(e, i64::from(tz)));
    }
    if !s.reported.is_empty() {
        m.put(keys::REPORTED, |e| enc_marks(e, &s.reported));
    }
    m.put_extra(&s.extra);
    m.into_bytes()
}

fn dec_segment(d: &mut Dec<'_>) -> Result<Segment, CodecError> {
    let at = d.position();
    let bad = || CodecError::MalformedEnvelope { at };
    let n = d.map_head()?;
    let mut start = None;
    let mut end = None;
    let mut level = None;
    let mut loc = None;
    let mut lang = None;
    let mut speaker = None;
    let mut observed = None;
    let mut ids = BTreeMap::new();
    let mut tz = None;
    let mut reported = Vec::new();
    let mut extra = Extra::new();
    let mut prev = None;
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        d.reject_null()?;
        match k {
            keys::START => start = Some(d.uint()?),
            keys::END => end = Some(d.uint()?),
            keys::LEVEL => level = Some(Level::new(d.text()?).ok_or_else(bad)?),
            keys::LOCATOR => loc = Some(locator::parse(d.text()?).map_err(|_| bad())?),
            keys::LANG => lang = Some(LangTag::new(d.text()?).map_err(|_| bad())?),
            keys::SPEAKER => speaker = Some(d.text()?.to_string()),
            keys::OBSERVED => observed = Some(d.uint()?),
            keys::IDS => ids = dec_text_map(d)?,
            keys::TZ => tz = Some(i32::try_from(dec_int(d)?).map_err(|_| bad())?),
            keys::REPORTED => reported = dec_marks(d)?,
            _ => {
                extra.insert(k, d.skip_item()?.to_vec());
            }
        }
    }
    let mut s = Segment::new(
        start.ok_or_else(bad)?,
        end.ok_or_else(bad)?,
        level.ok_or_else(bad)?,
        loc.ok_or_else(bad)?,
    );
    s.lang = lang;
    s.speaker = speaker;
    s.observed = observed;
    s.ids = ids;
    s.tz_offset = tz;
    s.reported = reported;
    s.extra = extra;
    Ok(s)
}

/// The canonical CBOR of a segment table: an array of rows, in document order.
///
/// Document order, not sorted order. The rows are a sequence — the order *is* the reading —
/// so this is one of the few arrays in the format that is not sorted by encoded bytes.
pub fn encode_table(rows: &[Segment]) -> Vec<u8> {
    let mut e = Enc::new();
    e.array_head(rows.len());
    for r in rows {
        e.raw(&enc_segment(r));
    }
    e.into_bytes()
}

/// Decode a segment table. An empty array is a legal table (A-5: `MAY be empty`).
pub fn decode_table(bytes: &[u8]) -> Result<Vec<Segment>, CodecError> {
    let mut d = Dec::new(bytes);
    let rows = d.array(dec_segment)?;
    Ok(rows)
}

/// A part reading with its table decoded.
///
/// [`PartReading`] is the record; this is the reading. The record keeps the table as bytes
/// because that is what is hashed, and [`Reading::to_record`] is the only place those bytes
/// are produced, so a reading and its record can never mean different things.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reading {
    pub tid: Tid,
    /// Reader id and version, such as `osis/1`.
    pub reader: String,
    pub rows: Vec<Segment>,
    /// The reader's raw metadata for this part, as canonical CBOR. Opaque here, as in the
    /// record: it is whatever the reader found and could not place in a named key.
    pub raw: Option<Vec<u8>>,
}

impl Reading {
    pub fn new(tid: Tid, reader: impl Into<String>, rows: Vec<Segment>) -> Reading {
        Reading {
            tid,
            reader: reader.into(),
            rows,
            raw: None,
        }
    }

    pub fn with_raw(mut self, raw: Vec<u8>) -> Reading {
        self.raw = Some(raw);
        self
    }

    pub fn to_record(&self) -> PartReading {
        let r = PartReading::new(self.tid, self.reader.clone(), encode_table(&self.rows));
        match &self.raw {
            Some(raw) => r.with_raw(raw.clone()),
            None => r,
        }
    }

    /// Decode a record's table back into a reading.
    pub fn from_record(r: &PartReading) -> Result<Reading, CodecError> {
        Ok(Reading {
            tid: r.tid,
            reader: r.reader.clone(),
            rows: decode_table(&r.segments)?,
            raw: r.raw.clone(),
        })
    }

    /// BLAKE3-256 of the table alone, which is a part entry's `structure`.
    pub fn structure_hash(&self) -> [u8; 32] {
        smysl_core::hash::hash_bytes(&encode_table(&self.rows))
    }

    /// `BLAKE3-256(0x12 ‖ canonical CBOR of the record body)`.
    pub fn rdid(&self) -> Rdid {
        self.to_record().rdid()
    }

    /// The part entry a manifest should hold for this reading, given the part's length.
    pub fn entry(&self, length: u64) -> PartEntry {
        PartEntry::new(self.tid, length, self.structure_hash(), self.rdid())
    }

    /// Check a reading against the entry a manifest recorded for it (`SMY-E401`).
    ///
    /// Two identities, checked in the order that makes the message useful: the structure hash
    /// first, because when a reader is upgraded under a corpus *both* move, and the structure
    /// is the one that says what changed. An rdid-only mismatch is narrower — same
    /// segmentation, different raw metadata — and saying so is the difference between "your
    /// reader changed" and "your reader's incidental output changed".
    pub fn verify_entry(&self, entry: &PartEntry) -> Result<(), LibError> {
        if self.structure_hash() != entry.structure {
            return Err(LibError::StructureChanged {
                tid: self.tid.canonical(),
                which: "structure",
            });
        }
        if self.rdid() != entry.rdid {
            return Err(LibError::StructureChanged {
                tid: self.tid.canonical(),
                which: "rdid",
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Key 9 round-trips, and the order it is written in is the order it is read back.
    #[test]
    fn a_rows_reported_speech_marks_survive_the_wire() {
        let row = Segment::new(
            0,
            60,
            Level::new("verse").expect("a level"),
            "Matt.4.4".parse().expect("a locator"),
        )
        .with_mark(Mark::new(30, 58).with_who("#Jesus"))
        .with_mark(Mark::new(10, 20));
        assert_eq!(
            row.reported.iter().map(|m| m.start).collect::<Vec<_>>(),
            vec![10, 30],
            "`with_mark` keeps the list sorted"
        );
        let bytes = encode_table(std::slice::from_ref(&row));
        let back = decode_table(&bytes).expect("a table");
        assert_eq!(back, vec![row]);
    }

    /// A mark list that is not sorted, or that overlaps, is not canonical.
    #[test]
    fn an_unsorted_or_overlapping_mark_list_is_refused() {
        let sorted = Segment::new(
            0,
            60,
            Level::new("verse").expect("a level"),
            "Matt.4.4".parse().expect("a locator"),
        )
        .with_mark(Mark::new(10, 20))
        .with_mark(Mark::new(30, 40));
        let good = encode_table(std::slice::from_ref(&sorted));
        assert!(decode_table(&good).is_ok());

        let mut backwards = sorted.clone();
        backwards.reported.reverse();
        let bad = encode_table(std::slice::from_ref(&backwards));
        assert!(
            matches!(decode_table(&bad), Err(CodecError::NonDeterministic { .. })),
            "the wire order is the list order, so an unsorted list is two encodings of one table"
        );

        let mut overlapping = sorted.clone();
        overlapping.reported[1].start = 15;
        let bad = encode_table(std::slice::from_ref(&overlapping));
        assert!(matches!(
            decode_table(&bad),
            Err(CodecError::NonDeterministic { .. })
        ));
    }

    /// A mark with no marks on it costs nothing on the wire.
    #[test]
    fn a_row_with_no_marks_writes_no_key_nine() {
        let plain = Segment::new(
            0,
            60,
            Level::new("verse").expect("a level"),
            "Matt.4.4".parse().expect("a locator"),
        );
        let bytes = encode_table(std::slice::from_ref(&plain));
        // The key is absent rather than an empty array, so every reading written before key 9
        // existed is byte-identical to the same reading written now.
        assert!(
            !bytes.contains(&0x09),
            "key 9 is not in the bytes: {bytes:02x?}"
        );
    }

    use super::*;
    use crate::norm::Normalised;

    fn level(s: &str) -> Level {
        Level::new(s).expect("a level")
    }

    fn loc(s: &str) -> Locator {
        locator::parse(s).expect("a locator")
    }

    fn a_tid() -> Tid {
        Normalised::of("In the beginning God created\n").tid()
    }

    fn rows() -> Vec<Segment> {
        vec![
            Segment::new(0, 16, level("verse"), loc("Gen.1.1")),
            Segment::new(16, 29, level("verse"), loc("Gen.1.2"))
                .with_lang(LangTag::new("ru").unwrap())
                .with_speaker("spk:abcdefghijklmnopqrstuvwxyz")
                .with_observed(1_700_000_000_000, Some(-300))
                .with_id("msg", "17")
                .with_id("reply", "16"),
        ]
    }

    #[test]
    fn the_row_key_table_is_contiguous_from_zero() {
        for (i, k) in keys::ALL.iter().enumerate() {
            assert_eq!(*k as usize, i, "row keys are 0..{}", keys::ALL.len());
        }
    }

    #[test]
    fn a_table_round_trips_byte_identically() {
        let bytes = encode_table(&rows());
        let back = decode_table(&bytes).unwrap();
        assert_eq!(back, rows());
        assert_eq!(encode_table(&back), bytes);
    }

    #[test]
    fn an_empty_table_is_an_empty_array() {
        let bytes = encode_table(&[]);
        assert_eq!(bytes, vec![0x80]);
        assert_eq!(decode_table(&bytes).unwrap(), Vec::<Segment>::new());
    }

    #[test]
    fn a_negative_timezone_offset_survives() {
        for tz in [-720i32, -300, -1, 0, 1, 330, 840] {
            let mut r = Segment::new(0, 1, level("message"), loc("L1"));
            r.tz_offset = Some(tz);
            let back = decode_table(&encode_table(&[r.clone()])).unwrap();
            assert_eq!(back[0].tz_offset, Some(tz), "tz {tz}");
            assert_eq!(back[0], r);
        }
    }

    #[test]
    fn unknown_row_keys_are_kept_verbatim() {
        let mut r = Segment::new(0, 4, level("line"), loc("L1"));
        r.extra.insert(31, vec![0x18, 0x2A]);
        let bytes = encode_table(std::slice::from_ref(&r));
        let back = decode_table(&bytes).unwrap();
        assert_eq!(back[0].extra.get(&31), Some(&vec![0x18, 0x2A]));
        assert_eq!(encode_table(&back), bytes, "and re-encode in place");
    }

    #[test]
    fn an_ids_map_is_ordered_by_encoded_key_bytes() {
        // "zz" before "aaa": the length prefix sorts first. Written out rather than derived,
        // because the whole point is that it is not the order a `BTreeMap` iterates in.
        let r = Segment::new(0, 1, level("message"), loc("L1"))
            .with_id("aaa", "1")
            .with_id("zz", "2");
        let bytes = encode_table(std::slice::from_ref(&r));
        // `0x62` is a two-byte text head, `0x63` a three-byte one.
        let short = bytes
            .windows(3)
            .position(|w| w == [0x62, b'z', b'z'])
            .expect("`zz` is in the encoding");
        let long = bytes
            .windows(4)
            .position(|w| w == [0x63, b'a', b'a', b'a'])
            .expect("`aaa` is in the encoding");
        assert!(short < long, "zz encodes before aaa");
        assert_eq!(decode_table(&bytes).unwrap(), vec![r]);
    }

    #[test]
    fn an_ids_map_out_of_encoded_key_order_is_refused() {
        // Built by hand: "aaa" then "zz" is string order and wire-illegal.
        let mut e = Enc::new();
        e.array_head(1);
        let mut m = MapBuilder::new();
        m.put(keys::START, |e| e.uint(0));
        m.put(keys::END, |e| e.uint(1));
        m.put(keys::LEVEL, |e| e.text("line"));
        m.put(keys::LOCATOR, |e| e.text("L1"));
        m.put(keys::IDS, |e| {
            e.head(major::MAP, 2);
            e.text("aaa");
            e.text("1");
            e.text("zz");
            e.text("2");
        });
        e.raw(&m.into_bytes());
        let err = decode_table(&e.into_bytes()).unwrap_err();
        assert!(
            matches!(err, CodecError::NonDeterministic { .. }),
            "{err:?}"
        );
    }

    #[test]
    fn a_row_missing_a_required_key_is_refused() {
        let mut e = Enc::new();
        e.array_head(1);
        let mut m = MapBuilder::new();
        m.put(keys::START, |e| e.uint(0));
        m.put(keys::END, |e| e.uint(1));
        m.put(keys::LEVEL, |e| e.text("line"));
        e.raw(&m.into_bytes());
        assert!(decode_table(&e.into_bytes()).is_err(), "no locator");
    }

    #[test]
    fn a_level_is_lowercase_and_starts_with_a_letter() {
        assert!(Level::new("chapter").is_some());
        assert!(Level::new("day-1").is_some());
        assert!(Level::new("h2").is_some());
        assert!(Level::new("Chapter").is_none());
        assert!(Level::new("2h").is_none());
        assert!(Level::new("").is_none());
        assert!(Level::new("some level").is_none());
        for l in LEVELS {
            assert!(Level::new(l).is_some(), "{l}");
        }
    }

    #[test]
    fn a_reading_and_its_record_say_the_same_thing() {
        let r = Reading::new(a_tid(), "osis/1", rows());
        let record = r.to_record();
        assert_eq!(record.segments, encode_table(&rows()));
        assert_eq!(Reading::from_record(&record).unwrap(), r);
        assert_eq!(record.structure_hash(), r.structure_hash());
        assert_eq!(record.rdid(), r.rdid());
    }

    /// The structure hash is over the table and the rdid over the record, so raw metadata
    /// moves one and not the other. Both are in the part entry, which is why they are two
    /// numbers and not one.
    #[test]
    fn raw_metadata_moves_the_rdid_and_not_the_structure_hash() {
        let plain = Reading::new(a_tid(), "osis/1", rows());
        let with_raw = plain.clone().with_raw(vec![0xA1, 0x61, b'k', 0x01]);
        assert_eq!(plain.structure_hash(), with_raw.structure_hash());
        assert_ne!(plain.rdid(), with_raw.rdid());
    }

    #[test]
    fn an_entry_from_a_reading_verifies_against_it() {
        let r = Reading::new(a_tid(), "osis/1", rows());
        let entry = r.entry(29);
        r.verify_entry(&entry).unwrap();
        assert_eq!(entry.length, 29);
        assert_eq!(entry.tid, r.tid);
    }

    #[test]
    fn a_reading_that_segments_differently_is_e401_naming_which_identity_moved() {
        let r = Reading::new(a_tid(), "osis/1", rows());
        let entry = r.entry(29);

        let mut resegmented = r.clone();
        resegmented.rows.pop();
        let e = resegmented.verify_entry(&entry).unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E401));
        assert!(e.to_string().contains("structure"), "{e}");

        // Same segmentation, different raw metadata: the narrower message.
        let same_rows = r.clone().with_raw(vec![0xA0]);
        let e = same_rows.verify_entry(&entry).unwrap_err();
        assert!(e.to_string().contains("rdid"), "{e}");
    }
}
