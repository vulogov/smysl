//! The text a text-level measure reads (SMYSL-2.6 §3.7): a scope's parts, cut into segments at
//! the recipe's level and turned into one term stream.
//!
//! # A text scope is the `from` of an sq query
//!
//! An sq scope answers with units, and a text measure counts text, which a library holds
//! whether or not anything was extracted from it — a Bible added with `text add` and never
//! ingested has text to measure and no units to answer a query with. So the scope a text
//! measure reads is the query's **`from` items**, resolved to manifests by the rules
//! `smysl_text::scope` gives them: `texts` and `library` are the whole library, `collection:x`
//! the aliases `x` and `x/…`, `import:x` the same under the reserved prefix, and `store:` the
//! library by its own name. Every head of every alias in scope, parts in reading order.
//!
//! **Anything after the `from` is refused** (`SMY-E461`). A `where` filters units, and a text
//! has no units to filter until something is extracted from it; reading `where book = Ps` as a
//! filter on text would be inventing a meaning sq/1 does not give it. A scope narrower than an
//! alias is written as the alias of the narrower text, which is how this repository's libraries
//! already name a book (`from collection:"kjv/08-rut"`, quoted because sq reads a bare `/` as
//! the start of a stage). **A channel is refused too**, for now: a conversation is
//! found by locator head as well as by alias, and that half needs the reading, which is a
//! measure over messages (T5's, with `level = message`) and not M2's.
//!
//! The basis is unchanged: the whole query, run and saved by [`crate::scope::resolve`], is what
//! the result grounds on. The digest is §3.4's for text measures, over the sorted tids.
//!
//! # Two streams, one term per word
//!
//! `smysl_text::Chain::terms` puts a stem beside each surface form, which is right for
//! retrieval and would make a statistic count one word twice. A text measure counts
//! [`Stream::Surface`] or [`Stream::Stem`], one term per word each, and the recipe's
//! `analyzer` records which: `smysl/an-en/1:surface`. The stem stream carries the `stem-level`
//! flag (§3.6), since a Russian or German statistic over stems is not one over words.
//!
//! # Term ids
//!
//! A type's id is its rank among the scope's types in byte order of the term, so "term-id
//! order" (§3.5) is a property of the vocabulary and not of the order the text happened to
//! introduce it in.
//!
//! # Characters
//!
//! T2 and T6 compress characters, not terms, so a corpus keeps each expression's segments as
//! text too ([`Doc`]), in NFC: a library's parts are NFC, and NFC is not closed under slicing,
//! so a verse cut out of a part is normalised again. At `window:<n>` the characters are the
//! parts', since a window of terms has no characters of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::ids::{Mid, Tid};
use smysl_core::LibError;
use smysl_query::ast::{Expr, Name, ScopeItem};
use smysl_text::analyze::{self, Chain};
use smysl_text::Library;
use unicode_normalization::{is_nfc, UnicodeNormalization};

use crate::recipe::Level;

pub mod compress;
pub mod entropy;
pub mod lz_match;
pub mod ncd;
pub mod zipf_heaps;

/// Which of an analyzer's two streams is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum Stream {
    /// The folded word.
    #[default]
    Surface,
    /// The stem of the folded word; the surface where the chain has no stemmer.
    Stem,
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Stream::Surface => "surface",
            Stream::Stem => "stem",
        })
    }
}

/// The flag a stem-stream result carries.
pub const STEM_LEVEL: &str = "stem-level";

/// The flag a result over an admitted lossy text carries.
pub const LOSSY_ADMITTED: &str = "lossy-admitted";

/// An analyzer as a recipe writes it: `<chain id>:<stream>`.
pub fn analyzer_id(chain: &str, stream: Stream) -> String {
    format!("{chain}:{stream}")
}

/// A recipe's `analyzer`, read back: a chain this build can produce and a stream.
pub fn parse_analyzer(id: &str) -> Result<(Chain, Stream), Diagnostic> {
    let refuse =
        |why: &str| Diagnostic::new(Code::E489).with_message(format!("analyzer `{id}`: {why}"));
    let (chain, stream) = id
        .rsplit_once(':')
        .ok_or_else(|| refuse("expected `<chain>:surface` or `<chain>:stem`"))?;
    let stream = match stream {
        "surface" => Stream::Surface,
        "stem" => Stream::Stem,
        _ => return Err(refuse("the stream is `surface` or `stem`")),
    };
    let known: Vec<&str> = analyze::ids().collect();
    let chain = analyze::by_id(chain).ok_or_else(|| {
        refuse(&format!(
            "this build cannot produce that chain; it has {}",
            known.join(", ")
        ))
    })?;
    Ok((chain, stream))
}

/// What loading a text scope can fail with: a refusal, or the library itself.
#[derive(Debug)]
#[non_exhaustive]
pub enum TextError {
    Refused(Diagnostic),
    Library(LibError),
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::Refused(d) => write!(f, "{}: {}", d.code, d.message),
            TextError::Library(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TextError {}

impl From<Diagnostic> for TextError {
    fn from(d: Diagnostic) -> TextError {
        TextError::Refused(d)
    }
}

impl From<LibError> for TextError {
    fn from(e: LibError) -> TextError {
        TextError::Library(e)
    }
}

/// One expression in a text scope: its alias and its head manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Text {
    pub alias: String,
    pub mid: Mid,
}

/// The expressions a scope's `from` items name, in alias order.
pub fn texts(scope: &str, library: &Library) -> Result<Vec<Text>, Diagnostic> {
    let parsed = smysl_query::parse(scope)?;
    let q = &parsed.query;
    let refuse = |why: String| Diagnostic::new(Code::E461).with_message(why);
    let Expr::Pipeline(p) = &q.expr else {
        return Err(refuse(
            "a text scope is one `from`; a set expression combines units, not texts".into(),
        ));
    };
    let Some(from) = &p.scope else {
        return Err(refuse(
            "a text scope names its texts: `from texts`, `from collection:<alias>`".into(),
        ));
    };
    let extra = p.source.is_some()
        || !p.filters.is_empty()
        || !p.times.is_empty()
        || !p.traversals.is_empty()
        || !p.closures.is_empty()
        || !q.ordering.is_empty()
        || !q.shaping.is_empty()
        || q.output.is_some();
    if extra {
        return Err(refuse(
            "a text measure reads only a scope's `from`: the stages after it select units, \
             and a text has none to select until something is extracted from it; name a \
             narrower text by its alias instead"
                .into(),
        ));
    }
    let catalog = library.catalog();
    let under = |name: &str| -> Vec<&String> {
        catalog
            .keys()
            .filter(|a| *a == name || a.strip_prefix(name).is_some_and(|r| r.starts_with('/')))
            .collect()
    };
    let mut aliases: BTreeSet<&String> = BTreeSet::new();
    for item in &from.items {
        match item {
            ScopeItem::Texts | ScopeItem::Library | ScopeItem::Store(_) => {
                aliases.extend(catalog.keys())
            }
            ScopeItem::Collection(n) => {
                let found = under(name_of(n));
                if found.is_empty() {
                    return Err(Diagnostic::new(Code::E511).with_message(format!(
                        "`collection:{}`: no expression in the catalogue has that alias or lies \
                         under it",
                        name_of(n)
                    )));
                }
                aliases.extend(found);
            }
            ScopeItem::Import(n) => {
                let name = format!("import:{}", name_of(n));
                let found = under(&name);
                if found.is_empty() {
                    return Err(Diagnostic::new(Code::E511).with_message(format!(
                        "`{name}`: no imported expression in the catalogue has that name"
                    )));
                }
                aliases.extend(found);
            }
            _ => {
                return Err(refuse(
                    "a channel is a text measure over messages, which this build does not \
                     have yet; name the conversation's alias with `collection:`"
                        .into(),
                ))
            }
        }
    }
    let mut out = Vec::new();
    for alias in aliases {
        match catalog[alias].as_slice() {
            [mid] => out.push(Text {
                alias: alias.clone(),
                mid: *mid,
            }),
            [] => {}
            heads => {
                return Err(Diagnostic::new(Code::E489).with_message(format!(
                    "`{alias}` has {} heads (`SMY-W418`); a measure over both would count \
                     the text they share twice, and over one would choose between them",
                    heads.len()
                )))
            }
        }
    }
    Ok(out)
}

fn name_of(n: &Name) -> &str {
    match n {
        Name::Ident(s) | Name::Str(s) => s,
        _ => "",
    }
}

/// One expression's characters at the recipe's level, NFC, empty segments dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub alias: String,
    pub segments: Vec<String>,
}

impl Doc {
    pub fn new(alias: impl Into<String>, segments: Vec<String>) -> Doc {
        Doc {
            alias: alias.into(),
            segments: segments
                .iter()
                .filter(|s| !s.is_empty())
                .map(|s| nfc(s))
                .collect(),
        }
    }

    /// The text a compressor reads: the segments joined by `\n`.
    pub fn text(&self) -> String {
        self.segments.join("\n")
    }
}

fn nfc(s: &str) -> String {
    if is_nfc(s) {
        s.to_string()
    } else {
        s.nfc().collect()
    }
}

/// A text scope, read: segments of term ids under one analyzer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Corpus {
    /// `<chain id>:<stream>`, the recipe's `analyzer`.
    pub analyzer: String,
    pub stream: Stream,
    pub level: Level,
    /// Every type, by id: byte order of the term.
    pub types: Vec<String>,
    /// The segments, in reading order, each its terms in order. Empty segments are dropped.
    pub segments: Vec<Vec<u32>>,
    /// The parts read.
    pub tids: BTreeSet<Tid>,
    /// §3.4's `scope:digest` for a text measure: [`crate::scope::digest`] of the sorted tids.
    pub digest: [u8; 32],
    /// The aliases read, in order.
    pub texts: Vec<String>,
    /// Whether a manifest in scope is lossy.
    pub lossy: bool,
    /// Each expression's characters, in alias order; none for a corpus built from terms.
    pub docs: Vec<Doc>,
    /// The index in `segments` of each expression's first segment, in alias order — one, at
    /// 0, for a corpus built from terms, which is one unnamed text.
    pub starts: Vec<usize>,
}

impl Corpus {
    /// A corpus from terms already split into segments — a harness, a synthetic text. It has
    /// no parts, so its digest is the digest of nothing and its texts are none.
    pub fn from_terms(
        analyzer: impl Into<String>,
        stream: Stream,
        level: Level,
        segments: Vec<Vec<String>>,
    ) -> Corpus {
        Corpus::build(
            analyzer.into(),
            stream,
            level,
            segments,
            BTreeSet::new(),
            Vec::new(),
            false,
            Vec::new(),
            vec![0],
        )
    }

    /// A corpus from texts already split into segments — a harness's characters, read under
    /// the plain chain's surface stream at `part`, one segment a part. Like
    /// [`Corpus::from_terms`] it has no parts and its digest is the digest of nothing.
    pub fn from_docs(docs: Vec<Doc>) -> Corpus {
        let chain = Chain::plain();
        let mut starts = Vec::with_capacity(docs.len());
        let mut segments = Vec::new();
        for d in &docs {
            starts.push(segments.len());
            segments.extend(d.segments.iter().map(|s| chain.surface(s)));
        }
        let texts = docs.iter().map(|d| d.alias.clone()).collect();
        Corpus::build(
            analyzer_id(chain.id(), Stream::Surface),
            Stream::Surface,
            Level::Part,
            segments,
            BTreeSet::new(),
            texts,
            false,
            docs,
            starts,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build(
        analyzer: String,
        stream: Stream,
        level: Level,
        segments: Vec<Vec<String>>,
        tids: BTreeSet<Tid>,
        texts: Vec<String>,
        lossy: bool,
        docs: Vec<Doc>,
        starts: Vec<usize>,
    ) -> Corpus {
        // The starts count the segments kept: an empty segment is dropped below.
        let starts = starts
            .iter()
            .map(|&s| segments[..s].iter().filter(|x| !x.is_empty()).count())
            .collect();
        let vocab: BTreeSet<&String> = segments.iter().flatten().collect();
        let ids: BTreeMap<&String, u32> = vocab
            .iter()
            .enumerate()
            .map(|(i, t)| (*t, u32::try_from(i).expect("fewer than 2^32 types")))
            .collect();
        let segs: Vec<Vec<u32>> = segments
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.iter().map(|t| ids[t]).collect())
            .collect();
        let types = vocab.into_iter().cloned().collect();
        Corpus {
            analyzer,
            stream,
            level,
            types,
            segments: segs,
            digest: crate::scope::digest(tids.iter().map(|t| t.as_uid().to_bytes())),
            tids,
            texts,
            lossy,
            docs,
            starts,
        }
    }

    /// Read `texts` from `library` at `level`, under `analyzer` when the recipe names one and
    /// under the chain the texts' language chooses when it does not.
    ///
    /// A scope whose parts choose two different chains is refused (`SMY-E489`) unless the
    /// recipe names one: the two halves would be counted in two vocabularies, and a statistic
    /// over the union of a stemmed and an unstemmed text describes neither.
    pub fn load(
        library: &Library,
        texts: &[Text],
        level: Level,
        analyzer: Option<&str>,
    ) -> Result<Corpus, TextError> {
        if level == Level::Window(0) {
            return Err(Diagnostic::new(Code::E489)
                .with_message("`window:0` is a window with nothing in it")
                .into());
        }
        let store = library.store();
        let mut parts: Vec<(&Text, Tid, String)> = Vec::new();
        let mut lossy = false;
        let mut chains: BTreeSet<&'static str> = BTreeSet::new();
        let mut chain_for: Option<Chain> = None;
        for t in texts {
            let manifest = store.manifest(&t.mid).ok_or_else(|| {
                Diagnostic::new(Code::E511).with_message(format!(
                    "`{}`: no manifest `{}`",
                    t.alias,
                    t.mid.canonical()
                ))
            })?;
            lossy |= manifest.lossy;
            for p in &manifest.parts {
                let lang = p.lang.as_ref().unwrap_or(&manifest.lang);
                let c = Chain::new(lang);
                chains.insert(c.id());
                chain_for.get_or_insert(c);
                parts.push((t, p.tid, lang.to_string()));
            }
        }
        let (chain, stream) = match analyzer {
            Some(id) => parse_analyzer(id)?,
            None => {
                if chains.len() > 1 {
                    return Err(Diagnostic::new(Code::E489)
                        .with_message(format!(
                            "the scope's texts are analysed by {}; name one in the recipe's \
                             `analyzer` to count them in one vocabulary",
                            chains.into_iter().collect::<Vec<_>>().join(" and ")
                        ))
                        .into());
                }
                (chain_for.unwrap_or_else(Chain::plain), Stream::Surface)
            }
        };
        let split = |s: &str| match stream {
            Stream::Surface => chain.surface(s),
            Stream::Stem => chain.stems(s),
        };

        let mut segments: Vec<Vec<String>> = Vec::new();
        let mut tids = BTreeSet::new();
        let mut aliases: Vec<String> = Vec::new();
        let mut window: Vec<String> = Vec::new();
        let mut docs: Vec<Doc> = Vec::new();
        let mut chars: Vec<String> = Vec::new();
        let mut starts: Vec<usize> = Vec::new();
        let mut last: Option<&str> = None;
        for (t, tid, _lang) in &parts {
            if last != Some(t.alias.as_str()) {
                // A window does not run across two expressions: they are two texts.
                if !window.is_empty() {
                    segments.push(std::mem::take(&mut window));
                }
                if let Some(prev) = last {
                    docs.push(Doc::new(prev, std::mem::take(&mut chars)));
                }
                starts.push(segments.len());
                aliases.push(t.alias.clone());
                last = Some(t.alias.as_str());
            }
            tids.insert(*tid);
            let part = library.part(tid)?;
            let text = String::from_utf8(part.text).map_err(|_| LibError::ObjectCorrupt {
                id: tid.canonical(),
            })?;
            match level {
                Level::Part => {
                    segments.push(split(&text));
                    chars.push(text);
                }
                Level::Window(n) => {
                    chars.push(text.clone());
                    for term in split(&text) {
                        window.push(term);
                        if window.len() == n as usize {
                            segments.push(std::mem::take(&mut window));
                        }
                    }
                }
                named => {
                    let want = named.to_string();
                    let reading = library.part_reading(&t.mid, tid)?;
                    for row in reading.rows.iter().filter(|r| r.level.as_str() == want) {
                        let s =
                            text.get(row.start as usize..row.end as usize)
                                .ok_or_else(|| LibError::ObjectCorrupt {
                                    id: format!("{} ({}..{})", tid.canonical(), row.start, row.end),
                                })?;
                        segments.push(split(s));
                        chars.push(s.to_string());
                    }
                }
            }
        }
        if !window.is_empty() {
            segments.push(window);
        }
        if let Some(prev) = last {
            docs.push(Doc::new(prev, chars));
        }
        Ok(Corpus::build(
            analyzer_id(chain.id(), stream),
            stream,
            level,
            segments,
            tids,
            aliases,
            lossy,
            docs,
            starts,
        ))
    }

    /// The segments of expression `k`, as a range of [`Corpus::segments`].
    pub fn text_range(&self, k: usize) -> (usize, usize) {
        let to = self
            .starts
            .get(k + 1)
            .copied()
            .unwrap_or(self.segments.len());
        (self.starts[k], to)
    }

    /// Tokens in scope.
    pub fn tokens(&self) -> u64 {
        self.segments.iter().map(|s| s.len() as u64).sum()
    }

    /// Each segment's counts, as `(type, count)` in type order: what a bootstrap replicate
    /// adds up, so that a replicate costs the segment's distinct types rather than its tokens.
    pub fn segment_counts(&self) -> Vec<Vec<(u32, u64)>> {
        self.segments
            .iter()
            .map(|s| {
                let mut c: BTreeMap<u32, u64> = BTreeMap::new();
                for &t in s {
                    *c.entry(t).or_default() += 1;
                }
                c.into_iter().collect()
            })
            .collect()
    }

    /// Counts per type over the segments `drawn`, into `out` (resized to the vocabulary).
    pub fn counts_into(
        per_segment: &[Vec<(u32, u64)>],
        drawn: &[usize],
        out: &mut Vec<u64>,
        v: usize,
    ) {
        out.clear();
        out.resize(v, 0);
        for &i in drawn {
            for &(t, c) in &per_segment[i] {
                out[t as usize] += c;
            }
        }
    }
}
