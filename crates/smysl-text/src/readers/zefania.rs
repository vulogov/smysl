//! `zefania/1` — Zefania XML, the format most free Bible modules are distributed in.
//!
//! Zefania is OSIS's opposite in one respect that matters: it names books by **number**,
//! `<BIBLEBOOK bnumber="2">`, so the reader has to know that book 2 is Exodus. That mapping is
//! [`super::books`]'s, which is also why it is one table — a second list of 66 book names is a
//! second list to disagree with the first.
//!
//! The rest is the same shape as [`super::osis`]: three levels, text in the innermost, notes
//! dropped, and the XML rules of §3.9.1 enforced by the crate-private `xml` module rather than
//! restated here.

use quick_xml::events::Event;

use crate::limits::Budget;
use crate::norm::Normalised;
use crate::readers::build::Doc;
use crate::readers::xml;
use crate::readers::{books, Input, Params, ReadOutput};
use crate::reading::Level;
use crate::LibError;

const ID: &str = "zefania/1";

/// The level this reader cuts parts on.
pub const LEVEL: &str = "chapter";

/// Elements whose content is apparatus rather than scripture.
const ASIDES: &[&[u8]] = &[b"NOTE", b"XREF", b"CAPTION", b"PROLOG", b"REMARK", b"BR"];

pub struct Zefania;

impl crate::readers::Reader for Zefania {
    fn id(&self) -> &'static str {
        ID
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

/// The parser's position as a byte offset, for a diagnostic to point at.
fn at_usize(at: u64) -> usize {
    usize::try_from(at).unwrap_or(usize::MAX)
}

/// A number attribute, which Zefania uses for every identifier it has.
fn number(value: &str, at: u64, what: &str) -> Result<u64, LibError> {
    value
        .trim()
        .parse()
        .map_err(|_| xml::unreadable(ID, at, format!("{what} as a number, not `{value}`")))
}

fn read_xml(source: &str, limits: &mut Budget) -> Result<ReadOutput, LibError> {
    let mut reader = xml::reader(source);

    let mut doc = Doc::new("zefania/1");
    let mut book: Option<&'static str> = None;
    let mut chapter: Option<u64> = None;
    let mut title: Option<String> = None;
    let mut lang: Option<String> = None;
    let mut capture: Option<&'static str> = None;
    let mut captured = String::new();
    let mut in_information = false;
    let mut skip: u32 = 0;
    let mut lossy = false;

    loop {
        let at = reader.buffer_position();
        let event = reader
            .read_event()
            .map_err(|e| xml::unreadable(ID, at, format!("well-formed XML ({e})")))?;
        match event {
            Event::Eof => break,
            Event::DocType(_) => return Err(xml::no_doctype(ID, at)),
            event @ (Event::Start(_) | Event::Empty(_)) => {
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
                    b"XMLBIBLE" => {
                        // `biblename` is the short label on the root element; the title in
                        // `<INFORMATION>` is the document's own metadata block and is the
                        // fuller statement of the same thing ("Luther 1912" against "Luther
                        // Bibel 1912"). So this is the fallback, and the information block
                        // overwrites it when there is one — a module with only the attribute
                        // still gets a title rather than none.
                        if let Some(value) = xml::attr(ID, &e, b"biblename", at)? {
                            if title.is_none() && !value.trim().is_empty() {
                                title = Some(value.trim().to_string());
                            }
                        }
                    }
                    b"INFORMATION" => in_information = true,
                    b"title" | b"TITLE" if in_information => {
                        capture = Some("title");
                        captured.clear();
                    }
                    b"language" | b"LANGUAGE" if in_information => {
                        capture = Some("language");
                        captured.clear();
                    }
                    b"BIBLEBOOK" => {
                        let value = xml::attr(ID, &e, b"bnumber", at)?
                            .ok_or_else(|| xml::unreadable(ID, at, "a BIBLEBOOK with a bnumber"))?;
                        let n = number(&value, at, "a book number")?;
                        let osis = books::from_number(n).ok_or_else(|| {
                            xml::unreadable(
                                ID,
                                at,
                                format!("a book number within the canon, not `{n}`"),
                            )
                        })?;
                        book = Some(osis);
                        chapter = None;
                        doc.open(
                            "book",
                            osis.parse().expect("a book name is a locator"),
                            at_usize(at),
                            limits,
                        )?;
                    }
                    b"CHAPTER" => {
                        let osis = book.ok_or_else(|| {
                            xml::unreadable(ID, at, "a BIBLEBOOK before the first CHAPTER")
                        })?;
                        let value = xml::attr(ID, &e, b"cnumber", at)?
                            .ok_or_else(|| xml::unreadable(ID, at, "a CHAPTER with a cnumber"))?;
                        let n = number(&value, at, "a chapter number")?;
                        chapter = Some(n);
                        let text = format!("{osis}.{n}");
                        doc.open(
                            "chapter",
                            text.parse()
                                .map_err(|e| xml::unreadable(ID, at, format!("a locator ({e})")))?,
                            at_usize(at),
                            limits,
                        )?;
                    }
                    b"VERS" => {
                        let osis = book.ok_or_else(|| {
                            xml::unreadable(ID, at, "a BIBLEBOOK before the first VERS")
                        })?;
                        let c = chapter.ok_or_else(|| {
                            xml::unreadable(ID, at, "a CHAPTER before the first VERS")
                        })?;
                        let value = xml::attr(ID, &e, b"vnumber", at)?
                            .ok_or_else(|| xml::unreadable(ID, at, "a VERS with a vnumber"))?;
                        let n = number(&value, at, "a verse number")?;
                        let text = format!("{osis}.{c}.{n}");
                        doc.open(
                            "verse",
                            text.parse()
                                .map_err(|e| xml::unreadable(ID, at, format!("a locator ({e})")))?,
                            at_usize(at),
                            limits,
                        )?;
                    }
                    _ if ASIDES.contains(&name) => {
                        lossy = true;
                        // `<BR/>` is a line break and a note sits between words: dropped, and
                        // still a boundary. See `osis.rs` for the same decision.
                        doc.gap();
                        if !empty {
                            skip = 1;
                        }
                    }
                    // `<STYLE>` and `<GRAM>` wrap words that are part of the verse. The
                    // markup goes, the words stay — which is a loss, and recorded as one.
                    b"STYLE" | b"GRAM" | b"gr" | b"DIV" | b"SUP" => lossy = true,
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
                    b"INFORMATION" => in_information = false,
                    b"title" | b"TITLE" | b"language" | b"LANGUAGE" => {
                        let text = captured.trim().to_string();
                        match capture.take() {
                            Some("title") if !text.is_empty() => title = Some(text),
                            Some("language") if !text.is_empty() => lang = Some(text),
                            _ => {}
                        }
                    }
                    b"VERS" => doc.close_level("verse"),
                    b"CHAPTER" => doc.close_level("chapter"),
                    b"BIBLEBOOK" => doc.close_level("book"),
                    _ => {}
                }
            }
            Event::Text(t) => {
                if skip > 0 {
                    continue;
                }
                let text = t.decode().map_err(|e| {
                    xml::unreadable(ID, at, format!("text in the declared encoding ({e})"))
                })?;
                match capture {
                    Some(_) => captured.push_str(&text),
                    None if !in_information => doc.write(&text),
                    None => {}
                }
            }
            Event::CData(t) => {
                if skip > 0 {
                    continue;
                }
                let text = t.decode().map_err(|e| {
                    xml::unreadable(ID, at, format!("CDATA in the declared encoding ({e})"))
                })?;
                match capture {
                    Some(_) => captured.push_str(&text),
                    None if !in_information => doc.write(&text),
                    None => {}
                }
            }
            Event::GeneralRef(r) => {
                if skip > 0 {
                    continue;
                }
                let name = r
                    .decode()
                    .map_err(|e| xml::unreadable(ID, at, format!("an entity name ({e})")))?;
                let resolved = xml::entity(ID, &name, at)?;
                match capture {
                    Some(_) => captured.push_str(&resolved),
                    None if !in_information => doc.write(&resolved),
                    None => {}
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
        // Zefania writes ISO 639-2 three-letter codes (`GER`, `ENG`), lowercased here because
        // BCP-47 is case-insensitive and this format's tags are compared as bytes. A code this
        // build cannot parse is dropped, not refused: it is metadata about the document, and
        // the manifest has `und` for the case where nobody knows.
        if let Ok(parsed) = smysl_core::ids::LangTag::new(tag.to_lowercase()) {
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

    const LUTHER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<XMLBIBLE biblename="Luther 1912" type="x-bible">
  <INFORMATION><title>Luther Bibel 1912</title><language>GER</language></INFORMATION>
  <BIBLEBOOK bnumber="1" bname="Genesis">
    <CHAPTER cnumber="1">
      <VERS vnumber="1">Am Anfang schuf Gott Himmel und Erde.</VERS>
      <VERS vnumber="2">Und die Erde war w&#252;st und leer.</VERS>
    </CHAPTER>
  </BIBLEBOOK>
</XMLBIBLE>"#;

    fn read(source: &str) -> ReadOutput {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Zefania,
            &Input::new(source.as_bytes()),
            &Params::new(),
            &mut budget,
        )
        .expect("read")
    }

    fn refuse(source: &str) -> LibError {
        let mut budget = Budget::new(Caps::DEFAULT, source.len() as u64).expect("budget");
        read_with(
            &Zefania,
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

    /// A book number becomes the canonical book name, which is the whole point of the mapping.
    #[test]
    fn book_numbers_become_canonical_names() {
        let out = read(LUTHER);
        let ids: Vec<String> = out.rows.iter().map(|r| r.locator.to_string()).collect();
        assert_eq!(ids, vec!["Gen", "Gen.1", "Gen.1.1", "Gen.1.2"]);
    }

    #[test]
    fn each_verse_holds_its_own_text() {
        let out = read(LUTHER);
        assert_eq!(
            verse(&out, "Gen.1.1"),
            "Am Anfang schuf Gott Himmel und Erde."
        );
        assert_eq!(verse(&out, "Gen.1.2"), "Und die Erde war wüst und leer.");
    }

    #[test]
    fn the_information_block_is_metadata_and_not_text() {
        let out = read(LUTHER);
        // The information block's title, not the root attribute's shorter label.
        assert_eq!(out.title.as_deref(), Some("Luther Bibel 1912"));
        assert_eq!(out.lang.as_ref().map(|l| l.as_str()), Some("ger"));
        assert!(
            !out.text.as_str().contains("Luther"),
            "{}",
            out.text.as_str()
        );
    }

    /// A `<STYLE>` run keeps its words; a `<NOTE>` keeps none of them.
    /// A module with no information block still gets the root element's label.
    #[test]
    fn the_root_label_is_the_fallback_title() {
        let out = read(
            r#"<XMLBIBLE biblename="Luther 1912"><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1">
            <VERS vnumber="1">Am Anfang.</VERS></CHAPTER></BIBLEBOOK></XMLBIBLE>"#,
        );
        assert_eq!(out.title.as_deref(), Some("Luther 1912"));
    }

    #[test]
    fn style_keeps_words_and_notes_do_not() {
        let out = read(
            r#"<XMLBIBLE><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1">
            <VERS vnumber="1">Am <STYLE css="it">Anfang</STYLE> schuf<NOTE type="x-studynote">eine Anmerkung</NOTE> Gott.</VERS>
            </CHAPTER></BIBLEBOOK></XMLBIBLE>"#,
        );
        assert_eq!(verse(&out, "Gen.1.1"), "Am Anfang schuf Gott.");
        assert!(out.lossy);
    }

    /// `<BR/>` is dropped and still separates the words around it.
    #[test]
    fn a_line_break_separates_without_being_text() {
        let out = read(
            r#"<XMLBIBLE><BIBLEBOOK bnumber="19"><CHAPTER cnumber="1">
            <VERS vnumber="1">Wohl dem<BR art="x-nl"/>der nicht wandelt.</VERS>
            </CHAPTER></BIBLEBOOK></XMLBIBLE>"#,
        );
        assert_eq!(verse(&out, "Ps.1.1"), "Wohl dem der nicht wandelt.");
    }

    /// The numbers have to be numbers, and within the canon.
    #[test]
    fn numbers_outside_the_canon_or_not_numbers_are_refused() {
        for (source, expected) in [
            (
                r#"<XMLBIBLE><BIBLEBOOK bnumber="67"><CHAPTER cnumber="1"/></BIBLEBOOK></XMLBIBLE>"#,
                "within the canon",
            ),
            (
                r#"<XMLBIBLE><BIBLEBOOK bnumber="Gen"><CHAPTER cnumber="1"/></BIBLEBOOK></XMLBIBLE>"#,
                "as a number",
            ),
            (
                r#"<XMLBIBLE><BIBLEBOOK bnumber="1"><CHAPTER cnumber="one"/></BIBLEBOOK></XMLBIBLE>"#,
                "as a number",
            ),
            (
                r#"<XMLBIBLE><CHAPTER cnumber="1"/></XMLBIBLE>"#,
                "BIBLEBOOK before the first CHAPTER",
            ),
            (
                r#"<XMLBIBLE><BIBLEBOOK bnumber="1"><VERS vnumber="1">x</VERS></BIBLEBOOK></XMLBIBLE>"#,
                "CHAPTER before the first VERS",
            ),
        ] {
            match refuse(source) {
                LibError::Unreadable { what, .. } => {
                    assert!(what.contains(expected), "{source}: {what}")
                }
                other => panic!("{source}: {other:?}"),
            }
        }
    }

    /// A source that numbers two verses the same is refused, naming the address.
    ///
    /// Found by fuzzing, from a mutation of this crate's own Luther fixture: two `<VERS>`
    /// elements with `vnumber="1"`. Two rows with one locator is a table `Structure::build`
    /// refuses, so the reader was producing a reading that nothing downstream could open —
    /// and the refusal has to come from the reader, which is the only layer that can say
    /// *where* the second one is. Merging them would be this crate deciding which of two
    /// verses the corpus holds.
    #[test]
    fn a_duplicated_verse_number_is_refused() {
        let err = refuse(
            r#"<XMLBIBLE><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1">
            <VERS vnumber="1">Am Anfang.</VERS><VERS vnumber="1">Und die Erde.</VERS>
            </CHAPTER></BIBLEBOOK></XMLBIBLE>"#,
        );
        match err {
            LibError::Unreadable { what, at, .. } => {
                assert!(what.contains("Gen.1.1"), "{what}");
                assert!(what.contains("stated twice"), "{what}");
                assert!(at > 0, "the offset should point at the second one");
            }
            other => panic!("{other:?}"),
        }
    }

    /// The same XML rules as `osis/1`, because they are the same code.
    #[test]
    fn the_xml_rules_hold_here_too() {
        let doctype = refuse(
            r#"<!DOCTYPE XMLBIBLE [<!ENTITY lol "ha">]><XMLBIBLE><BIBLEBOOK bnumber="1"/></XMLBIBLE>"#,
        );
        match doctype {
            LibError::Unreadable { what, .. } => {
                assert!(what.contains("document type declaration"), "{what}")
            }
            other => panic!("{other:?}"),
        }
        let unknown = refuse(
            r#"<XMLBIBLE><BIBLEBOOK bnumber="1"><CHAPTER cnumber="1"><VERS vnumber="1">a &lol; b</VERS></CHAPTER></BIBLEBOOK></XMLBIBLE>"#,
        );
        match unknown {
            LibError::Unreadable { what, .. } => assert!(what.contains("&lol;"), "{what}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_table_is_a_structure_that_resolves_what_it_emitted() {
        let out = read(LUTHER);
        let mut budget = Budget::new(Caps::DEFAULT, 1 << 16).expect("budget");
        let structure =
            Structure::build(&out.rows, out.text.len() as u64, &mut budget).expect("a valid table");
        for row in &out.rows {
            assert_eq!(structure.resolve(&row.locator), Some(row.range()));
        }
    }

    #[test]
    fn the_reader_answers_to_its_id() {
        assert_eq!(Zefania.id(), "zefania/1");
        assert!(Zefania.params().is_empty());
    }
}
