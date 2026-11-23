//! The analysis (RFC SMYSL-6 §3.2): pure and synchronous.
//!
//! Text in, plain values out, with positions already converted to the negotiated encoding. No
//! I/O, no thread, no clock, no JSON: `xtask check-purity` scans this directory for them, since
//! the crate as a whole is not pure.

mod line_index;
mod nav;

use std::collections::BTreeMap;

use smysl_check::{check, CheckOptions};
use smysl_core::surface::{parse_surface, ParseOutcome};
use smysl_core::{canonical_uid, Record, Span, Subject, Uid};
use smysl_graph::Store;

pub use line_index::{Encoding, LineIndex, Position, Range};
pub use nav::{Hover, Symbol, SymbolKind};

/// What a document is (§4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Language {
    /// `.smy`, `.smysl`: surface syntax.
    Surface,
    /// `.sq`: an `sq/1` query.
    Query,
}

impl Language {
    /// By the client's language id, then by the URI's extension.
    pub fn of(language_id: &str, uri: &str) -> Option<Language> {
        match language_id {
            "smysl" => return Some(Language::Surface),
            "smysl-query" => return Some(Language::Query),
            _ => {}
        }
        let path = uri.split(['?', '#']).next().unwrap_or(uri);
        if path.ends_with(".smy") || path.ends_with(".smysl") {
            Some(Language::Surface)
        } else if path.ends_with(".sq") {
            Some(Language::Query)
        } else {
            None
        }
    }
}

/// How bad a finding is: smysl has two severities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// One finding, placed in the text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Severity,
    /// The registry code, `SMY-E030`.
    pub code: String,
    /// The message, with the fix appended when smysl can name one.
    pub message: String,
}

/// One edit to an open document: the whole text, or a range replaced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Full(String),
    Range { range: Range, text: String },
}

/// An open document.
#[derive(Debug, Clone)]
pub struct Document {
    pub language: Language,
    pub version: i64,
    text: String,
    index: LineIndex,
}

impl Document {
    pub fn new(language: Language, version: i64, text: String) -> Document {
        let index = LineIndex::new(&text);
        Document {
            language,
            version,
            text,
            index,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn index(&self) -> &LineIndex {
        &self.index
    }

    /// Apply `changes` in order, as `didChange` gives them.
    pub fn apply(&mut self, changes: Vec<Change>, enc: Encoding) {
        for c in changes {
            match c {
                Change::Full(t) => self.text = t,
                Change::Range { range, text } => {
                    let a = self.index.offset(&self.text, range.start, enc);
                    let b = self.index.offset(&self.text, range.end, enc).max(a);
                    self.text.replace_range(a..b, &text);
                }
            }
            self.index = LineIndex::new(&self.text);
        }
    }

    /// The range of a byte span.
    pub fn range(&self, span: Span, enc: Encoding) -> Range {
        self.index.range(&self.text, span.start, span.end, enc)
    }
}

/// The open documents and the position encoding agreed with the client (§4.4).
#[derive(Debug, Clone, Default)]
pub struct AnalysisHost {
    docs: BTreeMap<String, Document>,
    encoding: Encoding,
}

impl AnalysisHost {
    pub fn new(encoding: Encoding) -> AnalysisHost {
        AnalysisHost {
            docs: BTreeMap::new(),
            encoding,
        }
    }

    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn open(&mut self, uri: &str, language: Language, version: i64, text: String) {
        self.docs
            .insert(uri.to_string(), Document::new(language, version, text));
    }

    /// Apply changes to an open document; `false` when it is not open.
    pub fn change(&mut self, uri: &str, version: i64, changes: Vec<Change>) -> bool {
        let enc = self.encoding;
        match self.docs.get_mut(uri) {
            Some(d) => {
                d.apply(changes, enc);
                d.version = version;
                true
            }
            None => false,
        }
    }

    pub fn close(&mut self, uri: &str) {
        self.docs.remove(uri);
    }

    pub fn document(&self, uri: &str) -> Option<&Document> {
        self.docs.get(uri)
    }

    /// A surface document's parse, indexed for navigation; `None` for a query, a document
    /// not open, or one whose header does not parse.
    fn model(&self, uri: &str) -> Option<nav::Model<'_>> {
        let doc = self.docs.get(uri)?;
        if doc.language != Language::Surface {
            return None;
        }
        let out = parse(doc).ok()?;
        Some(nav::Model::of(doc, out))
    }

    fn at(&self, uri: &str, pos: Position) -> Option<usize> {
        Some(nav::offset(self.docs.get(uri)?, pos, self.encoding))
    }

    /// The card for the unit under `pos` (§5.2).
    pub fn hover(&self, uri: &str, pos: Position) -> Option<Hover> {
        self.model(uri)?.hover(self.at(uri, pos)?, self.encoding)
    }

    /// Where the unit under `pos` is declared, in this document (§5.3).
    pub fn definition(&self, uri: &str, pos: Position) -> Option<Range> {
        self.model(uri)?
            .definition(self.at(uri, pos)?, self.encoding)
    }

    /// Every reference to the unit under `pos`, and its declaration when asked (§5.3).
    pub fn references(&self, uri: &str, pos: Position, declaration: bool) -> Vec<Range> {
        match (self.model(uri), self.at(uri, pos)) {
            (Some(m), Some(at)) => m.references(at, declaration, self.encoding),
            _ => Vec::new(),
        }
    }

    /// The document's outline, in text order (§5.3).
    pub fn symbols(&self, uri: &str) -> Vec<Symbol> {
        self.model(uri)
            .map(|m| m.symbols(self.encoding))
            .unwrap_or_default()
    }

    /// Every finding in an open document, in text order (§5.1); none for one not open.
    pub fn diagnostics(&self, uri: &str) -> Vec<Diagnostic> {
        let Some(doc) = self.docs.get(uri) else {
            return Vec::new();
        };
        let mut out = match doc.language {
            Language::Surface => surface_diagnostics(doc, self.encoding),
            Language::Query => query_diagnostics(doc, self.encoding),
        };
        out.sort();
        out.dedup();
        out
    }
}

/// A smysl diagnostic placed at `span`.
fn placed(doc: &Document, enc: Encoding, d: &smysl_core::Diagnostic, span: Span) -> Diagnostic {
    let mut message = d.message.clone();
    if let Some(s) = &d.suggestion {
        message.push_str("\nfix: ");
        message.push_str(s);
    }
    Diagnostic {
        range: doc.range(span, enc),
        severity: if d.is_error() {
            Severity::Error
        } else {
            Severity::Warning
        },
        code: d.code.to_string(),
        message,
    }
}

/// A finding with no place of its own sits on the first line.
fn first_line(doc: &Document) -> Span {
    let end = doc.text.find(['\n', '\r']).unwrap_or(doc.text.len());
    Span::new(0, end)
}

/// Where each record was written, by what a check finding names it by (§4.2).
struct Places {
    units: BTreeMap<Uid, Span>,
    records: BTreeMap<[u8; 32], Span>,
}

impl Places {
    fn of(out: &ParseOutcome) -> Places {
        let (mut units, mut records) = (BTreeMap::new(), BTreeMap::new());
        for (r, s) in out.records.iter().zip(&out.spans) {
            let Some(s) = s else { continue };
            if let Record::Unit(u) = r {
                units.entry(canonical_uid(u)).or_insert(*s);
            }
            records.entry(Store::record_hash(r)).or_insert(*s);
        }
        Places { units, records }
    }

    fn span(&self, subject: &Subject) -> Option<Span> {
        match subject {
            Subject::Span(s) => Some(*s),
            Subject::Unit(u) => self.units.get(u).copied(),
            Subject::Record(h) => self.records.get(h).copied(),
            _ => None,
        }
    }
}

/// The surface document's parse, and the parse outcome's record places.
pub fn parse(doc: &Document) -> Result<ParseOutcome, smysl_core::error::ParseError> {
    parse_surface(&doc.text)
}

fn surface_diagnostics(doc: &Document, enc: Encoding) -> Vec<Diagnostic> {
    let out = match parse(doc) {
        Ok(o) => o,
        Err(e) => {
            let span = match &e {
                smysl_core::error::ParseError::Syntax { span, .. } => *span,
                _ => first_line(doc),
            };
            let d = smysl_core::Diagnostic::new(e.code()).with_message(e.to_string());
            return vec![placed(doc, enc, &d, span)];
        }
    };
    // As `smysl check FILE` does: the document's records, its labels, its parse findings.
    let store = Store::from_records(out.records.clone());
    let mut report = check(
        &store,
        CheckOptions::default().with_labels(out.labels.clone()),
    );
    for d in &out.diagnostics {
        report.push(d.clone());
    }
    let places = Places::of(&out);
    report
        .diagnostics
        .iter()
        .map(|d| {
            let span = places.span(&d.subject).unwrap_or_else(|| first_line(doc));
            placed(doc, enc, d, span)
        })
        .collect()
}

fn query_diagnostics(doc: &Document, enc: Encoding) -> Vec<Diagnostic> {
    match smysl_query::parse(&doc.text) {
        Ok(_) => Vec::new(),
        Err(d) => {
            let span = match d.subject {
                Subject::Span(s) => s,
                _ => first_line(doc),
            };
            vec![placed(doc, enc, &d, span)]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "@doc smysl/0.1 { id: v/t }\n\n\
        @evidence e/a { status: speculative }\n~ A guess.\n\n\
        @claim c/b { status: derived, grounds: [e/a] }\n~ 語 on a guess 😀.\n\n\
        @rel c/b --elaborates--> e/zz\n";

    fn host(text: &str) -> AnalysisHost {
        let mut h = AnalysisHost::new(Encoding::Utf16);
        h.open("file:///t.smy", Language::Surface, 1, text.to_string());
        h
    }

    #[test]
    fn a_check_finding_lands_on_its_unit_and_a_parse_finding_on_its_span() {
        let h = host(DOC);
        let ds = h.diagnostics("file:///t.smy");
        let e030 = ds.iter().find(|d| d.code == "SMY-E030").expect("{ds:?}");
        // Rule M: derived over a speculative ground; the finding names the claim's record.
        assert_eq!(e030.range.start, Position::new(5, 0));
        assert_eq!(e030.severity, Severity::Error);
        let e060 = ds.iter().find(|d| d.code == "SMY-E060").expect("{ds:?}");
        assert_eq!(e060.range.start.line, 8);
        assert!(e060.message.contains("e/zz"), "{}", e060.message);
    }

    #[test]
    fn the_same_text_gives_the_same_findings() {
        let a = host(DOC).diagnostics("file:///t.smy");
        let b = host(DOC).diagnostics("file:///t.smy");
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }

    #[test]
    fn an_incremental_change_is_applied_in_utf16() {
        let mut h = host(DOC);
        // Replace `speculative` with `measured, source: { kind: metric, ref: "m" }`.
        let line = 2;
        let col = "@evidence e/a { status: ".len() as u32;
        let r = Range::new(
            Position::new(line, col),
            Position::new(line, col + "speculative".len() as u32),
        );
        assert!(h.change(
            "file:///t.smy",
            2,
            vec![Change::Range {
                range: r,
                text: "measured, source: { kind: metric, ref: \"m\" }".into()
            }]
        ));
        let ds = h.diagnostics("file:///t.smy");
        assert!(ds.iter().all(|d| d.code != "SMY-E030"), "{ds:?}");
        assert_eq!(h.document("file:///t.smy").unwrap().version, 2);
    }

    #[test]
    fn a_broken_header_and_a_query_are_placed() {
        let h = host("@doc smysl/9.0 { id: v/t }\n");
        let ds = h.diagnostics("file:///t.smy");
        assert_eq!(ds.len(), 1, "{ds:?}");
        assert_eq!(ds[0].range.start, Position::new(0, 0));
        let mut q = AnalysisHost::new(Encoding::Utf8);
        q.open("file:///q.sq", Language::Query, 1, "find where (".into());
        let ds = q.diagnostics("file:///q.sq");
        assert_eq!(ds.len(), 1, "{ds:?}");
        q.open("file:///q.sq", Language::Query, 2, "find claims".into());
        let _ = q.diagnostics("file:///q.sq");
    }

    #[test]
    fn languages_by_id_and_by_extension() {
        assert_eq!(Language::of("smysl", "x"), Some(Language::Surface));
        assert_eq!(
            Language::of("", "file:///a/b.smysl"),
            Some(Language::Surface)
        );
        assert_eq!(
            Language::of("plaintext", "file:///a/q.sq"),
            Some(Language::Query)
        );
        assert_eq!(Language::of("rust", "file:///a/b.rs"), None);
    }
}
