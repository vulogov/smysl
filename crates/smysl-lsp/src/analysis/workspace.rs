//! The workspace (RFC SMYSL-6 §4.4, LS-P4): what the documents around an open one say.
//!
//! Built from texts and records the transport has read; nothing here touches a file. Two
//! sources: a library's records, read by `smysl_text::Library::read_only`, which places nothing
//! in a file, and the `.smy` documents of a folder, each parsed and kept with its text so a
//! unit declared in one can be found from another. An open document's own file is left out of
//! what it sees, since the editor's buffer is newer.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::surface::ParseOutcome;
use smysl_core::{canonical_uid, Record, Span, Uid, UnitCore};

use super::{Document, Language};

/// One document of the folder, parsed.
#[derive(Clone)]
struct File {
    doc: Document,
    out: ParseOutcome,
}

/// What surrounds the open documents: a library's records, and the folder's documents.
#[derive(Clone, Default)]
pub struct Workspace {
    library: Vec<Record>,
    files: BTreeMap<String, File>,
    /// Shards of a library left unread, named for the log.
    pub skipped: Vec<String>,
}

impl std::fmt::Debug for Workspace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Workspace")
            .field("library", &self.library.len())
            .field("files", &self.files.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Workspace {
    pub fn new() -> Workspace {
        Workspace::default()
    }

    /// A library's records.
    pub fn add_records(&mut self, records: Vec<Record>) {
        self.library.extend(records);
    }

    /// A document of the folder; one that does not parse is left out.
    pub fn add_text(&mut self, uri: &str, text: String) {
        let doc = Document::new(Language::Surface, 0, text);
        if let Ok(out) = smysl_core::surface::parse_surface(doc.text()) {
            self.files.insert(uri.to_string(), File { doc, out });
        }
    }

    /// How many records and documents it holds.
    pub fn size(&self) -> (usize, usize) {
        (self.library.len(), self.files.len())
    }

    /// Every record, the library's and each document's but `except`'s.
    pub(crate) fn records(&self, except: &str) -> Vec<Record> {
        let mut out = self.library.clone();
        for (uri, f) in &self.files {
            if uri != except {
                out.extend(f.out.records.iter().cloned());
            }
        }
        out
    }

    /// A unit by uid, from a document but `except`, or the library.
    pub(crate) fn unit(&self, uid: &Uid, except: &str) -> Option<UnitCore> {
        self.files
            .iter()
            .filter(|(u, _)| *u != except)
            .flat_map(|(_, f)| f.out.units())
            .chain(self.library.iter().filter_map(Record::as_unit))
            .find(|u| canonical_uid(u) == *uid)
            .cloned()
    }

    /// Where a unit is declared in a document but `except`: the document, and the span of its
    /// record's first line.
    pub(crate) fn declaration(&self, uid: &Uid, except: &str) -> Option<(&str, &Document, Span)> {
        for (uri, f) in self.files.iter().filter(|(u, _)| *u != except) {
            for (r, s) in f.out.records.iter().zip(&f.out.spans) {
                if let (Record::Unit(u), Some(s)) = (r, s) {
                    if canonical_uid(u) == *uid {
                        return Some((
                            uri.as_str(),
                            &f.doc,
                            super::nav::selection(f.doc.text(), *s),
                        ));
                    }
                }
            }
        }
        None
    }

    /// Every reference to a unit in the documents but `except`.
    pub(crate) fn references(&self, uid: &Uid, except: &str) -> Vec<(&str, &Document, Span)> {
        let mut out = Vec::new();
        for (uri, f) in self.files.iter().filter(|(u, _)| *u != except) {
            for (s, u) in &f.out.refs {
                if u == uid {
                    out.push((uri.as_str(), &f.doc, *s));
                }
            }
        }
        out
    }

    /// The schemas the workspace declares or uses, for completing a query.
    pub(crate) fn schemas(&self) -> BTreeSet<String> {
        self.library
            .iter()
            .chain(self.files.values().flat_map(|f| f.out.records.iter()))
            .filter_map(|r| match r {
                Record::Unit(u) => Some(u.schema.to_string()),
                Record::SchemaDecl(d) => Some(d.id.to_string()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::{AnalysisHost, Encoding, LineIndex, Position};
    use super::*;

    const BASE: &str = "@doc smysl/0.1 { id: v/base }\n\n\
        @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ Latency rose.\n";

    fn uid_a() -> Uid {
        let out = smysl_core::surface::parse_surface(BASE).unwrap();
        out.uid_of(&"e/a".parse().unwrap()).unwrap()
    }

    fn open_doc() -> String {
        format!(
            "@doc smysl/0.1 {{ id: v/use }}\n\n\
             @claim c/b {{ status: derived, grounds: [{}] }}\n~ It regressed.\n",
            uid_a().canonical()
        )
    }

    fn hosts() -> (AnalysisHost, AnalysisHost) {
        let text = open_doc();
        let mut bare = AnalysisHost::new(Encoding::Utf16);
        bare.open("file:///w/use.smy", Language::Surface, 1, text.clone());
        let mut with = bare.clone();
        let mut ws = Workspace::new();
        ws.add_text("file:///w/base.smy", BASE.to_string());
        // The open document's own file on disk is in the folder too, and is left out.
        ws.add_text("file:///w/use.smy", text);
        with.set_workspace(ws);
        (bare, with)
    }

    #[test]
    fn a_uid_into_another_document_resolves_with_the_workspace() {
        let (bare, with) = hosts();
        let codes = |h: &AnalysisHost| -> Vec<String> {
            h.diagnostics("file:///w/use.smy")
                .into_iter()
                .map(|d| d.code)
                .collect()
        };
        assert!(
            codes(&bare).contains(&"SMY-E060".to_string()),
            "{:?}",
            codes(&bare)
        );
        assert!(codes(&with).is_empty(), "{:?}", codes(&with));
    }

    #[test]
    fn definition_references_and_hover_cross_documents() {
        let (_, with) = hosts();
        let text = open_doc();
        let at = text.find("b3:").unwrap() + 5;
        let pos = LineIndex::new(&text).position(&text, at, Encoding::Utf16);
        let def = with
            .definition("file:///w/use.smy", pos)
            .expect("declared in base");
        assert_eq!(def.uri, "file:///w/base.smy");
        assert_eq!(def.range.start, Position::new(2, 10));
        let refs = with.references("file:///w/use.smy", pos, true);
        let uris: Vec<&str> = refs.iter().map(|l| l.uri.as_str()).collect();
        assert_eq!(uris, vec!["file:///w/base.smy", "file:///w/use.smy"]);
        let h = with.hover("file:///w/use.smy", pos).unwrap().markdown;
        assert!(h.starts_with("**"), "{h}");
        assert!(h.contains("> Latency rose."), "{h}");
        assert!(h.contains("declared in `file:///w/base.smy`"), "{h}");
    }

    #[test]
    fn a_library_s_records_are_seen_and_placed_nowhere() {
        let out = smysl_core::surface::parse_surface(BASE).unwrap();
        let mut ws = Workspace::new();
        ws.add_records(out.records);
        let mut h = AnalysisHost::new(Encoding::Utf16);
        h.open("file:///w/use.smy", Language::Surface, 1, open_doc());
        h.set_workspace(ws);
        assert!(h.diagnostics("file:///w/use.smy").is_empty());
        let text = open_doc();
        let at = text.find("b3:").unwrap() + 5;
        let pos = LineIndex::new(&text).position(&text, at, Encoding::Utf16);
        assert!(h.definition("file:///w/use.smy", pos).is_none());
        assert!(h
            .hover("file:///w/use.smy", pos)
            .unwrap()
            .markdown
            .contains("from the library"));
    }

    #[test]
    fn a_query_is_planned_against_the_workspace() {
        let mut h = AnalysisHost::new(Encoding::Utf16);
        h.open(
            "file:///w/q.sq",
            Language::Query,
            1,
            "from store:elsewhere\n".into(),
        );
        assert!(
            h.diagnostics("file:///w/q.sq").is_empty(),
            "no workspace: parse only"
        );
        h.set_workspace(Workspace::new());
        let planned = h.diagnostics("file:///w/q.sq");
        assert_eq!(planned.len(), 1, "{planned:?}");
        assert!(planned[0].message.contains("workspace"), "{planned:?}");
        // What needs the library layer is the server's limit, not the query's error.
        h.open(
            "file:///w/l.sq",
            Language::Query,
            1,
            "from library\n".into(),
        );
        assert!(h.diagnostics("file:///w/l.sq").is_empty());
    }
}
