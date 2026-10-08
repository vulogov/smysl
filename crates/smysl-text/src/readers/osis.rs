//! `osis/1` — OSIS XML, the interchange form most digital Bibles are published in.
//!
//! OSIS is the one source whose identifiers are already the vocabulary this crate uses:
//! `osisID="Gen.1.1"` is the canonical locator, so this reader maps nothing and a locator it
//! emits is the one the file states. That is also why [`super::books`] chose OSIS — the reader
//! with no mapping table is the reader with no mapping table to be wrong.
//!
//! # Both verse forms
//!
//! Verses come two ways in the wild: as containers, `<verse osisID="Gen.1.1">…</verse>`, and
//! as milestones, `<verse sID="Gen.1.1"/>…<verse eID="Gen.1.1"/>`. Both are read. A reader
//! that handled only containers would silently produce a text with no verses at all from half
//! the Bibles in circulation — silently, because the file is well-formed XML and the chapters
//! still come out.
//!
//! # What it refuses
//!
//! A document type declaration, and any entity that is not one of XML's five predefined ones
//! or a character reference. This is §3.9.1's rule, and it is the reason this reader cannot be
//! made to expand an entity a thousandfold: there is no DTD to declare one in, and an
//! undeclared entity is a refusal naming it. Malformed XML is a refusal with the byte offset
//! the parser stopped at.

use quick_xml::events::{BytesStart, Event};

use crate::readers::xml;

use crate::limits::Budget;
use crate::locator::Locator;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::{books, Input, Params, ReadOutput};
use crate::reading::Level;
use crate::LibError;

/// The level this reader cuts parts on.
pub const LEVEL: &str = "chapter";

/// Elements whose content is about the text rather than part of it.
///
/// A note is an editor's apparatus and a title is a heading an editor chose; neither is in the
/// verse. Including them would put a cross-reference inside the span a quotation is matched
/// against, which is how a corpus comes to contain text no edition ever printed.
const ASIDES: &[&[u8]] = &[b"note", b"title", b"reference", b"milestone"];

pub struct Osis;

impl crate::readers::Reader for Osis {
    fn id(&self) -> &'static str {
        "osis/1"
    }

    fn read(
        &self,
        input: &Input<'_>,
        _params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError> {
        limits.scan(input.len() as u64)?;
        let source = Normalised::new(input.bytes())?;
        read_xml(source.as_str(), limits)
    }
}

fn unreadable(at: u64, what: impl Into<String>) -> LibError {
    xml::unreadable("osis/1", at, what)
}

/// The value of an attribute, if the element has it.
fn attr(e: &BytesStart<'_>, name: &[u8], at: u64) -> Result<Option<String>, LibError> {
    xml::attr("osis/1", e, name, at)
}

/// The parser's position as a byte offset, for a diagnostic to point at.
fn at_usize(at: u64) -> usize {
    usize::try_from(at).unwrap_or(usize::MAX)
}

fn locator(text: &str, at: u64) -> Result<Locator, LibError> {
    text.parse()
        .map_err(|e| unreadable(at, format!("a canonical locator, not `{text}` ({e})")))
}

fn read_xml(source: &str, limits: &mut Budget) -> Result<ReadOutput, LibError> {
    let mut reader = xml::reader(source);

    let mut doc = Doc::new("osis/1");
    let mut title: Option<String> = None;
    let mut lang: Option<String> = None;
    let mut in_header = false;
    let mut capture_title = false;
    let mut captured = String::new();
    // Depth of the aside being skipped, 0 when text is being kept. A counter and not a flag:
    // asides nest, and a flag would end the skip at the first inner close tag.
    let mut skip: u32 = 0;
    let mut lossy = false;

    loop {
        let at = reader.buffer_position();
        let event = reader
            .read_event()
            .map_err(|e| unreadable(at, format!("well-formed XML ({e})")))?;
        match event {
            Event::Eof => break,
            // §3.9.1: no DTD. Everything an entity attack needs is declared in one.
            Event::DocType(_) => {
                return Err(xml::no_doctype("osis/1", at));
            }
            event @ (Event::Start(_) | Event::Empty(_)) => {
                // Whether the element has content decides whether an aside starts a skip: a
                // self-closing `<title/>` has nothing inside it, and a skip begun there would
                // never be decremented — it would swallow the rest of the document.
                let empty = matches!(event, Event::Empty(_));
                let e = match event {
                    Event::Start(e) | Event::Empty(e) => e,
                    _ => unreachable!("matched above"),
                };
                let name = e.local_name();
                let name = name.as_ref();
                if skip > 0 {
                    if !empty {
                        skip += 1;
                    }
                    continue;
                }
                match name {
                    b"osisText" => {
                        if let Some(value) = attr(&e, b"xml:lang", at)? {
                            lang = Some(value);
                        }
                    }
                    b"header" => in_header = true,
                    b"title" if in_header && title.is_none() => {
                        capture_title = true;
                        captured.clear();
                    }
                    b"div" => {
                        let kind = attr(&e, b"type", at)?;
                        if kind.as_deref() == Some("book") {
                            let id = attr(&e, b"osisID", at)?
                                .ok_or_else(|| unreadable(at, "a book div with an osisID"))?;
                            if !books::is_osis(&id) {
                                return Err(unreadable(
                                    at,
                                    format!("a book in the canonical vocabulary, not `{id}`"),
                                ));
                            }
                            doc.open("book", locator(&id, at)?, at_usize(at), limits)?;
                        }
                    }
                    b"chapter" => {
                        if let Some(id) = attr(&e, b"osisID", at)?.or(attr(&e, b"sID", at)?) {
                            doc.open("chapter", locator(&id, at)?, at_usize(at), limits)?;
                        }
                    }
                    b"verse" => {
                        if let Some(id) = attr(&e, b"eID", at)? {
                            let _ = id;
                            doc.close_level("verse");
                        } else if let Some(id) = attr(&e, b"osisID", at)?.or(attr(&e, b"sID", at)?)
                        {
                            // An osisID may name several verses at once, `Gen.1.1 Gen.1.2`,
                            // which is OSIS's spelling of a bridge. The range it covers is
                            // what addresses the text, as it is for a USFM bridge.
                            let ids: Vec<&str> = id.split_whitespace().collect();
                            let first = ids
                                .first()
                                .ok_or_else(|| unreadable(at, "a verse with an osisID"))?;
                            let loc = match ids.len() {
                                1 => locator(first, at)?,
                                _ => {
                                    let last = ids.last().expect("len > 1");
                                    Locator::range(locator(first, at)?, locator(last, at)?)
                                        .map_err(|e| {
                                            unreadable(at, format!("a verse range ({e})"))
                                        })?
                                }
                            };
                            doc.open("verse", loc, at_usize(at), limits)?;
                        }
                    }
                    _ if ASIDES.contains(&name) => {
                        lossy = true;
                        // Dropped, but still a boundary: `<milestone type="line"/>` is a line
                        // break, and the words either side of it are two words. The gap is
                        // owed rather than written, so it costs nothing when the source
                        // already has whitespace around the aside.
                        doc.gap();
                        if !empty {
                            skip = 1;
                        }
                    }
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = e.local_name();
                let name = name.as_ref();
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                match name {
                    b"header" => in_header = false,
                    b"title" if capture_title => {
                        capture_title = false;
                        let text = captured.trim();
                        if !text.is_empty() {
                            title = Some(text.to_string());
                        }
                    }
                    b"verse" => doc.close_level("verse"),
                    b"chapter" => doc.close_level("chapter"),
                    b"div" => {}
                    _ => {}
                }
            }
            Event::Text(t) => {
                if skip > 0 {
                    continue;
                }
                let text = t
                    .decode()
                    .map_err(|e| unreadable(at, format!("text in the declared encoding ({e})")))?;
                if capture_title {
                    captured.push_str(&text);
                } else if !in_header {
                    doc.write(&text);
                }
            }
            Event::CData(t) => {
                if skip > 0 || in_header {
                    continue;
                }
                let text = t
                    .decode()
                    .map_err(|e| unreadable(at, format!("CDATA in the declared encoding ({e})")))?;
                doc.write(&text);
            }
            Event::GeneralRef(r) => {
                if skip > 0 {
                    continue;
                }
                let name = r
                    .decode()
                    .map_err(|e| unreadable(at, format!("an entity name ({e})")))?;
                let resolved = xml::entity("osis/1", &name, at)?;
                if capture_title {
                    captured.push_str(&resolved);
                } else if !in_header {
                    // Appended through the same path as text, so an entity inside a verse is
                    // inside that verse's range rather than between two of them.
                    doc.write(&resolved);
                }
            }
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
        }
    }

    let (text, rows) = doc.finish()?;
    let mut out = ReadOutput::new(text, rows, Level::new(LEVEL).expect("a level"));
    out.title = title;
    out.lossy = lossy;
    if let Some(tag) = lang {
        // A language tag the source states and this build cannot parse is dropped rather
        // than refused: `xml:lang` is metadata about the whole document, the manifest has
        // `und` for exactly this case, and no locator or span depends on it.
        if let Ok(parsed) = smysl_core::ids::LangTag::new(tag.as_str()) {
            out.lang = Some(parsed);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;
    use crate::readers::{read_with, Reader};
    use crate::structure::Structure;

    /// The container form: verses hold their text.
    const CONTAINED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<osis><osisText osisIDWork="KJV" xml:lang="en">
  <header><work osisWork="KJV"><title>King James Version</title></work></header>
  <div type="book" osisID="Gen">
    <chapter osisID="Gen.1">
      <verse osisID="Gen.1.1">In the beginning God created the heaven and the earth.</verse>
      <verse osisID="Gen.1.2">And the earth was without form.</verse>
    </chapter>
  </div>
</osisText></osis>"#;

    /// The milestone form: verses are points, and the text is between them.
    const MILESTONE: &str = r#"<osis><osisText xml:lang="en">
  <div type="book" osisID="Gen">
    <chapter sID="Gen.1" osisID="Gen.1"/>
    <verse sID="Gen.1.1" osisID="Gen.1.1"/>In the beginning God created the heaven and the earth.<verse eID="Gen.1.1"/>
    <verse sID="Gen.1.2" osisID="Gen.1.2"/>And the earth was without form.<verse eID="Gen.1.2"/>
    <chapter eID="Gen.1"/>
  </div>
</osisText></osis>"#;

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Osis,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    fn refuse(source: &str) -> LibError {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Osis,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect_err("refused")
    }

    fn verse<'a>(out: &'a ReadOutput, id: &str) -> &'a str {
        let row = out
            .rows
            .iter()
            .find(|r| r.locator.to_string() == id)
            .unwrap_or_else(|| panic!("{id} is not in the table"));
        &out.text.as_str()[row.start as usize..row.end as usize]
    }

    /// Both spellings of a verse produce the same text, the same table and the same tid.
    ///
    /// This is the test that matters for this reader. The two forms are different XML with
    /// nothing in common structurally, they are both in wide circulation, and a corpus built
    /// from one must be able to align with a corpus built from the other — which it can only
    /// do if the identity over the text is the same.
    #[test]
    fn the_container_and_milestone_forms_agree_to_the_byte() {
        let contained = read(CONTAINED);
        let milestone = read(MILESTONE);
        assert_eq!(contained.text.as_str(), milestone.text.as_str());
        assert_eq!(contained.text.tid(), milestone.text.tid());
        let ids = |out: &ReadOutput| -> Vec<String> {
            out.rows.iter().map(|r| r.locator.to_string()).collect()
        };
        assert_eq!(ids(&contained), ids(&milestone));
        assert_eq!(ids(&contained), vec!["Gen", "Gen.1", "Gen.1.1", "Gen.1.2"]);
    }

    #[test]
    fn each_verse_holds_its_own_text_without_the_indentation() {
        let out = read(CONTAINED);
        assert_eq!(
            verse(&out, "Gen.1.1"),
            "In the beginning God created the heaven and the earth."
        );
        assert_eq!(verse(&out, "Gen.1.2"), "And the earth was without form.");
        assert!(
            !out.text.as_str().contains('\n'),
            "pretty-printing is not text: {:?}",
            out.text.as_str()
        );
    }

    /// The header's title is metadata, and the header's text is not in the text.
    #[test]
    fn the_work_title_is_metadata() {
        let out = read(CONTAINED);
        assert_eq!(out.title.as_deref(), Some("King James Version"));
        assert!(!out.text.as_str().contains("King James"));
        assert_eq!(out.lang.as_ref().map(|l| l.as_str()), Some("en"));
    }

    /// A note is an editor's apparatus, and none of it reaches the verse.
    #[test]
    fn a_note_is_dropped_entirely_and_recorded_as_a_loss() {
        let out = read(
            r#"<osis><osisText><div type="book" osisID="Gen"><chapter osisID="Gen.1">
            <verse osisID="Gen.1.1">In the beginning<note type="study">See <reference osisRef="Ps.1">Ps 1</reference>.</note> God created.</verse>
            </chapter></div></osisText></osis>"#,
        );
        assert_eq!(verse(&out, "Gen.1.1"), "In the beginning God created.");
        assert!(out.lossy, "a note was dropped");
    }

    /// A self-closing aside does not swallow the rest of the document.
    ///
    /// The skip counter is incremented on a start tag and decremented on an end tag, and a
    /// `<milestone/>` has neither — so a reader that started a skip there would produce a
    /// document that simply stops, with no error, at the first one. Most OSIS Bibles carry
    /// milestones every few verses.
    #[test]
    fn a_self_closing_aside_does_not_swallow_the_document() {
        let out = read(
            r#"<osis><osisText><div type="book" osisID="Gen"><chapter osisID="Gen.1">
            <verse osisID="Gen.1.1">Before<milestone type="line"/>after.</verse>
            <verse osisID="Gen.1.2">Still here.</verse>
            </chapter></div></osisText></osis>"#,
        );
        assert_eq!(verse(&out, "Gen.1.1"), "Before after.");
        assert_eq!(verse(&out, "Gen.1.2"), "Still here.");
    }

    /// Predefined entities and character references are text; anything else is a refusal.
    #[test]
    fn predefined_entities_join_the_word_they_are_inside() {
        let out = read(
            r#"<osis><osisText><div type="book" osisID="Gen"><chapter osisID="Gen.1">
            <verse osisID="Gen.1.1">the Spirit&apos;s breath &amp; the deep&#8212;dark</verse>
            </chapter></div></osisText></osis>"#,
        );
        assert_eq!(
            verse(&out, "Gen.1.1"),
            "the Spirit's breath & the deep\u{2014}dark"
        );
    }

    /// §3.9.1: no DTD, and therefore no entity expansion to be made exponential.
    #[test]
    fn a_document_type_declaration_is_refused() {
        let err = refuse(
            r#"<?xml version="1.0"?><!DOCTYPE osis [<!ENTITY lol "ha">]><osis><osisText/></osis>"#,
        );
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("document type declaration"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// An entity no DTD could have declared here is refused by name, not dropped.
    ///
    /// Dropping it would remove text from a verse and leave a corpus that reads plausibly and
    /// is missing a word — the failure mode this whole reader is arranged to avoid.
    #[test]
    fn an_undeclared_entity_is_refused_by_name() {
        let err = refuse(
            r#"<osis><osisText><div type="book" osisID="Gen"><chapter osisID="Gen.1">
            <verse osisID="Gen.1.1">a &lol; b</verse></chapter></div></osisText></osis>"#,
        );
        match err {
            LibError::Unreadable { what, .. } => assert!(what.contains("&lol;"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn malformed_xml_is_refused_with_an_offset() {
        let err = refuse(r#"<osis><osisText><div type="book" osisID="Gen"></osisText></osis>"#);
        match err {
            LibError::Unreadable { what, at, .. } => {
                assert!(what.contains("well-formed XML"), "{what}");
                assert!(at > 0, "the offset should point into the document");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A book name outside the canonical vocabulary is refused rather than carried.
    #[test]
    fn a_book_outside_the_vocabulary_is_refused() {
        let err = refuse(r#"<osis><osisText><div type="book" osisID="Tob"/></osisText></osis>"#);
        match err {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("canonical vocabulary"), "{what}")
            }
            other => panic!("{other:?}"),
        }
    }

    /// An OSIS verse that names two verses at once is a bridge, as in USFM.
    #[test]
    fn an_osis_id_naming_two_verses_is_a_range() {
        let out = read(
            r#"<osis><osisText><div type="book" osisID="Exod"><chapter osisID="Exod.20">
            <verse osisID="Exod.20.1 Exod.20.2">And God spake all these words.</verse>
            </chapter></div></osisText></osis>"#,
        );
        let ids: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(ids, vec!["Exod", "Exod.20", "Exod.20.1-Exod.20.2"]);
    }

    /// The rows are a structure, and every locator in it resolves to its own text.
    #[test]
    fn the_table_is_a_structure_that_resolves_what_it_emitted() {
        let out = read(CONTAINED);
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        for row in &out.rows {
            assert_eq!(structure.resolve(&row.locator), Some(row.range()));
        }
        assert_eq!(structure.roots().len(), 1);
    }

    #[test]
    fn the_reader_answers_to_its_id() {
        assert_eq!(Osis.id(), "osis/1");
        assert!(Osis.params().is_empty());
    }
}
