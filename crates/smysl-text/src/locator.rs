//! Locators: the name a reader gives a place in a part, written one way.
//!
//! A locator is the informative half of a reference. SMYSL-2.3 A-5 makes the span
//! authoritative — `t3:<52 chars>[#<locator>]` with a byte span beside it, and `SMY-W405`
//! when the two disagree, span wins. So a locator is not how a passage is *found*; it is how
//! a passage is *said*, by a person, in a citation, across editions that do not agree on
//! bytes.
//!
//! # Where this grammar comes from
//!
//! RFC SMYSL-2.4 §3.2 points at "draft 3 Appendix B". Draft 3 is the design record and is not
//! in this repository — the RFC set supersedes it as the working document and carries no
//! locator grammar of its own. So this module *is* the grammar, written against what the six
//! TX-P1 readers have to be able to say, and the RFC records that (§3.2). Three forms and a
//! range, because each reader family needs exactly one of them:
//!
//! | form | example | reader |
//! |---|---|---|
//! | canonical | `Gen.1.1`, `1John.3`, `Ps.136` | `usfm`, `osis`, `zefania` |
//! | line | `L412` | `txt` |
//! | JSON Pointer (RFC 6901) | `/messages/3/text` | `json` |
//! | range | `Gen.1.1-Gen.1.3`, `L10-L14` | canonical or line ends, never pointers |
//!
//! The canonical form is deliberately OSIS-shaped (`Book.Chapter.Verse`) rather than invented:
//! the five Bibles of GE-T1 are distributed with those identifiers, every alignment table in
//! the spike is keyed by them (`Ex.20.1`), and a locator that had to be translated out of the
//! source's own vocabulary would be a locator nobody could check by eye.
//!
//! # One way to write each place
//!
//! `Locator::to_string` is canonical and [`parse`] round-trips it, which is what makes a
//! locator usable as a key: `source.reference` is a string, and two spellings of one place
//! would silently be two places. The normalisations are therefore refusals, not repairs —
//! `Gen.01.1` does not parse, rather than parsing as `Gen.1.1`, because a parser that accepted
//! both would put both into a corpus and only the writer would know which was meant.
//!
//! The one normalisation that *is* applied is a range whose ends are equal, which collapses to
//! its end: `L7-L7` is `L7`, by construction, so no corpus can hold both spellings.

use std::cmp::Ordering;
use std::fmt;

/// A place in a part, or a range of places.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Locator {
    /// `Gen.1.1`: an identifier head, then numbered steps. The head keeps its case, because
    /// OSIS identifiers are case-sensitive and `Ps` is not `ps`.
    Canonical { head: String, steps: Vec<u64> },
    /// `L412`: the 412th line of the part, counting from 1.
    Line(u64),
    /// `/messages/3/text`: an RFC 6901 JSON Pointer, held decoded. The reference tokens are
    /// *unescaped* here, so a segment may contain `/` and `~`; `Locator::to_string` escapes
    /// them back to `~1` and `~0`.
    Pointer(Vec<String>),
    /// `a-b`: the hull of two ends of the same kind, ordered, and never equal — equal ends are
    /// the single locator, enforced by [`Locator::range`].
    Range(Box<Locator>, Box<Locator>),
}

/// Which form a locator is in, for messages and for the same-kind rule on ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Kind {
    Canonical,
    Line,
    Pointer,
    Range,
}

impl Kind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Kind::Canonical => "canonical",
            Kind::Line => "line",
            Kind::Pointer => "pointer",
            Kind::Range => "range",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// Why a string is not a locator, and where.
///
/// The offset is a byte offset into the string given to [`parse`]. It is in the error because
/// the strings that fail are usually generated — a reader's own output, or a `ref` in a
/// hand-written file — and "malformed locator" without a position is a message that sends
/// somebody to read a hundred characters by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatorError {
    pub at: usize,
    pub reason: Reason,
}

/// What was wrong. One variant per refusal, so a test can assert the refusal rather than the
/// wording of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reason {
    /// Empty, or an empty component of something non-empty.
    Empty,
    /// A character locators do not admit: control, whitespace, or `#` (which would end the
    /// locator inside a reference).
    BadCharacter,
    /// A number with a leading zero, which has a canonical spelling that is not this one.
    LeadingZero,
    /// A number too large for `u64`, or a line number of zero.
    BadNumber,
    /// A head that is not an identifier: empty, or with no letter in it.
    BadHead,
    /// A JSON Pointer escape other than `~0` or `~1`.
    BadEscape,
    /// A range whose ends are different kinds.
    MixedRange,
    /// A range whose ends are out of order.
    BackwardRange,
    /// A range of JSON Pointers, which has no meaning this crate is willing to invent.
    PointerRange,
    /// A range of ranges.
    NestedRange,
}

impl Reason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Reason::Empty => "empty",
            Reason::BadCharacter => "character not allowed in a locator",
            Reason::LeadingZero => "number with a leading zero",
            Reason::BadNumber => "number out of range",
            Reason::BadHead => "not an identifier",
            Reason::BadEscape => "escape other than ~0 or ~1",
            Reason::MixedRange => "range over two different kinds",
            Reason::BackwardRange => "range ends out of order",
            Reason::PointerRange => "range of JSON Pointers",
            Reason::NestedRange => "range of ranges",
        }
    }
}

impl fmt::Display for LocatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at byte {}: {}", self.at, self.reason.as_str())
    }
}

impl std::error::Error for LocatorError {}

fn err(at: usize, reason: Reason) -> LocatorError {
    LocatorError { at, reason }
}

impl Locator {
    /// A line locator. Lines count from 1, so zero is refused: a reader that emitted `L0`
    /// would be off by one everywhere and the locator is the only place it would show.
    pub fn line(n: u64) -> Result<Locator, LocatorError> {
        if n == 0 {
            return Err(err(1, Reason::BadNumber));
        }
        Ok(Locator::Line(n))
    }

    /// A canonical locator, head and steps.
    pub fn canonical(head: &str, steps: &[u64]) -> Result<Locator, LocatorError> {
        check_head(head, 0)?;
        Ok(Locator::Canonical {
            head: head.to_string(),
            steps: steps.to_vec(),
        })
    }

    /// A JSON Pointer from already-unescaped reference tokens.
    pub fn pointer<S: AsRef<str>>(tokens: &[S]) -> Result<Locator, LocatorError> {
        if tokens.is_empty() {
            return Err(err(0, Reason::Empty));
        }
        for t in tokens {
            if let Some(bad) = t.as_ref().char_indices().find(|(_, c)| !ok_char(*c)) {
                return Err(err(bad.0, Reason::BadCharacter));
            }
        }
        Ok(Locator::Pointer(
            tokens.iter().map(|t| t.as_ref().to_string()).collect(),
        ))
    }

    /// The hull of two ends, or the single locator when they are equal.
    ///
    /// Equal ends collapse rather than being refused: a reader walking a structure naturally
    /// produces a range for a node whose first and last child are the same node, and refusing
    /// it would push the special case into every reader. Collapsing puts it here, once.
    pub fn range(from: Locator, to: Locator) -> Result<Locator, LocatorError> {
        if from.kind() == Kind::Range || to.kind() == Kind::Range {
            return Err(err(0, Reason::NestedRange));
        }
        if from.kind() != to.kind() {
            return Err(err(0, Reason::MixedRange));
        }
        if from.kind() == Kind::Pointer {
            return Err(err(0, Reason::PointerRange));
        }
        match from.cmp(&to) {
            Ordering::Equal => Ok(from),
            Ordering::Less => Ok(Locator::Range(Box::new(from), Box::new(to))),
            Ordering::Greater => Err(err(0, Reason::BackwardRange)),
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Locator::Canonical { .. } => Kind::Canonical,
            Locator::Line(_) => Kind::Line,
            Locator::Pointer(_) => Kind::Pointer,
            Locator::Range(..) => Kind::Range,
        }
    }

    /// The two ends of a range, or this locator twice.
    ///
    /// What resolution needs: a range resolves to the hull of its ends, and a single locator
    /// is its own hull, so the caller has no special case.
    pub fn ends(&self) -> (&Locator, &Locator) {
        match self {
            Locator::Range(a, b) => (a, b),
            single => (single, single),
        }
    }

    /// Whether this locator is a prefix of, or equal to, `other` — `Gen.1` contains `Gen.1.1`.
    ///
    /// Containment, not ordering: it is how a structure tree finds the node a citation names
    /// when the citation is coarser than the segmentation.
    pub fn contains(&self, other: &Locator) -> bool {
        match (self, other) {
            (
                Locator::Canonical {
                    head: h1,
                    steps: s1,
                },
                Locator::Canonical {
                    head: h2,
                    steps: s2,
                },
            ) => h1 == h2 && s2.starts_with(s1),
            (Locator::Pointer(a), Locator::Pointer(b)) => b.starts_with(a),
            (Locator::Range(a, b), other) => {
                let (lo, hi) = other.ends();
                a.kind() == lo.kind() && a.as_ref() <= lo && hi <= b.as_ref()
            }
            (a, b) => a == b,
        }
    }
}

/// Characters a locator may hold at all.
///
/// `#` is excluded because a reference is `t3:…#<locator>` and a locator holding a `#` could
/// not be read back out of one. Whitespace and controls are excluded because they survive a
/// round trip through a CBOR text string and then make two visually identical locators
/// different keys.
fn ok_char(c: char) -> bool {
    c != '#' && !c.is_whitespace() && !c.is_control()
}

fn check_head(head: &str, at: usize) -> Result<(), LocatorError> {
    if head.is_empty() {
        return Err(err(at, Reason::Empty));
    }
    if !head.chars().all(|c| c.is_ascii_alphanumeric())
        || !head.chars().any(|c| c.is_ascii_alphabetic())
    {
        return Err(err(at, Reason::BadHead));
    }
    // `L12` is a line locator, so it cannot also be a bare canonical head. Stated rather than
    // left to parse order, because the collision is silent: a reader emitting a book
    // abbreviated `L` would produce locators that resolve to lines.
    if is_line_shaped(head) {
        return Err(err(at, Reason::BadHead));
    }
    Ok(())
}

fn is_line_shaped(s: &str) -> bool {
    match s.strip_prefix('L') {
        Some(rest) => !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

/// A number in canonical spelling: digits, no leading zero, fits a `u64`.
fn number(s: &str, at: usize) -> Result<u64, LocatorError> {
    if s.is_empty() {
        return Err(err(at, Reason::Empty));
    }
    if !s.chars().all(|c| c.is_ascii_digit()) {
        return Err(err(at, Reason::BadNumber));
    }
    if s.len() > 1 && s.starts_with('0') {
        return Err(err(at, Reason::LeadingZero));
    }
    s.parse::<u64>().map_err(|_| err(at, Reason::BadNumber))
}

/// Parse a locator. Hand-written, because the grammar is four productions and a regex
/// dependency in the pure core would be a dependency for four productions.
pub fn parse(s: &str) -> Result<Locator, LocatorError> {
    if s.is_empty() {
        return Err(err(0, Reason::Empty));
    }
    if let Some(bad) = s.char_indices().find(|(_, c)| !ok_char(*c)) {
        return Err(err(bad.0, Reason::BadCharacter));
    }
    // A pointer is the whole string or none of it: `/` starts one, and its reference tokens
    // may contain `-`, so splitting a range out of it first would cut a pointer in half.
    if s.starts_with('/') {
        return parse_pointer(s);
    }
    match s.find('-') {
        Some(i) => {
            let from = parse_single(&s[..i], 0)?;
            let to = parse_single(&s[i + 1..], i + 1)?;
            Locator::range(from, to).map_err(|e| err(if e.at == 0 { i } else { e.at }, e.reason))
        }
        None => parse_single(s, 0),
    }
}

fn parse_single(s: &str, at: usize) -> Result<Locator, LocatorError> {
    if s.is_empty() {
        return Err(err(at, Reason::Empty));
    }
    if s.starts_with('/') {
        return Err(err(at, Reason::PointerRange));
    }
    if let Some(rest) = s.strip_prefix('L') {
        if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
            let n = number(rest, at + 1)?;
            return Locator::line(n).map_err(|e| err(at + 1, e.reason));
        }
    }
    let mut parts = s.split('.');
    let head = parts.next().unwrap_or_default();
    check_head(head, at)?;
    let mut steps = Vec::new();
    let mut offset = at + head.len() + 1;
    for p in parts {
        steps.push(number(p, offset)?);
        offset += p.len() + 1;
    }
    Ok(Locator::Canonical {
        head: head.to_string(),
        steps,
    })
}

fn parse_pointer(s: &str) -> Result<Locator, LocatorError> {
    let mut tokens = Vec::new();
    // `split('/')` on a string starting with `/` gives a leading empty piece, which is the
    // part of RFC 6901 that says a pointer is a sequence of `/`-prefixed tokens rather than
    // `/`-separated ones. The empty token is legal in JSON Pointer (an object key of ""), so
    // it is kept rather than refused.
    let mut offset = 1;
    for raw in s[1..].split('/') {
        tokens.push(unescape(raw, offset)?);
        offset += raw.len() + 1;
    }
    Ok(Locator::Pointer(tokens))
}

fn unescape(raw: &str, at: usize) -> Result<String, LocatorError> {
    if !raw.contains('~') {
        return Ok(raw.to_string());
    }
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.char_indices();
    while let Some((i, c)) = chars.next() {
        if c != '~' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some((_, '0')) => out.push('~'),
            Some((_, '1')) => out.push('/'),
            _ => return Err(err(at + i, Reason::BadEscape)),
        }
    }
    Ok(out)
}

fn escape(token: &str) -> String {
    if !token.contains('~') && !token.contains('/') {
        return token.to_string();
    }
    // `~` first: escaping `/` first would turn it into `~1` and the `~` pass would then make
    // it `~01`, which unescapes to `~1`. The order is the whole content of RFC 6901 §3.
    token.replace('~', "~0").replace('/', "~1")
}

impl fmt::Display for Locator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Locator::Canonical { head, steps } => {
                f.write_str(head)?;
                for s in steps {
                    write!(f, ".{s}")?;
                }
                Ok(())
            }
            Locator::Line(n) => write!(f, "L{n}"),
            Locator::Pointer(tokens) => {
                for t in tokens {
                    write!(f, "/{}", escape(t))?;
                }
                Ok(())
            }
            Locator::Range(a, b) => write!(f, "{a}-{b}"),
        }
    }
}

impl std::str::FromStr for Locator {
    type Err = LocatorError;

    fn from_str(s: &str) -> Result<Locator, LocatorError> {
        parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_form_round_trips_through_its_canonical_spelling() {
        for s in [
            "Gen",
            "Gen.1",
            "Gen.1.1",
            "1John.3.16",
            "Ps.136.1",
            "sec.2.3.1",
            "L1",
            "L412",
            "/messages",
            "/messages/3/text",
            "/",
            "/a~0b",
            "/a~1b",
            "Gen.1.1-Gen.1.3",
            "Gen.1-Gen.3",
            "L10-L14",
        ] {
            let l = parse(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(l.to_string(), s, "canonical spelling of {s}");
            assert_eq!(parse(&l.to_string()), Ok(l), "round trip of {s}");
        }
    }

    #[test]
    fn a_pointer_keeps_its_tokens_decoded_and_escapes_them_back() {
        let l = parse("/a~1b/c~0d").unwrap();
        assert_eq!(
            l,
            Locator::Pointer(vec!["a/b".to_string(), "c~d".to_string()])
        );
        assert_eq!(l.to_string(), "/a~1b/c~0d");
        // The order of the two substitutions is the thing that bites: a token holding `~1`
        // literally must not come back as `/`.
        let tricky = Locator::pointer(&["~1"]).unwrap();
        assert_eq!(tricky.to_string(), "/~01");
        assert_eq!(parse("/~01").unwrap(), tricky);
    }

    #[test]
    fn an_empty_pointer_token_is_legal_because_an_empty_json_key_is() {
        assert_eq!(parse("/").unwrap(), Locator::Pointer(vec![String::new()]));
        assert_eq!(
            parse("/a//b").unwrap(),
            Locator::Pointer(vec!["a".into(), String::new(), "b".into()])
        );
    }

    #[test]
    fn a_number_has_one_spelling() {
        assert_eq!(parse("Gen.01").unwrap_err().reason, Reason::LeadingZero);
        assert_eq!(parse("L01").unwrap_err().reason, Reason::LeadingZero);
        assert_eq!(parse("Gen.1.").unwrap_err().reason, Reason::Empty);
        assert_eq!(parse("Gen..1").unwrap_err().reason, Reason::Empty);
        assert_eq!(parse("Gen.1a").unwrap_err().reason, Reason::BadNumber);
        assert_eq!(parse("L0").unwrap_err().reason, Reason::BadNumber);
        assert_eq!(
            parse("Gen.99999999999999999999").unwrap_err().reason,
            Reason::BadNumber
        );
    }

    #[test]
    fn the_refusals_say_where() {
        assert_eq!(parse("Gen.1.0x").unwrap_err().at, 6);
        assert_eq!(parse("Gen 1").unwrap_err().reason, Reason::BadCharacter);
        assert_eq!(parse("Gen#1").unwrap_err().reason, Reason::BadCharacter);
        assert_eq!(parse("Gen#1").unwrap_err().at, 3);
        assert_eq!(parse("").unwrap_err().reason, Reason::Empty);
    }

    #[test]
    fn a_head_is_an_identifier_and_never_line_shaped() {
        assert!(
            parse("1John.1").is_ok(),
            "OSIS heads may start with a digit"
        );
        assert_eq!(parse("123.1").unwrap_err().reason, Reason::BadHead);
        assert_eq!(parse("L12.1").unwrap_err().reason, Reason::BadHead);
        assert_eq!(parse("Gen-.1").unwrap_err().reason, Reason::Empty);
        assert!(parse("L").is_ok(), "`L` alone is a head, not a line");
    }

    #[test]
    fn a_range_is_two_ends_of_one_kind_in_order() {
        assert_eq!(parse("Gen.1.1-L3").unwrap_err().reason, Reason::MixedRange);
        assert_eq!(parse("L9-L2").unwrap_err().reason, Reason::BackwardRange);
        assert_eq!(
            parse("Gen.1.3-Gen.1.1").unwrap_err().reason,
            Reason::BackwardRange
        );
        // A pointer is never split on `-`, so `/a-/b` is one pointer whose first token is
        // `a-` rather than a range of two. There is no syntax for a pointer range at all,
        // which is how a meaning nobody has defined stays undefined.
        assert_eq!(
            parse("/a-/b").unwrap(),
            Locator::Pointer(vec!["a-".into(), "b".into()])
        );
        assert_eq!(
            parse("L1-/b").unwrap_err().reason,
            Reason::PointerRange,
            "a range end that is a pointer is refused where it is read"
        );
        assert_eq!(
            Locator::range(parse("/a").unwrap(), parse("/b").unwrap())
                .unwrap_err()
                .reason,
            Reason::PointerRange
        );
        assert_eq!(
            parse("L1-L2-L3").unwrap_err().reason,
            Reason::BadHead,
            "the first `-` splits, so the tail is `L2-L3`, which is not a locator at all"
        );
    }

    /// Equal ends collapse, so a corpus cannot hold both spellings of one place.
    #[test]
    fn a_range_whose_ends_are_equal_is_the_single_locator() {
        assert_eq!(parse("L7-L7").unwrap(), Locator::Line(7));
        assert_eq!(parse("L7-L7").unwrap().to_string(), "L7");
        assert_eq!(
            Locator::range(Locator::Line(7), Locator::Line(7)).unwrap(),
            Locator::Line(7)
        );
    }

    #[test]
    fn ends_gives_a_single_locator_its_own_hull() {
        let single = parse("Gen.1.1").unwrap();
        let (a, b) = single.ends();
        assert_eq!(a, b);
        let r = parse("Gen.1.1-Gen.1.3").unwrap();
        let (a, b) = r.ends();
        assert_eq!(a.to_string(), "Gen.1.1");
        assert_eq!(b.to_string(), "Gen.1.3");
    }

    #[test]
    fn containment_is_prefix_for_canonicals_and_pointers() {
        let chapter = parse("Gen.1").unwrap();
        assert!(chapter.contains(&parse("Gen.1.1").unwrap()));
        assert!(chapter.contains(&chapter));
        assert!(!chapter.contains(&parse("Gen.2.1").unwrap()));
        assert!(!chapter.contains(&parse("Exod.1.1").unwrap()));
        assert!(!parse("Gen.1.1").unwrap().contains(&chapter));

        let msgs = parse("/messages").unwrap();
        assert!(msgs.contains(&parse("/messages/3/text").unwrap()));
        assert!(!msgs.contains(&parse("/other").unwrap()));

        let r = parse("Gen.1.1-Gen.1.5").unwrap();
        assert!(r.contains(&parse("Gen.1.3").unwrap()));
        assert!(r.contains(&parse("Gen.1.2-Gen.1.4").unwrap()));
        assert!(!r.contains(&parse("Gen.1.6").unwrap()));
    }

    #[test]
    fn ordering_is_by_head_then_by_step() {
        let mut v = [
            parse("Gen.1.10").unwrap(),
            parse("Gen.1.2").unwrap(),
            parse("Gen.1").unwrap(),
            parse("Exod.1.1").unwrap(),
        ];
        v.sort();
        let got: Vec<String> = v.iter().map(|l| l.to_string()).collect();
        assert_eq!(got, ["Exod.1.1", "Gen.1", "Gen.1.2", "Gen.1.10"]);
    }

    #[test]
    fn the_constructors_refuse_what_the_parser_refuses() {
        assert_eq!(Locator::line(0).unwrap_err().reason, Reason::BadNumber);
        assert_eq!(
            Locator::canonical("L9", &[1]).unwrap_err().reason,
            Reason::BadHead
        );
        assert_eq!(
            Locator::canonical("", &[]).unwrap_err().reason,
            Reason::Empty
        );
        assert_eq!(
            Locator::pointer::<&str>(&[]).unwrap_err().reason,
            Reason::Empty
        );
        assert_eq!(
            Locator::pointer(&["a b"]).unwrap_err().reason,
            Reason::BadCharacter
        );
        assert_eq!(
            Locator::range(parse("Gen.1").unwrap(), parse("Gen.1.1-Gen.1.2").unwrap())
                .unwrap_err()
                .reason,
            Reason::NestedRange
        );
    }

    #[test]
    fn from_str_is_the_parser() {
        let l: Locator = "Gen.1.1".parse().unwrap();
        assert_eq!(l, parse("Gen.1.1").unwrap());
    }
}
