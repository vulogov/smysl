//! Line classification (§15.2).
//!
//! A cheap pass that assigns every line exactly one class. It exists for one reason: on
//! any parse error, recovery skips forward to the next `RecordStart` at column 0. That is
//! what lets a file with one malformed record still yield every other record, which in
//! turn is what lets the ingest repair loop resend only the offending span (§22.3).

use crate::diag::Span;
use crate::ids::{KernelType, SchemaId};

/// What a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum LineClass {
    /// `@doc smysl/0.1 { … }`
    DocHeader,
    /// `@claim c/x { … }` - any kernel or extension type at column 0.
    RecordStart,
    /// `@rel a --kind--> b`
    RelLine,
    /// `@thread t/x { … }`
    ThreadStart,
    /// `  role → ref`
    Step,
    /// `~ the gist`
    Gist,
    /// `--` alone on its line.
    Separator,
    Blank,
    /// `# a note` or `// a note` at column 0.
    ///
    /// Classified here and *interpreted by the parser*, because the same line means two
    /// different things depending on where it lands. Between records it is a comment and
    /// is skipped. Inside a body it is prose - a body is free text, and `# Heading` is a
    /// perfectly ordinary thing to write in it, so treating that as a comment would
    /// silently delete a reader's words. `block()` keeps it; `run()` drops it.
    Comment,
    /// Anything else: body, detail, or a gist continuation.
    Text,
}

impl LineClass {
    /// Whether this class begins a new record, and so is a recovery point.
    pub const fn starts_record(self) -> bool {
        matches!(
            self,
            LineClass::DocHeader
                | LineClass::RecordStart
                | LineClass::RelLine
                | LineClass::ThreadStart
        )
    }
}

/// One classified line, with the byte range it occupies excluding its terminator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Line<'a> {
    pub class: LineClass,
    pub span: Span,
    pub text: &'a str,
    /// One-based, for human-facing messages.
    pub number: usize,
}

impl Line<'_> {
    /// The text after the leading sigil, for `Gist` lines.
    pub fn gist_text(&self) -> &str {
        self.text.trim_start_matches('~').trim_start()
    }
}

/// Classify every line of `src`.
pub fn lex(src: &str) -> Vec<Line<'_>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for (n, raw) in src.split('\n').enumerate() {
        // `split` keeps a trailing empty field for a final newline; drop it so a file
        // ending in `\n` does not gain a phantom blank line.
        let is_last = offset + raw.len() >= src.len();
        let text = raw.strip_suffix('\r').unwrap_or(raw);
        if !(is_last && text.is_empty() && offset > 0) {
            out.push(Line {
                class: classify(text),
                span: Span::new(offset, offset + text.len()),
                text,
                number: n + 1,
            });
        }
        offset += raw.len() + 1;
    }
    out
}

fn classify(line: &str) -> LineClass {
    if line.trim().is_empty() {
        return LineClass::Blank;
    }
    if line.trim_end() == "--" && !line.starts_with(' ') {
        return LineClass::Separator;
    }
    // Column 0 only. An indented `#` is inside a body or a step, where it is prose.
    // Both markers, because an HJSON header already accepts both inside a record - the
    // surface contradicted itself by rejecting between records what it took within one.
    if line.starts_with('#') || line.starts_with("//") {
        return LineClass::Comment;
    }
    if let Some(rest) = line.strip_prefix('@') {
        let word = rest
            .split(|c: char| c.is_whitespace() || c == '{')
            .next()
            .unwrap_or("");
        return match word {
            "doc" => LineClass::DocHeader,
            "rel" => LineClass::RelLine,
            "thread" => LineClass::ThreadStart,
            w if is_record_type(w) => LineClass::RecordStart,
            // A type this build does not know, which a later version may have added. The
            // writer emits exactly the type string it decoded - it has to, since the type
            // is hashed - so refusing it here meant emitting documents this parser
            // rejected.
            //
            // Gated on what *follows* rather than on the word, because the word alone
            // cannot distinguish a future kernel type from a typo or an @mention. A record
            // start carries a label or an open brace; `@vulogov mentioned this` carries
            // neither and stays body text.
            w if is_ext_segment_word(w) && has_label_or_header(rest) => LineClass::RecordStart,
            _ => LineClass::Text,
        };
    }
    if line.starts_with("~ ") || line == "~" || line.starts_with("~\t") {
        return LineClass::Gist;
    }
    if is_step(line) {
        return LineClass::Step;
    }
    LineClass::Text
}

/// A recognised unit type: a kernel type, or an `x.<domain>/<type>` extension.
///
/// Only recognised types start a record, so a body line beginning with `@` - an email
/// address, a mention - stays body text rather than silently truncating the record.
pub fn is_record_type(word: &str) -> bool {
    if KernelType::parse(word).is_some() {
        return true;
    }
    word.starts_with("x.") && SchemaId::parse(word).is_ok()
}

/// Whether a word could name a type at all: lowercase, digits, `-`, `.`, non-empty.
///
/// Deliberately not "is a known type" - the point is to admit a type this build has never
/// heard of - but still narrow enough that punctuation and mixed case do not qualify.
fn is_ext_segment_word(w: &str) -> bool {
    !w.is_empty()
        && w.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'.')
}

/// Whether what follows an unrecognised `@word` looks like a record rather than prose.
///
/// A label (`ns/name`) or an open brace. This is what keeps an `@mention` in a body from
/// silently truncating the unit it sits inside.
fn has_label_or_header(rest: &str) -> bool {
    let after = match rest.find(|c: char| c.is_whitespace() || c == '{') {
        Some(i) => &rest[i..],
        None => return false,
    };
    let trimmed = after.trim_start();
    if trimmed.starts_with('{') {
        return true;
    }
    let next = match trimmed.split_whitespace().next() {
        Some(w) => w,
        None => return false,
    };
    match next.split_once('/') {
        Some((ns, name)) => !ns.is_empty() && !name.is_empty() && is_ext_segment_word(ns),
        None => false,
    }
}

/// `  role → ref` or `  role -> ref`, at exactly the two-space step indent.
fn is_step(line: &str) -> bool {
    let Some(rest) = line.strip_prefix("  ") else {
        return false;
    };
    if rest.starts_with(' ') {
        return false;
    }
    let Some(arrow) = find_arrow(rest) else {
        return false;
    };
    let role = rest[..arrow].trim_end();
    !role.is_empty()
        && role
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c == b'-' || c.is_ascii_digit())
}

/// The byte offset of the first `→` or `->`, if any.
pub fn find_arrow(s: &str) -> Option<usize> {
    let uni = s.find('\u{2192}');
    let ascii = s.find("->");
    match (uni, ascii) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// The width in bytes of the arrow at `at`.
pub fn arrow_len(s: &str, at: usize) -> usize {
    if s[at..].starts_with('\u{2192}') {
        '\u{2192}'.len_utf8()
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes(src: &str) -> Vec<LineClass> {
        lex(src).into_iter().map(|l| l.class).collect()
    }

    #[test]
    fn classifies_the_rfc_example() {
        let src = "@doc smysl/0.1 {\n  intent: incident-brief\n}\n\n@claim c/x {\n  status: measured\n}\n~ a gist\n\nbody text\n\n--\ndetail text\n\n@rel a/b --causes--> c/d\n\n@thread t/brief { schema: brief }\n~ thread gist\n  bottom-line \u{2192} c/x\n";
        let c = classes(src);
        assert_eq!(c[0], LineClass::DocHeader);
        assert_eq!(c[4], LineClass::RecordStart);
        assert_eq!(c[7], LineClass::Gist);
        assert_eq!(c[9], LineClass::Text);
        assert_eq!(c[11], LineClass::Separator);
        assert_eq!(c[14], LineClass::RelLine);
        assert_eq!(c[16], LineClass::ThreadStart);
        assert_eq!(c[18], LineClass::Step);
    }

    #[test]
    fn every_kernel_type_starts_a_record() {
        for &k in KernelType::ALL {
            assert_eq!(
                classify(&format!("@{k} c/x {{}}")),
                LineClass::RecordStart,
                "@{k}"
            );
        }
    }

    #[test]
    fn extension_types_start_a_record() {
        assert_eq!(classify("@x.sre/incident i/1 {}"), LineClass::RecordStart);
        assert_eq!(classify("@x.sre/incident"), LineClass::RecordStart);
    }

    /// An `@` in prose must not truncate a record. This is why classification consults
    /// the type set rather than just the sigil.
    #[test]
    /// An `@word` in prose stays prose. What decides is whether a *label or a header*
    /// follows, not whether the word is a type this build knows - because a kernel type
    /// added in a later version is indistinguishable from an unknown word, and refusing it
    /// meant the writer emitted documents this parser rejected.
    fn an_unrecognised_at_word_is_body_text() {
        // A mention, an address, a sentence: no label, no brace, so prose.
        assert_eq!(classify("@vladimir please look"), LineClass::Text);
        assert_eq!(classify("@ claim"), LineClass::Text);
        assert_eq!(classify("@x.sre"), LineClass::Text);
        assert_eq!(classify("@postmortem and then some prose"), LineClass::Text);
        assert_eq!(classify("@team see the thread"), LineClass::Text);
    }

    /// The other side of that boundary, and a deliberate change: `@claimant c/x` used to
    /// be body text and is now a record of an unrecognised type. It is shaped exactly like
    /// a record - a type, then a label - and a version that adds `claimant` would write it
    /// this way. `check` reports every one as `SMY-W010` rather than passing it in silence.
    #[test]
    fn an_unrecognised_type_with_a_label_or_header_starts_a_record() {
        assert_eq!(classify("@claimant c/x"), LineClass::RecordStart);
        assert_eq!(classify("@postmortem p/a { status: speculative }"), LineClass::RecordStart);
        assert_eq!(classify("@postmortem { status: speculative }"), LineClass::RecordStart);
        // Mixed case and punctuation cannot name a type, so they stay prose either way.
        assert_eq!(classify("@Postmortem p/a"), LineClass::Text);
        assert_eq!(classify("@post!mortem p/a"), LineClass::Text);
    }

    #[test]
    fn a_separator_must_be_alone_on_its_line() {
        assert_eq!(classify("--"), LineClass::Separator);
        assert_eq!(classify("--  "), LineClass::Separator);
        assert_eq!(classify("-- not alone"), LineClass::Text);
        assert_eq!(classify("  --"), LineClass::Text);
        assert_eq!(classify("a -- b"), LineClass::Text);
    }

    #[test]
    fn gist_lines_need_the_sigil_and_a_space() {
        assert_eq!(classify("~ the gist"), LineClass::Gist);
        assert_eq!(classify("~"), LineClass::Gist);
        assert_eq!(classify("~tilde-prefixed prose"), LineClass::Text);
        assert_eq!(classify("  ~ indented"), LineClass::Text);
    }

    #[test]
    fn steps_need_the_two_space_indent_and_an_arrow() {
        assert_eq!(classify("  bottom-line \u{2192} c/x"), LineClass::Step);
        assert_eq!(classify("  support -> c/y"), LineClass::Step);
        assert_eq!(classify("bottom-line -> c/x"), LineClass::Text);
        assert_eq!(classify("    support -> c/y"), LineClass::Text);
        assert_eq!(classify("  support c/y"), LineClass::Text);
        assert_eq!(classify("  -> c/y"), LineClass::Text);
    }

    /// Markdown lists are indented text, not steps.
    #[test]
    fn a_markdown_list_is_not_a_step() {
        assert_eq!(classify("  - an item"), LineClass::Text);
        assert_eq!(classify("  1. an item"), LineClass::Text);
    }

    #[test]
    fn blank_lines_are_recognised_however_they_are_spelled() {
        for s in ["", " ", "\t", "   \t "] {
            assert_eq!(classify(s), LineClass::Blank, "{s:?}");
        }
    }

    #[test]
    fn spans_cover_the_line_without_its_terminator() {
        let src = "abc\ndef\n";
        let l = lex(src);
        assert_eq!(l.len(), 2);
        assert_eq!(l[0].span, Span::new(0, 3));
        assert_eq!(l[1].span, Span::new(4, 7));
        assert_eq!(l[0].span.slice(src), Some("abc"));
    }

    #[test]
    fn a_trailing_newline_does_not_add_a_line() {
        assert_eq!(lex("a\n").len(), 1);
        assert_eq!(lex("a").len(), 1);
        assert_eq!(lex("a\n\n").len(), 2);
        assert_eq!(lex("").len(), 1);
    }

    #[test]
    fn carriage_returns_are_stripped_from_the_text() {
        let l = lex("abc\r\ndef");
        assert_eq!(l[0].text, "abc");
        assert_eq!(l[0].class, LineClass::Text);
    }

    #[test]
    fn record_starts_are_the_recovery_points() {
        for c in [
            LineClass::DocHeader,
            LineClass::RecordStart,
            LineClass::RelLine,
            LineClass::ThreadStart,
        ] {
            assert!(c.starts_record());
        }
        for c in [
            LineClass::Step,
            LineClass::Gist,
            LineClass::Separator,
            LineClass::Blank,
            LineClass::Text,
        ] {
            assert!(!c.starts_record());
        }
    }

    #[test]
    fn arrows_are_found_in_both_spellings() {
        assert_eq!(find_arrow("a \u{2192} b"), Some(2));
        assert_eq!(find_arrow("a -> b"), Some(2));
        assert_eq!(find_arrow("a b"), None);
        assert_eq!(arrow_len("a \u{2192} b", 2), 3);
        assert_eq!(arrow_len("a -> b", 2), 2);
    }

    #[test]
    fn the_first_arrow_wins_whichever_spelling_it_is() {
        assert_eq!(find_arrow("x -> y \u{2192} z"), Some(2));
        assert_eq!(find_arrow("x \u{2192} y -> z"), Some(2));
    }

    #[test]
    fn gist_text_strips_the_sigil() {
        let l = lex("~ the gist");
        assert_eq!(l[0].gist_text(), "the gist");
        assert_eq!(lex("~")[0].gist_text(), "");
    }

    #[test]
    fn line_numbers_are_one_based() {
        let l = lex("a\nb\nc");
        assert_eq!(l[0].number, 1);
        assert_eq!(l[2].number, 3);
    }
}
