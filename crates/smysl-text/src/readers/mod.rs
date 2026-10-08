//! The reader contract: bytes of some file format in, normalised text and a segment table out.
//!
//! A reader is the only part of this crate that knows what a file format looks like, and the
//! only part whose output a corpus is named after — a tid is over the text a reader produced,
//! so two readers that disagree about the same bytes produce two libraries. That is why
//! [`Reader::read`] is a function of its arguments and nothing else: no clock, no environment,
//! no filesystem, and no name for the input (see [`Input`]).
//!
//! # What the RFC left to this step
//!
//! SMYSL-2.4 §3.2 gives the trait signature and §4.1 lists `Input`, `Params` and `ReadOutput`
//! in the module table. Neither says what any of them holds, so the shapes here are this
//! step's, written down in the RFC the way step 2 wrote down the locator grammar.
//!
//! # Parameters are part of what produced a corpus
//!
//! A parameter changes a reader's output, so a parameter that is not recorded is a corpus that
//! means something else on re-read — the defect SMYSL-2.4 already fixed once by making the
//! part policy a required manifest key. The remedy needs no new key: a reader field spells an
//! id and its settings exactly as `Policy::id` does, `"whatsapp/1 date-format=dmy"`, which is
//! what [`reader_field`] renders and [`parse_reader_field`] reads back.
//!
//! None of the six TX-P1 readers takes a parameter, so this build cannot write such a field —
//! and `no_reader_in_this_phase_takes_a_parameter` asserts that, because the format spec still
//! describes manifest key 3 as an id alone. The release that adds a parameterised reader is the release that widens the spec;
//! the test is what makes those two things happen in the same commit.

use std::collections::BTreeMap;

use smysl_core::ids::LangTag;

use crate::limits::Budget;
use crate::norm::Normalised;
use crate::reading::{Level, Segment};
use crate::LibError;

/// Bytes to read, and nothing else.
///
/// There is no name and no path here on purpose. A reader whose output depended on what the
/// file was called would name the same text two things in two libraries, and a reader that
/// could open a path would be a reader the purity gate cannot describe. An archive reader
/// (TX-P2) takes its entry names from the archive bytes, which are in `bytes` like everything
/// else.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    bytes: &'a [u8],
}

impl<'a> Input<'a> {
    pub fn new(bytes: &'a [u8]) -> Input<'a> {
        Input { bytes }
    }

    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

/// One parameter a reader accepts, declared by the reader so that validation happens once.
///
/// Declared rather than parsed inside each reader: six readers validating their own parameters
/// are six places for a typo to be ignored, and an ignored parameter is the silence this
/// module's header is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    pub key: &'static str,
    pub required: bool,
    /// What the value means, in the reader's own words. This is what a refusal prints.
    pub what: &'static str,
}

/// The parameters a caller passed, as given.
///
/// Values are text because that is what a command line and a manifest both carry; a reader
/// parses its own. The grammar is narrow — no whitespace, no `=` — because these render into
/// one space-separated field beside the reader id, and a value that could not be read back
/// would be a setting recorded in a form nothing can parse.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Params {
    values: BTreeMap<String, String>,
}

impl Params {
    pub fn new() -> Params {
        Params::default()
    }

    /// Add a parameter, refusing a key or a value that could not survive the round trip.
    pub fn set(
        &mut self,
        reader: &str,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Result<(), LibError> {
        let key = key.into();
        let value = value.into();
        let bad = |reason: &str| LibError::BadParam {
            reader: reader.to_string(),
            key: key.clone(),
            reason: reason.to_string(),
        };
        if !is_param_key(&key) {
            return Err(bad("a key is lowercase ASCII letters, digits and `-`"));
        }
        if value.is_empty() {
            return Err(bad("a value cannot be empty"));
        }
        if value.chars().any(|c| c.is_whitespace() || c == '=') {
            return Err(bad("a value holds no whitespace and no `=`"));
        }
        self.values.insert(key, value);
        Ok(())
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Every parameter the caller gave is one this reader has, and every required one is there.
    ///
    /// Called by [`read_with`] before the reader runs, so no reader can forget it.
    pub fn check(&self, reader: &str, specs: &[ParamSpec]) -> Result<(), LibError> {
        for key in self.values.keys() {
            if !specs.iter().any(|s| s.key == key) {
                let known = if specs.is_empty() {
                    "this reader takes no parameters".to_string()
                } else {
                    let names: Vec<&str> = specs.iter().map(|s| s.key).collect();
                    format!("this reader takes {}", names.join(", "))
                };
                return Err(LibError::BadParam {
                    reader: reader.to_string(),
                    key: key.clone(),
                    reason: known,
                });
            }
        }
        for spec in specs.iter().filter(|s| s.required) {
            if !self.values.contains_key(spec.key) {
                return Err(LibError::BadParam {
                    reader: reader.to_string(),
                    key: spec.key.to_string(),
                    reason: format!("required: {}", spec.what),
                });
            }
        }
        Ok(())
    }
}

fn is_param_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The manifest's reader field (key 3): the id, then each parameter, in key order.
///
/// `"txt/1"` with no parameters, `"whatsapp/1 date-format=dmy"` with one. Sorted, because
/// `Params` is a `BTreeMap` and a field whose spelling depended on insertion order would make
/// two identical manifests two different mids.
pub fn reader_field(id: &str, params: &Params) -> String {
    let mut out = String::from(id);
    for (k, v) in &params.values {
        out.push(' ');
        out.push_str(k);
        out.push('=');
        out.push_str(v);
    }
    out
}

/// Read a manifest's reader field back into an id and its parameters.
///
/// `None` when the field is not one this code could have written — which is a manifest from a
/// newer build, and the caller's business rather than this function's.
pub fn parse_reader_field(field: &str) -> Option<(String, Params)> {
    let mut parts = field.split(' ');
    let id = parts.next()?.to_string();
    if id.is_empty() {
        return None;
    }
    let mut params = Params::new();
    for token in parts {
        let (k, v) = token.split_once('=')?;
        params.set(&id, k, v).ok()?;
    }
    Some((id, params))
}

/// What a reader found: the text a tid will be taken over, and the table over that text.
///
/// `rows` are in document order and satisfy what [`crate::structure::Structure::build`]
/// requires — siblings disjoint, children inside parents, no two rows with one locator. The
/// metadata fields are what the reader read out of the source's own header, for a
/// [`crate::ManifestBuilder`] to carry; a reader that found none leaves them empty rather than
/// inventing them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReadOutput {
    pub text: Normalised,
    pub rows: Vec<Segment>,
    /// The level a part boundary falls on by default for this reader.
    ///
    /// Here because a policy whose boundary level names no row produces **no parts**, and a
    /// text that silently becomes nothing is worse than a refusal. `txt/1` has no chapters,
    /// so the part policy's default level cannot be the only answer to this question.
    pub top_level: Level,
    pub lang: Option<LangTag>,
    pub title: Option<String>,
    pub creators: Vec<String>,
    /// EDTF as the source printed it, unvalidated at this layer (TX-P3 parses it).
    pub published: Option<String>,
    pub identifiers: BTreeMap<String, String>,
    /// The reader's manifest-level metadata it could not place in a named field, as canonical
    /// CBOR (manifest key 16).
    pub raw: Option<Vec<u8>>,
    /// The reader dropped something the source carried — formatting, markup, an attachment.
    pub lossy: bool,
}

impl ReadOutput {
    /// The minimum a reader must produce: text, a table, and the level parts are cut on.
    pub fn new(text: Normalised, rows: Vec<Segment>, top_level: Level) -> ReadOutput {
        ReadOutput {
            text,
            rows,
            top_level,
            lang: None,
            title: None,
            creators: Vec::new(),
            published: None,
            identifiers: BTreeMap::new(),
            raw: None,
            lossy: false,
        }
    }

    pub fn with_title(mut self, title: impl Into<String>) -> ReadOutput {
        self.title = Some(title.into());
        self
    }

    pub fn with_lang(mut self, lang: LangTag) -> ReadOutput {
        self.lang = Some(lang);
        self
    }

    pub fn lossy(mut self) -> ReadOutput {
        self.lossy = true;
        self
    }
}

/// One file format, read under a budget.
pub trait Reader {
    /// `name/version`, as [`crate::READERS`] lists it and a manifest records it.
    fn id(&self) -> &'static str;

    /// The parameters this reader accepts. Empty for every TX-P1 reader.
    fn params(&self) -> &'static [ParamSpec] {
        &[]
    }

    /// Read the input. Charges the budget for what it scans and what it builds.
    ///
    /// Call [`read_with`] rather than this directly: it validates the parameters first, which
    /// is a check no reader should be able to skip.
    fn read(
        &self,
        input: &Input<'_>,
        params: &Params,
        limits: &mut Budget,
    ) -> Result<ReadOutput, LibError>;
}

/// Validate the parameters, then read.
pub fn read_with(
    reader: &dyn Reader,
    input: &Input<'_>,
    params: &Params,
    limits: &mut Budget,
) -> Result<ReadOutput, LibError> {
    params.check(reader.id(), reader.params())?;
    reader.read(input, params, limits)
}

/// Which feature builds which reader, for the message a caller gets when it is not built.
///
/// A table rather than a `cfg!` chain so that the pairing is testable: every id in
/// [`crate::READERS`] has a row, and every row names a feature this crate declares.
pub const FEATURES: &[(&str, &str)] = &[
    ("txt/1", "reader-txt"),
    ("md/1", "reader-md"),
    ("usfm/1", "reader-usfm"),
    ("osis/1", "reader-osis"),
    ("zefania/1", "reader-zefania"),
    ("json/1", "reader-json"),
];

/// Why a reader id did not produce a reader.
///
/// Two cases and not one, because "this build does not have it" and "no such reader exists"
/// call for different actions from whoever asked: enable a feature, or fix the id. Collapsing
/// them would make a missing feature look like a typo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoReader {
    /// The id is real, this binary was built without it, and this is the feature that adds it.
    NotBuilt {
        id: &'static str,
        feature: &'static str,
    },
    /// No reader of this crate answers to that id.
    NoSuchReader,
}

impl std::fmt::Display for NoReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NoReader::NotBuilt { id, feature } => {
                write!(
                    f,
                    "reader `{id}` is not in this build (feature `{feature}`)"
                )
            }
            NoReader::NoSuchReader => write!(f, "no such reader"),
        }
    }
}

impl std::error::Error for NoReader {}

/// The reader with this id, if this build has it.
pub fn reader(id: &str) -> Result<&'static dyn Reader, NoReader> {
    if let Some(r) = built(id) {
        return Ok(r);
    }
    match FEATURES.iter().find(|(name, _)| *name == id) {
        Some((name, feature)) => Err(NoReader::NotBuilt { id: name, feature }),
        None => Err(NoReader::NoSuchReader),
    }
}

/// The ids this build can actually read, in [`crate::READERS`] order.
pub fn available() -> Vec<&'static str> {
    crate::READERS
        .iter()
        .copied()
        .filter(|id| built(id).is_some())
        .collect()
}

fn built(id: &str) -> Option<&'static dyn Reader> {
    match id {
        #[cfg(feature = "reader-txt")]
        "txt/1" => Some(&txt::Txt),
        #[cfg(feature = "reader-usfm")]
        "usfm/1" => Some(&usfm::Usfm),
        #[cfg(feature = "reader-json")]
        "json/1" => Some(&json::Json),
        #[cfg(feature = "reader-md")]
        "md/1" => Some(&md::Md),
        #[cfg(feature = "reader-osis")]
        "osis/1" => Some(&osis::Osis),
        #[cfg(feature = "reader-zefania")]
        "zefania/1" => Some(&zefania::Zefania),
        _ => None,
    }
}

pub mod books;

// Every reader that nests levels uses the shared builder; `txt/1` is the one that does not,
// because a line has nothing inside it. The list has to name all five, and it named three
// until `cargo build --features reader-md` alone failed to compile — which `--all-features`
// can never catch and `make crate-features` is for.
#[cfg(any(
    feature = "reader-md",
    feature = "reader-usfm",
    feature = "reader-osis",
    feature = "reader-zefania",
    feature = "reader-json"
))]
mod build;

#[cfg(any(feature = "reader-osis", feature = "reader-zefania"))]
mod xml;

#[cfg(feature = "reader-txt")]
pub mod txt;

#[cfg(feature = "reader-json")]
pub mod json;

#[cfg(feature = "reader-md")]
pub mod md;

#[cfg(feature = "reader-osis")]
pub mod osis;

#[cfg(feature = "reader-usfm")]
pub mod usfm;

#[cfg(feature = "reader-zefania")]
pub mod zefania;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::Caps;

    fn budget() -> Budget {
        Budget::new(Caps::DEFAULT, 1 << 16).unwrap()
    }

    struct Dummy;

    impl Reader for Dummy {
        fn id(&self) -> &'static str {
            "dummy/1"
        }

        fn params(&self) -> &'static [ParamSpec] {
            &[
                ParamSpec {
                    key: "date-format",
                    required: true,
                    what: "the order of day, month and year in a timestamp",
                },
                ParamSpec {
                    key: "tz",
                    required: false,
                    what: "the timezone offset to assume",
                },
            ]
        }

        fn read(
            &self,
            input: &Input<'_>,
            _params: &Params,
            limits: &mut Budget,
        ) -> Result<ReadOutput, LibError> {
            limits.scan(input.len() as u64)?;
            let text = Normalised::new(input.bytes())?;
            Ok(ReadOutput::new(
                text,
                Vec::new(),
                Level::new("line").expect("a level"),
            ))
        }
    }

    /// Every id this crate promises has a feature that builds it, and no row invents an id.
    #[test]
    fn every_reader_id_has_exactly_one_feature() {
        for id in crate::READERS {
            let rows: Vec<_> = FEATURES.iter().filter(|(name, _)| name == id).collect();
            assert_eq!(rows.len(), 1, "{id} should have one feature row");
        }
        assert_eq!(FEATURES.len(), crate::READERS.len());
        for (id, _) in FEATURES {
            assert!(crate::READERS.contains(id), "{id} is not a known reader");
        }
    }

    /// A missing feature and a nonexistent reader are different answers.
    ///
    /// The point of the distinction: `--reader osis/1` on a build without `reader-osis` is a
    /// build problem, and `--reader oasis/1` is a typo. One message for both would send the
    /// reader of it to the wrong place.
    #[test]
    fn an_unbuilt_reader_names_its_feature_and_an_unknown_one_does_not() {
        match reader("made-up/1") {
            Err(NoReader::NoSuchReader) => {}
            Err(other) => panic!("{other}"),
            Ok(r) => panic!("{} should not exist", r.id()),
        }
        for (id, feature) in FEATURES {
            match reader(id) {
                Ok(r) => assert_eq!(r.id(), *id),
                Err(NoReader::NotBuilt {
                    id: got,
                    feature: f,
                }) => {
                    assert_eq!(got, *id);
                    assert_eq!(f, *feature);
                }
                Err(e) => panic!("{id}: {e}"),
            }
        }
    }

    /// Whatever is built answers to the id it was asked for.
    #[test]
    fn every_available_reader_reports_the_id_it_was_found_under() {
        for id in available() {
            assert_eq!(reader(id).expect("built").id(), id);
        }
    }

    /// No TX-P1 reader takes a parameter, which is what keeps manifest key 3 an id alone.
    ///
    /// The format spec describes key 3 as "a reader id and version, such as `osis/1`". A
    /// parameter would have to be recorded there (see this module's header), and that is a
    /// spec change. This test fails on the commit that adds a parameterised reader, which is
    /// the commit where the spec, the three ports and this crate move together.
    #[test]
    fn no_reader_in_this_phase_takes_a_parameter() {
        for id in available() {
            let r = reader(id).expect("built");
            assert!(
                r.params().is_empty(),
                "{id} declares parameters; manifest key 3's grammar has to widen first"
            );
        }
    }

    #[test]
    fn an_unknown_parameter_is_refused_by_name_with_the_ones_that_exist() {
        let mut params = Params::new();
        params.set("dummy/1", "date-fromat", "dmy").expect("set");
        let err = params
            .check("dummy/1", Dummy.params())
            .expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "date-fromat");
                assert!(reason.contains("date-format"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_missing_required_parameter_is_refused_with_what_it_means() {
        let params = Params::new();
        let err = params
            .check("dummy/1", Dummy.params())
            .expect_err("refused");
        match err {
            LibError::BadParam { key, reason, .. } => {
                assert_eq!(key, "date-format");
                assert!(reason.contains("day, month and year"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// `read_with` is the only entry point, so no reader can run on unchecked parameters.
    #[test]
    fn reading_validates_the_parameters_first() {
        let mut params = Params::new();
        params.set("dummy/1", "tz", "+0300").expect("set");
        let input = Input::new(b"hello");
        let err = read_with(&Dummy, &input, &params, &mut budget()).expect_err("refused");
        assert!(matches!(err, LibError::BadParam { .. }), "{err:?}");
    }

    /// A value that could not be read back out of the field is refused when it is set.
    #[test]
    fn a_parameter_value_cannot_hold_what_the_field_separates_on() {
        let mut params = Params::new();
        for bad in ["a b", "a=b", "", "a\tb", "a\nb"] {
            params
                .set("dummy/1", "tz", bad)
                .expect_err(&format!("{bad:?} should be refused"));
        }
        for bad in ["TZ", "1tz", "tz_offset", ""] {
            params
                .set("dummy/1", bad, "x")
                .expect_err(&format!("{bad:?} should be refused"));
        }
    }

    #[test]
    fn a_reader_field_round_trips_with_and_without_parameters() {
        let plain = reader_field("txt/1", &Params::new());
        assert_eq!(plain, "txt/1");
        let (id, params) = parse_reader_field(&plain).expect("parsed");
        assert_eq!(id, "txt/1");
        assert!(params.is_empty());

        let mut params = Params::new();
        params.set("whatsapp/1", "tz", "+0300").expect("set");
        params.set("whatsapp/1", "date-format", "dmy").expect("set");
        let field = reader_field("whatsapp/1", &params);
        // Key order, not insertion order: the same two settings must spell one field.
        assert_eq!(field, "whatsapp/1 date-format=dmy tz=+0300");
        let (id, back) = parse_reader_field(&field).expect("parsed");
        assert_eq!(id, "whatsapp/1");
        assert_eq!(back, params);
    }

    #[test]
    fn a_field_this_code_could_not_have_written_is_not_guessed_at() {
        for bad in ["", "txt/1 noequals", "txt/1 =v", "txt/1 k=", "txt/1 K=v"] {
            assert!(parse_reader_field(bad).is_none(), "{bad:?}");
        }
    }
}
