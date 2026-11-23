//! Hover and navigation (RFC SMYSL-6 §5.2–§5.3, LS-P2): what the cursor is on, what it is,
//! where it is declared, and where else it is named.
//!
//! Everything is read from one parse of the document: the records with their spans, and the
//! references with theirs (`ParseOutcome::spans` and `refs`). A unit is found under the cursor
//! by a reference to it, by the label in its own header, or by a `b3:` uid written out.

use std::collections::BTreeMap;

use smysl_core::surface::ParseOutcome;
use smysl_core::{canonical_uid, Record, RelKind, Span, Uid, UnitCore};
use smysl_graph::{effective_status, RetractionPolicy, Store};

use super::{Document, Encoding, Position, Range};

/// What [`super::AnalysisHost::hover`] shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The token hovered.
    pub range: Range,
    /// The card, in Markdown.
    pub markdown: String,
}

/// What a document symbol is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SymbolKind {
    /// The `@doc` header.
    Doc,
    /// An `@schema` declaration.
    Schema,
    Unit,
    Relation,
    Thread,
}

/// One entry of the document's outline.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Symbol {
    pub range: Range,
    /// The part to select: a unit's label, or the record's first line.
    pub selection: Range,
    pub name: String,
    pub detail: String,
    pub kind: SymbolKind,
}

/// One document's parse, indexed for navigation.
pub(crate) struct Model<'a> {
    doc: &'a Document,
    out: ParseOutcome,
    /// Each unit's record span.
    decls: BTreeMap<Uid, Span>,
    /// The label written in each unit's own header, and its span.
    label_at: BTreeMap<Uid, (String, Span)>,
}

/// The first line of a span, without its terminator.
fn first_line(text: &str, s: Span) -> Span {
    let end = text[s.start..s.end]
        .find(['\n', '\r'])
        .map_or(s.end, |i| s.start + i);
    Span::new(s.start, end)
}

/// In `@claim c/b { … }`, the span of `c/b`.
fn header_label(text: &str, record: Span) -> Option<(String, Span)> {
    let line = first_line(text, record);
    let head = &text[line.start..line.end];
    let after_type = head.find(char::is_whitespace)?;
    let rest = &head[after_type..];
    let lead = rest.len() - rest.trim_start().len();
    let token: String = rest
        .trim_start()
        .chars()
        .take_while(|c| !c.is_whitespace() && *c != '{')
        .collect();
    if token.is_empty() || token.parse::<smysl_core::Label>().is_err() {
        return None;
    }
    let start = line.start + after_type + lead;
    Some((token.clone(), Span::new(start, start + token.len())))
}

impl<'a> Model<'a> {
    /// A document with errors in some records still has the others.
    pub(crate) fn of(doc: &'a Document, out: ParseOutcome) -> Model<'a> {
        let mut m = Model {
            doc,
            out,
            decls: BTreeMap::new(),
            label_at: BTreeMap::new(),
        };
        let text = doc.text();
        for (r, s) in m.out.records.iter().zip(&m.out.spans) {
            if let (Record::Unit(_), Some(s)) = (r, s) {
                let uid = canonical_uid(r.as_unit().expect("a unit"));
                m.decls.entry(uid).or_insert(*s);
                if let Some(l) = header_label(text, *s) {
                    m.label_at.entry(uid).or_insert(l);
                }
            }
        }
        m
    }

    fn unit(&self, uid: &Uid) -> Option<&UnitCore> {
        self.out.units().find(|u| canonical_uid(u) == *uid)
    }

    /// How a uid reads in this document: its label, or a short uid.
    fn name(&self, uid: &Uid) -> String {
        self.out
            .labels
            .iter()
            .find(|(_, u)| *u == uid)
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| uid.short())
    }

    /// The unit under byte `at`, and the span of the token naming it.
    pub(crate) fn target(&self, at: usize) -> Option<(Span, Uid)> {
        let within = |s: &Span| s.start <= at && at <= s.end;
        if let Some((s, u)) = self.out.refs.iter().find(|(s, _)| within(s)) {
            return Some((*s, *u));
        }
        if let Some((u, (_, s))) = self.label_at.iter().find(|(_, (_, s))| within(s)) {
            return Some((*s, *u));
        }
        // A uid written out, `b3:` and its base32.
        let text = self.doc.text();
        let is_uid_char = |c: char| c.is_ascii_alphanumeric() || c == ':';
        let at = at.min(text.len());
        if !text.is_char_boundary(at) {
            return None;
        }
        let start = text[..at]
            .char_indices()
            .rev()
            .take_while(|(_, c)| is_uid_char(*c))
            .last()
            .map_or(at, |(i, _)| i);
        let end = text[at..]
            .char_indices()
            .find(|(_, c)| !is_uid_char(*c))
            .map_or(text.len(), |(i, _)| at + i);
        let token = &text[start..end];
        let uid: Uid = token.strip_prefix("b3:").and(token.parse().ok())?;
        Some((Span::new(start, end), uid))
    }

    /// The span a definition goes to: the label in the unit's header, or its first line.
    fn definition_span(&self, uid: &Uid) -> Option<Span> {
        if let Some((_, s)) = self.label_at.get(uid) {
            return Some(*s);
        }
        self.decls.get(uid).map(|s| first_line(self.doc.text(), *s))
    }

    pub(crate) fn definition(&self, at: usize, enc: Encoding) -> Option<Range> {
        let (_, uid) = self.target(at)?;
        Some(self.doc.range(self.definition_span(&uid)?, enc))
    }

    pub(crate) fn references(&self, at: usize, declaration: bool, enc: Encoding) -> Vec<Range> {
        let Some((_, uid)) = self.target(at) else {
            return Vec::new();
        };
        let mut spans: Vec<Span> = self
            .out
            .refs
            .iter()
            .filter(|(_, u)| *u == uid)
            .map(|(s, _)| *s)
            .collect();
        if declaration {
            spans.extend(self.definition_span(&uid));
        }
        spans.sort_by_key(|s| (s.start, s.end));
        spans.dedup();
        spans.into_iter().map(|s| self.doc.range(s, enc)).collect()
    }

    pub(crate) fn hover(&self, at: usize, enc: Encoding) -> Option<Hover> {
        let (span, uid) = self.target(at)?;
        let range = self.doc.range(span, enc);
        let Some(u) = self.unit(&uid) else {
            return Some(Hover {
                range,
                markdown: format!("`{uid}`\n\nNot declared in this document."),
            });
        };
        let store = Store::from_records(self.out.records.clone());
        let effective = effective_status(&store, RetractionPolicy::default()).get(&uid);
        let profile = smysl_check::store_granularity(&store);
        let tokens = profile
            .tokens(&u.gist)
            .unwrap_or_else(|| smysl_core::types::tokens(&u.gist));
        let mut md = format!("**{}** · `@{}` · {}", self.name(&uid), u.schema, u.status);
        if let Some(e) = effective.filter(|e| *e != u.status) {
            md.push_str(&format!(" (effective: **{e}**)"));
        }
        md.push_str(&format!("\n\n`{uid}`\n\n> {}\n\n", u.gist));
        let over = if tokens > profile.l0_max {
            " — over"
        } else {
            ""
        };
        md.push_str(&format!(
            "gist: {tokens} tokens of `l0_max` {}{over}\n\n",
            profile.l0_max
        ));
        let list = |set: &std::collections::BTreeSet<Uid>| -> String {
            set.iter()
                .map(|g| match self.unit(g) {
                    Some(gu) => format!("{} ({})", self.name(g), gu.status),
                    None => self.name(g),
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !u.grounds.is_empty() {
            md.push_str(&format!("grounds: {}\n\n", list(&u.grounds)));
        }
        if !u.deps.is_empty() {
            md.push_str(&format!("deps: {}\n\n", list(&u.deps)));
        }
        let rebutted = self
            .out
            .records
            .iter()
            .filter(
                |r| matches!(r, Record::Relation(x) if x.kind == RelKind::Rebuts && x.to == uid),
            )
            .count();
        if rebutted > 0 {
            md.push_str(&format!(
                "rebutted by {rebutted} unit{}\n\n",
                if rebutted == 1 { "" } else { "s" }
            ));
        }
        if let Some(s) = &u.source {
            md.push_str(&format!("source: {} `{}`\n\n", s.kind, s.reference));
        }
        Some(Hover {
            range,
            markdown: md.trim_end().to_string(),
        })
    }

    pub(crate) fn symbols(&self, enc: Encoding) -> Vec<Symbol> {
        let text = self.doc.text();
        let mut out = Vec::new();
        // The `@doc` header: the view, which has no record span of its own.
        if let Some(at) = text
            .match_indices("@doc")
            .map(|(i, _)| i)
            .find(|i| *i == 0 || text[..*i].ends_with(['\n', '\r']))
        {
            let line = first_line(text, Span::new(at, text.len()));
            let name = self
                .out
                .view
                .as_ref()
                .map(|v| v.id.to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "@doc".to_string());
            out.push(Symbol {
                range: self.doc.range(line, enc),
                selection: self.doc.range(line, enc),
                name,
                detail: self.out.format_version.clone(),
                kind: SymbolKind::Doc,
            });
        }
        for (r, s) in self.out.records.iter().zip(&self.out.spans) {
            let Some(s) = *s else { continue };
            let line = first_line(text, s);
            let (name, detail, kind, selection) = match r {
                Record::Unit(u) => {
                    let uid = canonical_uid(u);
                    let sel = self.label_at.get(&uid).map_or(line, |(_, l)| *l);
                    let gist: String = u.gist.chars().take(60).collect();
                    (
                        self.name(&uid),
                        format!("@{} · {} — {gist}", u.schema, u.status),
                        SymbolKind::Unit,
                        sel,
                    )
                }
                Record::Relation(x) => (
                    format!(
                        "{} --{}--> {}",
                        self.name(&x.from),
                        x.kind,
                        self.name(&x.to)
                    ),
                    String::new(),
                    SymbolKind::Relation,
                    line,
                ),
                Record::Thread(t) => (t.id.to_string(), t.gist.clone(), SymbolKind::Thread, line),
                Record::SchemaDecl(d) => (
                    d.id.to_string(),
                    format!("version {}", d.version),
                    SymbolKind::Schema,
                    line,
                ),
                _ => continue,
            };
            out.push(Symbol {
                range: self.doc.range(s, enc),
                selection: self.doc.range(selection, enc),
                name,
                detail,
                kind,
            });
        }
        out.sort();
        out
    }
}

/// The byte offset of `pos` in `doc`.
pub(crate) fn offset(doc: &Document, pos: Position, enc: Encoding) -> usize {
    doc.index().offset(doc.text(), pos, enc)
}

#[cfg(test)]
mod tests {
    use super::super::{AnalysisHost, Language};
    use super::*;

    const DOC: &str = "@doc smysl/0.1 { id: v/n }\n\n\
        @schema x.ops/1 { version: 1, relations: [\"x.ops/blocks\"] }\n\n\
        @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ 語 rose 😀.\n\n\
        @claim c/b { status: derived, grounds: [e/a] }\n~ It regressed.\n\n\
        @claim c/r { status: speculative }\n~ It did not.\n\n\
        @rel c/r --rebuts--> c/b\n\
        @rel c/b --elaborates--> e/a\n";

    fn host() -> AnalysisHost {
        let mut h = AnalysisHost::new(Encoding::Utf16);
        h.open("file:///n.smy", Language::Surface, 1, DOC.to_string());
        h
    }

    fn pos_of(needle: &str, nth: usize, plus: u32) -> Position {
        let at = DOC.match_indices(needle).nth(nth).unwrap().0;
        let ix = super::super::LineIndex::new(DOC);
        let mut p = ix.position(DOC, at, Encoding::Utf16);
        p.character += plus;
        p
    }

    #[test]
    fn definition_goes_from_a_reference_to_the_label_in_the_header() {
        let h = host();
        // `e/a` in `grounds: [e/a]` is the second `e/a` in the text.
        let def = h.definition("file:///n.smy", pos_of("e/a", 1, 1)).unwrap();
        assert_eq!(def.start, pos_of("e/a", 0, 0));
        assert_eq!(def.end.character - def.start.character, 3);
        // From the declaration itself, too; and a `@rel` end.
        assert_eq!(
            h.definition("file:///n.smy", pos_of("c/b", 2, 0))
                .unwrap()
                .start,
            pos_of("c/b", 0, 0)
        );
        assert!(h.definition("file:///n.smy", Position::new(0, 0)).is_none());
    }

    #[test]
    fn references_and_the_declaration() {
        let h = host();
        let refs = h.references("file:///n.smy", pos_of("e/a", 0, 0), false);
        assert_eq!(refs.len(), 2, "{refs:?}");
        let with = h.references("file:///n.smy", pos_of("e/a", 0, 0), true);
        assert_eq!(with.len(), 3);
        assert_eq!(with[0].start, pos_of("e/a", 0, 0));
    }

    #[test]
    fn hover_says_status_tokens_grounds_rebuttals_and_source() {
        let h = host();
        let c = h
            .hover("file:///n.smy", pos_of("c/b", 0, 1))
            .unwrap()
            .markdown;
        assert!(c.starts_with("**c/b** · `@claim` · derived"), "{c}");
        assert!(c.contains("> It regressed."), "{c}");
        assert!(c.contains("tokens of `l0_max` 30"), "{c}");
        assert!(c.contains("grounds: e/a (measured)"), "{c}");
        assert!(c.contains("rebutted by 1 unit"), "{c}");
        let e = h
            .hover("file:///n.smy", pos_of("e/a", 0, 0))
            .unwrap()
            .markdown;
        assert!(e.contains("source: metric `p95`"), "{e}");
        // A `b3:` uid written out is found too, declared here or not.
        let mut h2 = AnalysisHost::new(Encoding::Utf8);
        let uid = "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        h2.open(
            "file:///u.smy",
            Language::Surface,
            1,
            format!("@doc smysl/0.1 {{ id: v/u }}\n// {uid}\n"),
        );
        let hv = h2
            .hover("file:///u.smy", Position::new(1, 6))
            .expect("a uid written out");
        assert!(hv.markdown.contains("Not declared"), "{}", hv.markdown);
        assert_eq!(hv.range.start, Position::new(1, 3));
        assert_eq!(
            host()
                .hover("file:///n.smy", pos_of("c/b", 0, 1))
                .unwrap()
                .markdown,
            c,
            "deterministic"
        );
    }

    #[test]
    fn the_outline_in_text_order() {
        let h = host();
        let syms = h.symbols("file:///n.smy");
        let names: Vec<(&str, SymbolKind)> =
            syms.iter().map(|s| (s.name.as_str(), s.kind)).collect();
        assert_eq!(
            names,
            vec![
                ("v/n", SymbolKind::Doc),
                ("x.ops/1", SymbolKind::Schema),
                ("e/a", SymbolKind::Unit),
                ("c/b", SymbolKind::Unit),
                ("c/r", SymbolKind::Unit),
                ("c/r --rebuts--> c/b", SymbolKind::Relation),
                ("c/b --elaborates--> e/a", SymbolKind::Relation),
            ]
        );
        let cb = &syms[3];
        assert_eq!(cb.selection.start, pos_of("c/b", 0, 0));
        assert!(
            cb.detail.starts_with("@claim · derived — It regressed."),
            "{}",
            cb.detail
        );
    }
}
