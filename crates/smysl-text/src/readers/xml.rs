//! What the two XML readers must do identically.
//!
//! `osis/1` and `zefania/1` walk different vocabularies, so their element handling is their
//! own. What is here is the part where disagreement would be a defect rather than a
//! difference: how an entity is resolved, what happens to a document type declaration, and
//! how an attribute's value is read. Those are §3.9.1's rules, and a reader that implemented
//! them its own way would be a second chance to get entity expansion wrong.

use quick_xml::events::BytesStart;
use quick_xml::{Reader, XmlVersion};

use crate::LibError;

/// A parser configured the way both readers need it.
pub(crate) fn reader(source: &str) -> Reader<&[u8]> {
    let mut reader = Reader::from_str(source);
    // Whitespace is kept: it is what separates two elements' text, and `Doc::write` is what
    // decides whether a run of it is a separator or nothing.
    reader.config_mut().trim_text(false);
    // A document whose end tags do not match its start tags is malformed, and this reader
    // says so rather than guessing which tag was meant.
    reader.config_mut().check_end_names = true;
    reader
}

pub(crate) fn unreadable(reader: &str, at: u64, what: impl Into<String>) -> LibError {
    LibError::Unreadable {
        reader: reader.to_string(),
        at: usize::try_from(at).unwrap_or(usize::MAX),
        what: what.into(),
    }
}

/// An attribute's value, normalised as the XML specification requires.
///
/// Normalisation resolves the five predefined entities and character references, and nothing
/// else — the same rule [`entity`] applies to character data.
pub(crate) fn attr(
    reader: &str,
    e: &BytesStart<'_>,
    name: &[u8],
    at: u64,
) -> Result<Option<String>, LibError> {
    match e
        .try_get_attribute(name)
        .map_err(|err| unreadable(reader, at, format!("well-formed attributes ({err})")))?
    {
        Some(a) => Ok(Some(
            a.normalized_value(XmlVersion::Implicit1_0)
                .map_err(|err| unreadable(reader, at, format!("an attribute value ({err})")))?
                .into_owned(),
        )),
        None => Ok(None),
    }
}

/// Resolve `&name;` the way a parser with no DTD must: XML's five predefined entities and
/// character references, and nothing else.
///
/// An entity outside that set could only have been declared in a document type declaration,
/// which these readers refuse — so it is refused here too, **by name**. Dropping it instead
/// would silently remove text from a verse, which is the one outcome worse than a refusal:
/// the corpus still reads plausibly and is missing a word.
pub(crate) fn entity(reader: &str, name: &str, at: u64) -> Result<String, LibError> {
    let resolved = match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        other => {
            let code = match other.strip_prefix('#') {
                Some(hex) if hex.starts_with('x') || hex.starts_with('X') => {
                    u32::from_str_radix(&hex[1..], 16).ok()
                }
                Some(dec) => dec.parse::<u32>().ok(),
                None => None,
            };
            match code.and_then(char::from_u32) {
                Some(c) => c,
                None => {
                    return Err(unreadable(
                        reader,
                        at,
                        format!("a predefined entity or a character reference, not `&{other};`"),
                    ))
                }
            }
        }
    };
    Ok(resolved.to_string())
}

/// The refusal for a document type declaration, so both readers word it the same way.
pub(crate) fn no_doctype(reader: &str, at: u64) -> LibError {
    unreadable(reader, at, "no document type declaration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_five_predefined_entities_resolve() {
        for (name, expected) in [
            ("amp", "&"),
            ("lt", "<"),
            ("gt", ">"),
            ("quot", "\""),
            ("apos", "'"),
        ] {
            assert_eq!(entity("t", name, 0).expect(name), expected);
        }
    }

    #[test]
    fn character_references_resolve_in_both_bases() {
        assert_eq!(entity("t", "#8212", 0).expect("decimal"), "\u{2014}");
        assert_eq!(entity("t", "#x2014", 0).expect("hex"), "\u{2014}");
        assert_eq!(entity("t", "#X2014", 0).expect("hex upper"), "\u{2014}");
    }

    /// Everything else is refused, including the shapes an attack uses.
    #[test]
    fn nothing_else_resolves() {
        for name in ["lol", "xxe", "#", "#x", "#xZZ", "#99999999", "#xD800", ""] {
            let err = entity("t", name, 7).expect_err(name);
            match err {
                LibError::Unreadable { at, what, .. } => {
                    assert_eq!(at, 7);
                    assert!(what.contains(name), "{name}: {what}");
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// A surrogate is a valid number and not a character; `char::from_u32` is what refuses it.
    #[test]
    fn a_surrogate_code_point_is_not_a_character() {
        assert!(entity("t", "#55296", 0).is_err(), "U+D800 in decimal");
    }
}
