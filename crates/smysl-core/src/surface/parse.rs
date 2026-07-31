//! The surface parser (§15.3, Appendix A).
//!
//! **The parser never returns `Err` for a malformed record.** It emits a diagnostic with a
//! byte span and recovers to the next record start. A hard error is returned only for an
//! unparseable `@doc` header, because without it nothing downstream knows what it is
//! reading.
//!
//! That is not politeness. The ingest repair loop resends only the offending region
//! (§22.3), which requires per-span diagnostics; a fail-fast parser would force
//! whole-chunk retries and turn one bad unit into a lost chunk.
//!
//! References are authored as labels, so parsing is two-pass: collect raw units, then
//! resolve labels to uids in dependency order, because a unit's uid depends on the uids of
//! everything it points at.

use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Code, Diagnostic, Span, Subject};
use crate::error::ParseError;
use crate::hash::canonical_uid;
use crate::ids::{AgentId, Label, LangTag, SchemaId, ThreadId, Uid, ViewId};
use crate::surface::hjson::{parse_object_prefix, HObject, HValue, Spanned};
use crate::surface::lex::{arrow_len, find_arrow, lex, Line, LineClass};
use crate::surface::payload::object_to_payload;
use crate::types::epistemics::{Date, SourceKind, SourceRef, Status};
use crate::types::provenance::Hlc;
use crate::types::relation::{RelKind, Relation};
use crate::types::thread::{Role, Step, Thread, ThreadSchema};
use crate::types::unit::{UnitCore, UnitCoreBuilder};
use crate::types::view::{Admission, GranularityProfile, View};
use crate::types::Record;

/// The result of parsing a surface document.
///
/// `labels` and `salience` are here rather than on the records because neither is part of
/// a unit's identity (§1.2) - but both are authored in surface syntax, so a round trip
/// would lose them if the outcome did not carry them.
#[derive(Debug, Clone, Default)]
pub struct ParseOutcome {
    pub view: Option<View>,
    pub records: Vec<Record>,
    pub labels: BTreeMap<Label, Uid>,
    pub salience: BTreeMap<Uid, f32>,
    pub diagnostics: Vec<Diagnostic>,
    /// Records skipped by error recovery.
    pub recovered: usize,
    /// Comment lines seen and dropped.
    ///
    /// Comments are not part of any record, so nothing downstream can carry them and a
    /// re-emission cannot reproduce them. Counted so that a *writer* can say so instead of
    /// deleting a reviewer's notes in silence - the manual recommends `fmt --write` as a
    /// pre-commit habit, which makes silent loss the difference between a formatter and a
    /// hazard.
    pub comments: usize,
}

/// Undo the leading-backslash escape on a body or detail line.
///
/// A line beginning `#` or `//` at column 0 is a comment wherever it sits, which is what
/// stops a note between two records from being absorbed into the previous unit's body. The
/// cost was that a body could never *start* a line with either marker — a real limitation
/// for exactly the content this format carries, since a Markdown heading and a line of C++
/// both do.
///
/// `\#`, `\//` and `\\` at column 0 are the escape. Only those three: a backslash before
/// anything else is an ordinary backslash, so prose full of Windows paths and LaTeX needs no
/// thought. `\\` is escapable because otherwise a body line starting with a literal
/// backslash could not round-trip.
fn unescape_body_line(line: &str) -> std::borrow::Cow<'_, str> {
    match line.strip_prefix('\\') {
        Some(rest) if rest.starts_with('#') || rest.starts_with("//") || rest.starts_with('\\') => {
            std::borrow::Cow::Borrowed(rest)
        }
        _ => std::borrow::Cow::Borrowed(line),
    }
}

impl ParseOutcome {
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    pub fn units(&self) -> impl Iterator<Item = &UnitCore> {
        self.records.iter().filter_map(Record::as_unit)
    }

    /// The uid a label resolved to.
    pub fn uid_of(&self, label: &Label) -> Option<Uid> {
        self.labels.get(label).copied()
    }
}

/// A reference as authored: a label, or a full uid.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Ref {
    Label(Label),
    Uid(Uid),
}

fn parse_ref(s: &str) -> Option<Ref> {
    if s.starts_with("b3:") {
        return Uid::parse(s).ok().map(Ref::Uid);
    }
    Label::new(s).ok().map(Ref::Label)
}

struct RawUnit {
    label: Option<Label>,
    schema: SchemaId,
    gist: String,
    body: Option<String>,
    detail: Option<String>,
    deps: Vec<Spanned<Ref>>,
    grounds: Vec<Spanned<Ref>>,
    status: Status,
    source: Option<SourceRef>,
    payload: Option<Vec<u8>>,
    salience: Option<f32>,
    span: Span,
}

struct RawRelation {
    kind: RelKind,
    from: Spanned<Ref>,
    to: Spanned<Ref>,
    weight: Option<f32>,
    note: Option<Spanned<Ref>>,
    span: Span,
}

struct RawThread {
    id: ThreadId,
    schema: ThreadSchema,
    owner: AgentId,
    gist: String,
    steps: Vec<(Role, Spanned<Ref>, Option<String>)>,
    ts: Hlc,
    span: Span,
}

struct RawView {
    id: ViewId,
    roots: Vec<Spanned<Ref>>,
    threads: BTreeSet<ThreadId>,
    requires: BTreeSet<SchemaId>,
    granularity: GranularityProfile,
    intent: String,
    lang: LangTag,
}

/// Parse surface text.
pub fn parse_surface(src: &str) -> Result<ParseOutcome, ParseError> {
    let mut p = Parser {
        src,
        lines: lex(src),
        i: 0,
        out: ParseOutcome::default(),
        units: Vec::new(),
        relations: Vec::new(),
        threads: Vec::new(),
        view: None,
    };
    p.run()?;
    Ok(p.finish())
}

struct Parser<'a> {
    src: &'a str,
    lines: Vec<Line<'a>>,
    i: usize,
    out: ParseOutcome,
    units: Vec<RawUnit>,
    relations: Vec<RawRelation>,
    threads: Vec<RawThread>,
    view: Option<RawView>,
}

impl<'a> Parser<'a> {
    fn line(&self) -> Option<&Line<'a>> {
        self.lines.get(self.i)
    }

    fn err(&mut self, code: Code, span: Span, msg: impl Into<String>) {
        self.out
            .diagnostics
            .push(Diagnostic::at(code, span).with_message(msg));
    }

    /// Skip to the next record start, counting one recovered record.
    fn recover(&mut self) {
        self.out.recovered += 1;
        self.i += 1;
        while let Some(l) = self.line() {
            if l.class.starts_record() {
                return;
            }
            self.i += 1;
        }
    }

    fn run(&mut self) -> Result<(), ParseError> {
        while self.i < self.lines.len() {
            let l = self.lines[self.i];
            match l.class {
                LineClass::Blank => self.i += 1,
                // A comment between records carries nothing the graph can hold, so it is
                // skipped rather than diagnosed - but counted, so a writer can report that
                // re-emitting the document will not reproduce it.
                LineClass::Comment => {
                    self.out.comments += 1;
                    self.i += 1;
                }
                LineClass::DocHeader => self.doc_header()?,
                LineClass::RecordStart => {
                    if let Some(u) = self.unit() {
                        self.units.push(u);
                    }
                }
                LineClass::RelLine => {
                    if let Some(r) = self.relation() {
                        self.relations.push(r);
                    }
                    self.i += 1;
                }
                LineClass::ThreadStart => {
                    if let Some(t) = self.thread() {
                        self.threads.push(t);
                    }
                }
                _ => {
                    self.err(
                        Code::E001,
                        l.span,
                        format!("stray {:?} outside a record", l.class),
                    );
                    self.recover();
                }
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // @doc
    // -----------------------------------------------------------------------

    fn doc_header(&mut self) -> Result<(), ParseError> {
        let l = self.lines[self.i];
        let rest = l.text.strip_prefix("@doc").unwrap_or("").trim_start();
        let version = rest.split_whitespace().next().unwrap_or("");
        if !crate::format_version_supported(version) {
            return Err(ParseError::UnsupportedFormatVersion {
                found: version.to_string(),
            });
        }

        let (mut header, header_span) = match self.header_object(l) {
            Ok(h) => h,
            Err(()) => {
                return Err(ParseError::Syntax {
                    span: l.span,
                    message: "unparseable @doc header".into(),
                })
            }
        };

        let id = header
            .take("id")
            .and_then(|v| v.value.as_str().and_then(|s| ViewId::new(s).ok()))
            .unwrap_or_else(|| ViewId::new("v/doc").expect("a valid literal"));

        let roots = self.ref_list(&mut header, "roots");
        let threads: BTreeSet<ThreadId> = header
            .take("threads")
            .map(|v| self.id_list(&v, |s: &str| ThreadId::new(s)))
            .unwrap_or_default();
        let requires: BTreeSet<SchemaId> = header
            .take("requires")
            .map(|v| self.id_list(&v, |s: &str| SchemaId::parse(s)))
            .unwrap_or_default();

        for r in &requires {
            if let Some(major) = r.kernel_major() {
                if major != crate::KERNEL_MAJOR {
                    return Err(ParseError::UnsupportedKernelMajor {
                        found: r.as_str().to_string(),
                    });
                }
            }
        }

        let granularity = header
            .take("granularity")
            .map(|v| self.granularity(&v))
            .unwrap_or_default();
        let intent = header
            .take("intent")
            .and_then(|v| v.value.as_str().map(str::to_string))
            .unwrap_or_default();
        let lang = header
            .take("lang")
            .and_then(|v| v.value.as_str().and_then(|s| LangTag::new(s).ok()))
            .unwrap_or_default();

        self.view = Some(RawView {
            id,
            roots,
            threads,
            requires,
            granularity,
            intent,
            lang,
        });
        self.advance_past(header_span.end);
        Ok(())
    }

    fn granularity(&mut self, v: &Spanned<HValue>) -> GranularityProfile {
        let Some(o) = v.value.as_object() else {
            self.err(Code::E001, v.span, "granularity must be an object");
            return GranularityProfile::default();
        };
        let mut g = o
            .get("profile")
            .and_then(|p| p.value.as_str())
            .and_then(GranularityProfile::preset)
            .unwrap_or_default();
        if let Some(n) = o.get("l0_max").and_then(|x| x.value.as_int()) {
            g.l0_max = n.max(0) as u32;
        }
        if let Some(r) = o.get("l1_range").and_then(|x| x.value.as_array()) {
            if r.len() == 2 {
                g.l1_min = r[0].value.as_int().unwrap_or(0).max(0) as u32;
                g.l1_max = r[1].value.as_int().unwrap_or(0).max(0) as u32;
            }
        }
        if let Some(a) = o.get("admission").and_then(|x| x.value.as_str()) {
            if let Some(a) = Admission::parse(a) {
                g.admission = a;
            }
        }
        if let Some(p) = o.get("profile").and_then(|x| x.value.as_str()) {
            g.profile = p.to_string();
        }
        g
    }

    // -----------------------------------------------------------------------
    // units
    // -----------------------------------------------------------------------

    fn unit(&mut self) -> Option<RawUnit> {
        let start = self.lines[self.i];
        let after_sigil = &start.text[1..];
        let mut words = after_sigil.split_whitespace();
        let ty = words.next().unwrap_or("");
        // `parse_forward`: a bare type this build does not know becomes
        // `SchemaId::UnknownKernel` rather than a refusal, so a document written by a later
        // version parses here and re-emits unchanged. It is not silent - `check`'s extension
        // pass reports every one as `SMY-W010`, naming the type.
        //
        // This does change what a typo does. `@clai c/a { … }` used to be a hard `SMY-E001`
        // and is now a warning naming `clai`. The tool cannot tell a typo from a kernel type
        // added next year - the two are structurally identical - so it can have forward
        // compatibility or typo-as-error, not both. `--strict` restores the failure for
        // anyone who wants it, and the message is more precise than it was.
        let schema = match SchemaId::parse_forward(ty) {
            Ok(s) => s,
            Err(_) => {
                self.err(Code::E001, start.span, format!("unknown unit type `{ty}`"));
                self.recover();
                return None;
            }
        };

        // An optional label follows the type, before any `{`.
        let head_end = start
            .text
            .find('{')
            .map(|p| start.span.start + p)
            .unwrap_or(start.span.end);
        let head = &self.src[start.span.start..head_end];
        let label_word = head.split_whitespace().nth(1);
        let label = match label_word {
            None => None,
            Some(w) => match Label::new(w) {
                Ok(l) => Some(l),
                Err(_) => {
                    self.err(Code::E001, start.span, format!("malformed label `{w}`"));
                    self.recover();
                    return None;
                }
            },
        };

        let (mut header, header_span) = match self.header_object(start) {
            Ok(h) => h,
            Err(()) => {
                self.recover();
                return None;
            }
        };

        let status = match header.take("status") {
            None => Status::Speculative,
            Some(v) => match v.value.as_str().and_then(Status::parse) {
                Some(s) => s,
                None => {
                    self.err(Code::E001, v.span, "unknown status");
                    self.recover();
                    return None;
                }
            },
        };

        let deps = self.ref_list(&mut header, "deps");
        let grounds = self.ref_list(&mut header, "grounds");
        let source = header.take("source").and_then(|v| self.source(&v));
        let salience = header
            .take("salience")
            .and_then(|v| v.value.as_f64())
            .map(|f| f as f32);

        let payload = object_to_payload(&header);

        // The gist MUST immediately follow the header block (§6).
        let (gist, body, detail) = match self.gist_body_detail(header_span) {
            Some(t) => t,
            None => {
                self.recover();
                return None;
            }
        };

        let span = Span::new(start.span.start, self.current_offset());
        Some(RawUnit {
            label,
            schema,
            gist,
            body,
            detail,
            deps,
            grounds,
            status,
            source,
            payload,
            salience,
            span,
        })
    }

    /// Collect the gist, body, and detail that follow a header, leaving `self.i` on the
    /// line after them.
    fn gist_body_detail(
        &mut self,
        header_span: Span,
    ) -> Option<(String, Option<String>, Option<String>)> {
        self.advance_past(header_span.end);

        let Some(l) = self.line() else {
            self.err(Code::E021, header_span, "record ends before its gist");
            return None;
        };
        if l.class != LineClass::Gist {
            self.err(
                Code::E021,
                l.span,
                "the gist must immediately follow the header block",
            );
            return None;
        }

        let mut gist = l.gist_text().to_string();
        self.i += 1;
        // Continuation lines are indented by two spaces.
        while let Some(l) = self.line() {
            if l.class == LineClass::Text && l.text.starts_with("  ") {
                gist.push(' ');
                gist.push_str(l.text.trim());
                self.i += 1;
            } else {
                break;
            }
        }
        // Trim the assembled gist, not just test it. `~` alone followed by a continuation
        // line produced a gist with a leading space, and the writer emits `~ ` + gist while
        // the reader strips the sigil *and* the whitespace after it — so that space was
        // silently eaten on re-parse and the uid moved with it. A gist is a one-line
        // summary; surrounding whitespace was never content. Found by fuzzing.
        let gist = gist.trim().to_string();
        if gist.is_empty() {
            self.err(Code::E021, header_span, "empty gist");
            return None;
        }

        let body = self.block(false);
        let detail = if self.line().map(|l| l.class) == Some(LineClass::Separator) {
            self.i += 1;
            self.block(true)
        } else {
            None
        };
        Some((gist, body, detail))
    }

    /// Accumulate a markdown block up to the next record start, or - unless `through`
    /// is set - the next `--` separator.
    fn block(&mut self, through_separator: bool) -> Option<String> {
        let mut lines: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        while let Some(l) = self.line() {
            match l.class {
                c if c.starts_record() => break,
                LineClass::Separator if !through_separator => break,
                LineClass::Gist => break,
                // Skipped, not kept and not a terminator. A body runs from the gist to the
                // next record, so a comment sitting *between* records falls inside this
                // range - keeping it made the comment become the previous unit's body,
                // which is worse than any alternative: content invented from a note, and
                // a granularity warning fired about it.
                //
                // A body line that genuinely needs to start with `#` or `//` writes `\#`
                // or `\//`, unescaped below. That escape is 0.6.0; before it, such a line
                // was a comment wherever it appeared and was silently dropped.
                LineClass::Comment => {
                    self.out.comments += 1;
                    self.i += 1;
                }
                _ => {
                    lines.push(unescape_body_line(l.text));
                    self.i += 1;
                }
            }
        }
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
        }
        while lines.first().is_some_and(|l| l.trim().is_empty()) {
            lines.remove(0);
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// Move to the first line that starts after `end`. A header object may span lines,
    /// so line-at-a-time advancement is not enough.
    fn advance_past(&mut self, end: usize) {
        while self.i < self.lines.len() && self.lines[self.i].span.start < end {
            self.i += 1;
        }
    }

    fn current_offset(&self) -> usize {
        self.lines
            .get(self.i.saturating_sub(1))
            .map(|l| l.span.end)
            .unwrap_or(self.src.len())
    }

    // -----------------------------------------------------------------------
    // relations
    // -----------------------------------------------------------------------

    fn relation(&mut self) -> Option<RawRelation> {
        let l = self.lines[self.i];
        let rest = l.text.strip_prefix("@rel")?.trim_start();
        let base = l.span.start + (l.text.len() - rest.len());

        // `<from> --kind--> <to> [{ … }]`
        let Some(open) = rest.find("--") else {
            self.err(Code::E001, l.span, "relation is missing `--kind-->`");
            return None;
        };
        let after = &rest[open + 2..];
        let Some(close) = after.find("-->") else {
            self.err(Code::E001, l.span, "relation is missing `-->`");
            return None;
        };
        let from_txt = rest[..open].trim();
        let kind_txt = after[..close].trim();
        let tail = after[close + 3..].trim_start();

        let (to_txt, header_at) = match tail.find('{') {
            Some(p) => (tail[..p].trim(), Some(base + (rest.len() - tail.len()) + p)),
            None => (tail.trim(), None),
        };

        let kind = match RelKind::parse(kind_txt) {
            Ok(k) => k,
            Err(_) => {
                self.err(
                    Code::W013,
                    l.span,
                    format!("unknown relation kind `{kind_txt}`"),
                );
                RelKind::Extension(kind_txt.to_string())
            }
        };

        let from = self.reference(from_txt, l.span)?;
        let to = self.reference(to_txt, l.span)?;

        let (mut weight, mut note) = (None, None);
        if let Some(at) = header_at {
            if let Ok(o) = parse_object_prefix(&self.src[at..], at) {
                weight = o
                    .value
                    .get("weight")
                    .and_then(|v| v.value.as_f64())
                    .map(|f| f as f32);
                note = o
                    .value
                    .get("note")
                    .and_then(|v| v.value.as_str())
                    .and_then(parse_ref)
                    .map(|r| Spanned::new(r, l.span));
            } else {
                self.err(Code::E001, l.span, "unparseable relation header");
            }
        }

        Some(RawRelation {
            kind,
            from,
            to,
            weight,
            note,
            span: l.span,
        })
    }

    // -----------------------------------------------------------------------
    // threads
    // -----------------------------------------------------------------------

    fn thread(&mut self) -> Option<RawThread> {
        let start = self.lines[self.i];
        let head_end = start
            .text
            .find('{')
            .map(|p| start.span.start + p)
            .unwrap_or(start.span.end);
        let head = &self.src[start.span.start..head_end];
        let id_word = head.split_whitespace().nth(1).unwrap_or("");
        let id = match ThreadId::new(id_word) {
            Ok(i) => i,
            Err(_) => {
                self.err(
                    Code::E001,
                    start.span,
                    format!("malformed thread id `{id_word}`"),
                );
                self.recover();
                return None;
            }
        };

        let (mut header, header_span) = match self.header_object(start) {
            Ok(h) => h,
            Err(()) => {
                self.recover();
                return None;
            }
        };

        let schema = match header
            .take("schema")
            .and_then(|v| v.value.as_str().and_then(ThreadSchema::parse))
        {
            Some(s) => s,
            None => {
                self.err(Code::E001, start.span, "thread is missing a known `schema`");
                self.recover();
                return None;
            }
        };
        let owner = match header
            .take("owner")
            .and_then(|v| v.value.as_str().and_then(|s| AgentId::new(s).ok()))
        {
            Some(o) => o,
            None => {
                self.err(Code::E001, start.span, "thread is missing a valid `owner`");
                self.recover();
                return None;
            }
        };
        let ts = header
            .take("ts")
            .and_then(|v| self.hlc(&v, &owner))
            .unwrap_or_else(|| Hlc::zero(owner.clone()));

        self.advance_past(header_span.end);

        let mut gist = String::new();
        if self.line().map(|l| l.class) == Some(LineClass::Gist) {
            gist = self.lines[self.i].gist_text().to_string();
            self.i += 1;
            while let Some(l) = self.line() {
                if l.class == LineClass::Text && l.text.starts_with("  ") {
                    gist.push(' ');
                    gist.push_str(l.text.trim());
                    self.i += 1;
                } else {
                    break;
                }
            }
        }
        // Trimmed for the same reason a unit's gist is, and missed when that one was fixed:
        // the writer emits `~ ` + gist and the reader strips the sigil *and* the whitespace
        // after it, so surrounding space cannot survive a round trip. A thread's gist runs
        // through this path rather than `gist_body_detail`, so the 0.4.0 fix never reached
        // it. Found by seeding the fuzzer with the repo's own corpus fixtures, which reach
        // `@thread` in seconds where a cold random search does not.
        let gist = gist.trim().to_string();

        let mut steps = Vec::new();
        while let Some(l) = self.line() {
            match l.class {
                LineClass::Step => {
                    if let Some(s) = self.step(*l) {
                        steps.push(s);
                    }
                    self.i += 1;
                }
                LineClass::Blank => self.i += 1,
                _ => break,
            }
        }

        Some(RawThread {
            id,
            schema,
            owner,
            gist,
            steps,
            ts,
            span: Span::new(start.span.start, self.current_offset()),
        })
    }

    fn step(&mut self, l: Line<'a>) -> Option<(Role, Spanned<Ref>, Option<String>)> {
        let body = &l.text[2..];
        let at = find_arrow(body)?;
        let role_txt = body[..at].trim();
        let rest = body[at + arrow_len(body, at)..].trim();
        // A step is `role -> target[: note]`, and the note separator is a colon - but a
        // canonical uid contains one too (`b3:` + 52 chars). Splitting on the first colon
        // therefore tore `b3:xxxx` into the reference `b3` and a note, so a thread step
        // could name a label and never a uid. That made `write_surface` output unparseable
        // whenever a step's target had no label to fall back on, which is exactly what
        // `merge` produces for a unit none of its inputs named.
        //
        // Skip past the uid's own colon before looking for the note's.
        let note_from = if rest.starts_with(Uid::PREFIX) {
            Uid::PREFIX.len()
        } else {
            0
        };
        let (ref_txt, note) = match rest[note_from..].split_once(':') {
            Some((r, n)) => (
                rest[..note_from + r.len()].trim(),
                Some(n.trim().to_string()),
            ),
            None => (rest, None),
        };
        let role = match Role::parse(role_txt) {
            Some(r) => r,
            None => {
                self.err(Code::E001, l.span, format!("unknown role `{role_txt}`"));
                return None;
            }
        };
        let r = self.reference(ref_txt, l.span)?;
        Some((role, r, note))
    }

    fn hlc(&mut self, v: &Spanned<HValue>, owner: &AgentId) -> Option<Hlc> {
        let a = v.value.as_array()?;
        if a.len() != 2 {
            return None;
        }
        Some(Hlc::new(
            a[0].value.as_int()? as u64,
            a[1].value.as_int()? as u32,
            owner.clone(),
        ))
    }

    // -----------------------------------------------------------------------
    // shared header helpers
    // -----------------------------------------------------------------------

    /// Parse the `{ … }` that follows a record's opening word, if any. An absent header
    /// is an empty object, not an error - `@question q/x {}` and `@question q/x` are the
    /// same record.
    fn header_object(&mut self, l: Line<'a>) -> Result<(HObject, Span), ()> {
        let Some(rel) = l.text.find('{') else {
            return Ok((HObject::new(), l.span));
        };
        let at = l.span.start + rel;
        match parse_object_prefix(&self.src[at..], at) {
            Ok(o) => Ok((o.value, o.span)),
            Err(e) => {
                self.err(
                    Code::E001,
                    Span::new(e.at, (e.at + 1).min(self.src.len())),
                    e.message,
                );
                Err(())
            }
        }
    }

    fn ref_list(&mut self, header: &mut HObject, key: &str) -> Vec<Spanned<Ref>> {
        let Some(v) = header.take(key) else {
            return Vec::new();
        };
        let Some(items) = v.value.as_array() else {
            self.err(Code::E001, v.span, format!("`{key}` must be an array"));
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|i| {
                let s = i.value.as_str()?;
                match parse_ref(s) {
                    Some(r) => Some(Spanned::new(r, i.span)),
                    None => {
                        self.out.diagnostics.push(
                            Diagnostic::at(Code::E001, i.span)
                                .with_message(format!("malformed reference `{s}`")),
                        );
                        None
                    }
                }
            })
            .collect()
    }

    fn id_list<T: Ord, E>(
        &mut self,
        v: &Spanned<HValue>,
        f: impl Fn(&str) -> Result<T, E>,
    ) -> BTreeSet<T> {
        v.value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| i.value.as_str().and_then(|s| f(s).ok()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn reference(&mut self, s: &str, span: Span) -> Option<Spanned<Ref>> {
        match parse_ref(s) {
            Some(r) => Some(Spanned::new(r, span)),
            None => {
                self.err(Code::E001, span, format!("malformed reference `{s}`"));
                None
            }
        }
    }

    fn source(&mut self, v: &Spanned<HValue>) -> Option<SourceRef> {
        let o = match v.value.as_object() {
            Some(o) => o,
            None => {
                self.err(Code::E001, v.span, "source must be an object");
                return None;
            }
        };
        let kind = o
            .get("kind")
            .and_then(|k| k.value.as_str())
            .and_then(SourceKind::parse);
        let reference = o
            .get("ref")
            .or_else(|| o.get("reference"))
            .and_then(|r| r.value.as_str());
        let (Some(kind), Some(reference)) = (kind, reference) else {
            self.err(Code::E001, v.span, "source needs `kind` and `ref`");
            return None;
        };
        let captured = o
            .get("captured")
            .and_then(|c| c.value.as_str())
            .and_then(|s| Date::parse(s).ok());
        Some(SourceRef {
            kind,
            reference: reference.to_string(),
            captured,
        })
    }

    // -----------------------------------------------------------------------
    // resolution
    // -----------------------------------------------------------------------

    fn finish(mut self) -> ParseOutcome {
        let index: BTreeMap<Label, usize> = self
            .units
            .iter()
            .enumerate()
            .filter_map(|(i, u)| u.label.clone().map(|l| (l, i)))
            .collect();

        let mut resolved: Vec<Option<Uid>> = vec![None; self.units.len()];
        let mut visiting = vec![false; self.units.len()];
        let mut cores: Vec<Option<UnitCore>> = vec![None; self.units.len()];
        let mut diagnostics = Vec::new();

        for i in 0..self.units.len() {
            resolve(
                i,
                &self.units,
                &index,
                &mut resolved,
                &mut visiting,
                &mut cores,
                &mut diagnostics,
            );
        }
        self.out.diagnostics.append(&mut diagnostics);

        let mut labels = BTreeMap::new();
        // A uid may be named twice — identity is content, so two declarations with the same
        // gist, status and grounds are one unit under two names. Surface syntax puts a label
        // on the unit declaration and has nowhere to put a second, so only one can be written
        // back. Keeping the first in canonical order, and having the writer keep the same
        // one, is what makes `parse -> write -> parse` a fixed point instead of a coin toss
        // over which name survives.
        //
        // Found by fuzzing: before `Record::LabelBinding` the loss was invisible, because
        // nothing carried a label through the wire to notice it going missing.
        // Collect first, then choose — because "first" must mean *canonical* order, not
        // document order. The writer keeps the smallest label for a uid, so a parser keeping
        // whichever appeared first in the file disagrees with it and the round trip swaps
        // names. That is this bug exactly, and my first fix reproduced it from the other side.
        let mut by_uid: BTreeMap<Uid, Vec<(Label, Span)>> = BTreeMap::new();
        for (i, u) in self.units.iter().enumerate() {
            if let (Some(l), Some(uid)) = (&u.label, resolved[i]) {
                by_uid.entry(uid).or_default().push((l.clone(), u.span));
            }
        }
        for (uid, mut names) in by_uid {
            names.sort_by(|a, b| a.0.cmp(&b.0));
            let keeper = names[0].0.clone();
            for (l, span) in names.into_iter().skip(1) {
                if l == keeper {
                    continue;
                }
                self.out
                    .diagnostics
                    .push(Diagnostic::at(Code::W054, span).with_message(format!(
                        "`{l}` names the same unit as `{keeper}`; only `{keeper}` survives \
                         a round trip, because a unit carries one name"
                    )));
            }
            labels.insert(keeper, uid);
        }

        let lookup = |r: &Ref, out: &mut ParseOutcome, span: Span| -> Option<Uid> {
            match r {
                Ref::Uid(u) => Some(*u),
                Ref::Label(l) => match labels.get(l) {
                    Some(u) => Some(*u),
                    None => {
                        out.diagnostics.push(
                            Diagnostic::at(Code::E060, span)
                                .with_message(format!("unresolved reference `{l}`")),
                        );
                        None
                    }
                },
            }
        };

        for (i, core) in cores.into_iter().enumerate() {
            if let Some(c) = core {
                if let (Some(s), Some(uid)) = (self.units[i].salience, resolved[i]) {
                    self.out
                        .salience
                        .insert(uid, crate::types::quantise(s.clamp(0.0, 1.0)));
                }
                self.out.records.push(Record::Unit(c));
            }
        }

        // Emit the bindings as records, so a label reaches the log rather than living only
        // in `ParseOutcome`. Everything downstream - the store, `merge`, `log_bytes` - then
        // carries labels for free, which is what it means for a label to survive a round
        // trip. They sit after the units they name so a reader meets the unit first.
        for (label, uid) in &labels {
            self.out
                .records
                .push(Record::LabelBinding(crate::types::LabelBinding::new(
                    label.clone(),
                    *uid,
                )));
        }

        for r in &self.relations {
            let (Some(from), Some(to)) = (
                lookup(&r.from.value, &mut self.out, r.span),
                lookup(&r.to.value, &mut self.out, r.span),
            ) else {
                continue;
            };
            let mut rel = Relation::new(r.kind.clone(), from, to);
            if let Some(w) = r.weight {
                rel = rel.with_weight(w);
            }
            if let Some(n) = &r.note {
                if let Some(u) = lookup(&n.value, &mut self.out, r.span) {
                    rel = rel.with_note(u);
                }
            }
            self.out.records.push(Record::Relation(rel));
        }

        for t in &self.threads {
            let steps: Vec<Step> = t
                .steps
                .iter()
                .filter_map(|(role, r, note)| {
                    let uid = lookup(&r.value, &mut self.out, t.span)?;
                    let mut s = Step::new(*role, uid);
                    s.note = note.clone();
                    Some(s)
                })
                .collect();
            self.out.records.push(Record::Thread(
                Thread::new(
                    t.id.clone(),
                    t.schema,
                    t.owner.clone(),
                    t.gist.clone(),
                    t.ts.clone(),
                )
                .with_steps(steps),
            ));
        }

        if let Some(v) = &self.view {
            let roots: BTreeSet<Uid> = v
                .roots
                .iter()
                .filter_map(|r| lookup(&r.value, &mut self.out, r.span))
                .collect();
            let view = View {
                id: v.id.clone(),
                roots,
                threads: v.threads.clone(),
                requires: v.requires.clone(),
                granularity: v.granularity.clone(),
                intent: v.intent.clone(),
                lang: v.lang.clone(),
                extra: Default::default(),
            };
            // The view is a record like any other, so a store built from a parse knows
            // the granularity its units were produced under.
            self.out.records.push(Record::View(view.clone()));
            self.out.view = Some(view);
        }

        self.out.labels = labels;
        self.out.diagnostics.sort();
        self.out
    }
}

/// Resolve one unit's uid, recursing into whatever it references.
///
/// A unit's uid depends on the uids of its deps and grounds, so resolution is a
/// depth-first walk in dependency order. A cycle is `SMY-E061`; an unresolvable label is
/// `SMY-E060`. Both drop the offending reference and keep the unit, because a corpus with
/// one broken edge is usable and a refused parse is not (rule I's spirit, applied here).
#[allow(clippy::too_many_arguments)]
fn resolve(
    i: usize,
    units: &[RawUnit],
    index: &BTreeMap<Label, usize>,
    resolved: &mut Vec<Option<Uid>>,
    visiting: &mut Vec<bool>,
    cores: &mut Vec<Option<UnitCore>>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Uid> {
    if let Some(u) = resolved[i] {
        return Some(u);
    }
    if visiting[i] {
        diagnostics.push(
            Diagnostic::at(Code::E061, units[i].span)
                .with_message("cycle in deps or grounds; the back edge is dropped"),
        );
        return None;
    }
    visiting[i] = true;

    let set = |refs: &[Spanned<Ref>],
               resolved: &mut Vec<Option<Uid>>,
               visiting: &mut Vec<bool>,
               cores: &mut Vec<Option<UnitCore>>,
               diagnostics: &mut Vec<Diagnostic>|
     -> BTreeSet<Uid> {
        let mut out = BTreeSet::new();
        for r in refs {
            let uid = match &r.value {
                Ref::Uid(u) => Some(*u),
                Ref::Label(l) => match index.get(l) {
                    Some(&j) => resolve(j, units, index, resolved, visiting, cores, diagnostics),
                    None => {
                        diagnostics.push(
                            Diagnostic::at(Code::E060, r.span)
                                .with_message(format!("unresolved reference `{l}`")),
                        );
                        None
                    }
                },
            };
            if let Some(u) = uid {
                out.insert(u);
            }
        }
        out
    };

    let deps = set(&units[i].deps, resolved, visiting, cores, diagnostics);
    let grounds = set(&units[i].grounds, resolved, visiting, cores, diagnostics);
    visiting[i] = false;

    let u = &units[i];
    let mut b = UnitCoreBuilder::new(u.schema.clone(), u.gist.clone(), u.status);
    b.body = u.body.clone();
    b.detail = u.detail.clone();
    b.deps = deps;
    b.grounds = grounds;
    b.source = u.source.clone();
    b.payload = u.payload.clone();

    match UnitCore::new(b) {
        Ok(core) => {
            let uid = canonical_uid(&core);
            resolved[i] = Some(uid);
            cores[i] = Some(core);
            Some(uid)
        }
        Err(e) => {
            diagnostics.push(
                Diagnostic::at(e.code(), u.span)
                    .with_subject(Subject::Span(u.span))
                    .with_message(e.to_string()),
            );
            None
        }
    }
}
