//! Completion, formatting and code actions (RFC SMYSL-6 §5.4–§5.6, LS-P3).
//!
//! Each answer is computed from the document's text and its parse, and each edit is a span of
//! the text and its replacement. The code actions are bounded by G4: one lowers a status to its
//! weakest ground's and never raises one, one corrects a label's spelling, and one declares an
//! extension relation kind. Nothing here deletes a relation or writes a `@resolve`.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::surface::{write_surface, ParseOutcome, WriteContext};
use smysl_core::{canonical_uid, KernelType, Record, RelKind, Span, Status, Uid, UnitCore};

use super::{Document, Encoding, Language, Range};

/// What a completion offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CompletionKind {
    /// A record type after `@`.
    Type,
    /// A header key.
    Key,
    /// A status.
    Status,
    /// A unit's label.
    Label,
    /// A relation kind.
    Kind,
    /// An `sq/1` keyword or field.
    Keyword,
}

/// One completion: the text it inserts over the word being typed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Completion {
    pub label: String,
    pub detail: String,
    pub kind: CompletionKind,
    /// The span the label replaces: the partial word before the cursor.
    pub replace: Range,
}

/// One edit to a document.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextEdit {
    pub range: Range,
    pub text: String,
}

/// A quick fix.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Action {
    pub title: String,
    /// The finding it answers, `SMY-E030`.
    pub code: String,
    pub edits: Vec<TextEdit>,
}

/// The five statuses an author may write: `unfounded` is reached only by retraction (`SMY-E034`).
const AUTHORED: [Status; 5] = [
    Status::Speculative,
    Status::Inferred,
    Status::Derived,
    Status::Cited,
    Status::Measured,
];

/// The header keys a unit takes (§5.4).
const KEYS: [&str; 6] = ["status", "grounds", "deps", "source", "lang", "salience"];

/// `sq/1`'s stage words and its kernel fields (RFC SMYSL-2.9; R1-9).
const SQ_WORDS: [&str; 24] = [
    "from",
    "find",
    "about",
    "text",
    "uid",
    "view",
    "pointer",
    "metric",
    "question",
    "match",
    "where",
    "said",
    "observed",
    "expand",
    "with",
    "order",
    "group",
    "by",
    "limit",
    "pack",
    "count",
    "union",
    "intersect",
    "using",
];
const SQ_FIELDS: [&str; 5] = ["kind", "schema", "status", "commitment", "lang"];

/// Whether `c` can be part of a word being completed: labels, kinds, schema ids.
fn word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '-' | '_')
}

/// The byte where the partial word before `at` starts.
fn word_start(text: &str, at: usize) -> usize {
    text[..at]
        .char_indices()
        .rev()
        .take_while(|(_, c)| word_char(*c))
        .last()
        .map_or(at, |(i, _)| i)
}

/// The labels of a parse, each with its unit.
fn labelled(out: &ParseOutcome) -> Vec<(String, Option<&UnitCore>)> {
    out.labels
        .iter()
        .map(|(l, u)| (l.to_string(), out.units().find(|x| canonical_uid(x) == *u)))
        .collect()
}

/// Completions at byte `at` of `doc` (§5.4).
pub(crate) fn complete(
    doc: &Document,
    out: Option<&ParseOutcome>,
    ws: Option<&super::Workspace>,
    at: usize,
    enc: Encoding,
) -> Vec<Completion> {
    let text = doc.text();
    let at = at.min(text.len());
    if !text.is_char_boundary(at) {
        return Vec::new();
    }
    let mut start = word_start(text, at);
    // In `@rel a --kin`, the word is what follows `--`, whose dashes `word_char` would take.
    let line_of_at = text[..at].rfind(['\n', '\r']).map_or(0, |i| i + 1);
    let typed = &text[line_of_at..at];
    if typed.starts_with("@rel") && !typed.contains("-->") {
        if let Some(d) = typed.rfind("--") {
            start = start.max(line_of_at + d + 2);
        }
    }
    let partial = &text[start..at];
    let replace = doc.range(Span::new(start, at), enc);
    let line_start = text[..start].rfind(['\n', '\r']).map_or(0, |i| i + 1);
    let before = &text[line_start..start];
    let mut items: Vec<(String, String, CompletionKind)> = Vec::new();

    if doc.language == Language::Query {
        for w in SQ_WORDS {
            items.push((w.into(), "stage".into(), CompletionKind::Keyword));
        }
        for f in SQ_FIELDS {
            items.push((f.into(), "field".into(), CompletionKind::Keyword));
        }
        if let Some(ws) = ws {
            for id in ws.schemas() {
                items.push((
                    id,
                    "schema in the workspace".into(),
                    CompletionKind::Keyword,
                ));
            }
        }
    } else if before.trim_start() == "@" {
        // `@` at the start of a line: a record.
        for t in KernelType::ALL {
            items.push((t.as_str().into(), "unit".into(), CompletionKind::Type));
        }
        for r in ["rel", "schema", "thread"] {
            items.push((r.into(), "record".into(), CompletionKind::Type));
        }
        if let Some(out) = out {
            for r in &out.records {
                if let Record::SchemaDecl(d) = r {
                    for t in &d.types {
                        items.push((
                            t.to_string(),
                            format!("declared in {}", d.id),
                            CompletionKind::Type,
                        ));
                    }
                }
            }
        }
    } else if before.trim_end().ends_with("status:") {
        for s in AUTHORED {
            items.push((s.as_str().into(), "status".into(), CompletionKind::Status));
        }
    } else if before.starts_with("@rel") && before.ends_with("--") && !before.contains("-->") {
        // Between `--` and `-->`: a relation kind.
        for k in RelKind::KERNEL {
            items.push((k.to_string(), "kernel kind".into(), CompletionKind::Kind));
        }
        if let Some(out) = out {
            for r in &out.records {
                if let Record::SchemaDecl(d) = r {
                    for k in &d.relations {
                        items.push((
                            k.to_string(),
                            format!("declared in {}", d.id),
                            CompletionKind::Kind,
                        ));
                    }
                }
            }
        }
    } else if inside_list(before) || before.starts_with("@rel") {
        if let Some(out) = out {
            for (l, u) in labelled(out) {
                let detail = u.map_or(String::new(), |u| format!("{} — {}", u.status, u.gist));
                items.push((l, detail, CompletionKind::Label));
            }
        }
    } else if inside_header(before) {
        for k in KEYS {
            items.push((k.into(), "header key".into(), CompletionKind::Key));
        }
    }
    let mut out: Vec<Completion> = items
        .into_iter()
        .filter(|(l, _, _)| l.starts_with(partial))
        .map(|(label, detail, kind)| Completion {
            label,
            detail,
            kind,
            replace,
        })
        .collect();
    out.sort();
    out.dedup_by(|a, b| a.label == b.label);
    out
}

/// Inside `grounds: [` or `deps: [`, not yet closed.
fn inside_list(before: &str) -> bool {
    ["grounds:", "deps:"].iter().any(|k| {
        before.rfind(k).is_some_and(|i| {
            let rest = &before[i + k.len()..];
            rest.trim_start().starts_with('[') && !rest.contains(']')
        })
    })
}

/// Inside a record header's braces, where a key goes: after `{` or `,`.
fn inside_header(before: &str) -> bool {
    before.starts_with('@')
        && before.matches('{').count() > before.matches('}').count()
        && before.trim_end().ends_with(['{', ','])
}

/// Why a document is not formatted, or its whole-text edit (§5.5). `Ok(None)` when it is
/// already canonical.
pub(crate) fn format(doc: &Document, enc: Encoding) -> Result<Option<TextEdit>, String> {
    let text = doc.text();
    let formatted = match doc.language {
        Language::Surface => {
            let out = smysl_core::surface::parse_surface(text).map_err(|e| e.to_string())?;
            if out.comments > 0 {
                return Err(format!(
                    "not formatted: {} comment line(s) are in no record and would be lost",
                    out.comments
                ));
            }
            if out.recovered > 0 || out.has_errors() {
                return Err(
                    "not formatted: the document has errors, and a record would be lost".into(),
                );
            }
            let ctx = WriteContext::from_labels(&out.labels)
                .with_salience(out.salience.clone())
                .with_format_version(out.format_version.clone());
            let formatted = write_surface(out.view.as_ref(), &out.records, &ctx);
            // The round trip must not move a uid, as `smysl fmt` checks.
            match smysl_core::surface::parse_surface(&formatted) {
                Ok(again) if again.labels == out.labels && again.records == out.records => {}
                _ => {
                    return Err(
                        "not formatted: the canonical form does not reproduce the records".into(),
                    )
                }
            }
            formatted
        }
        Language::Query => {
            // `#` opens a comment at the start of a token, and the printer discards comments.
            if text
                .lines()
                .any(|l| l.trim_start().starts_with('#') || l.contains(" #"))
            {
                return Err(
                    "not formatted: the query has a comment, which the canonical form drops".into(),
                );
            }
            let parsed = smysl_query::parse(text).map_err(|d| d.message)?;
            let mut p = smysl_query::print(&parsed.query);
            if !p.ends_with('\n') {
                p.push('\n');
            }
            p
        }
    };
    if formatted == text {
        return Ok(None);
    }
    Ok(Some(TextEdit {
        range: doc.range(Span::new(0, text.len()), enc),
        text: formatted,
    }))
}

/// Levenshtein distance, for "did you mean" (§5.6).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let sub = prev[j] + usize::from(ca != *cb);
            cur.push(sub.min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// The span of `status: X`'s value in a unit's header.
fn status_value(text: &str, record: Span) -> Option<Span> {
    let header_end = text[record.start..record.end]
        .find(['\n', '\r'])
        .map_or(record.end, |i| record.start + i);
    let head = &text[record.start..header_end];
    let k = head.find("status:")? + "status:".len();
    let rest = &head[k..];
    let lead = rest.len() - rest.trim_start().len();
    let len = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .count();
    (len > 0).then(|| {
        let s = record.start + k + lead;
        Span::new(s, s + len)
    })
}

/// The quick fixes whose finding lies in `[from, to)` (§5.6).
pub(crate) fn actions(
    doc: &Document,
    out: &ParseOutcome,
    from: usize,
    to: usize,
    enc: Encoding,
) -> Vec<Action> {
    let text = doc.text();
    let touches = |s: &Span| s.start <= to && from <= s.end;
    let mut acts = Vec::new();

    // SMY-E030: lower a unit's status to its weakest ground's. Only ever lower.
    let status_of: BTreeMap<Uid, Status> =
        out.units().map(|u| (canonical_uid(u), u.status)).collect();
    for (r, s) in out.records.iter().zip(&out.spans) {
        let (Record::Unit(u), Some(s)) = (r, s) else {
            continue;
        };
        // Rule M binds only what rests on grounds (`derived`, `inferred`): `measured` and
        // `cited` stand on their source, as `check` reads it.
        if !touches(s) || !u.status.is_rule_m_constrained() {
            continue;
        }
        let Some(weakest) = u
            .grounds
            .iter()
            .filter_map(|g| status_of.get(g))
            .min()
            .copied()
        else {
            continue;
        };
        let target = weakest.max(Status::Speculative);
        if u.status <= target {
            continue;
        }
        if let Some(v) = status_value(text, *s) {
            acts.push(Action {
                title: format!("Lower status to `{target}`, its weakest ground's"),
                code: "SMY-E030".into(),
                edits: vec![TextEdit {
                    range: doc.range(v, enc),
                    text: target.as_str().into(),
                }],
            });
        }
    }

    // SMY-E060: a label nobody declared, and a declared one within edit distance 2.
    let known: BTreeSet<String> = out.labels.keys().map(|l| l.to_string()).collect();
    for d in &out.diagnostics {
        if d.code != smysl_core::Code::E060 {
            continue;
        }
        let smysl_core::Subject::Span(s) = d.subject else {
            continue;
        };
        if !touches(&s) {
            continue;
        }
        let Some(name) = d.message.split('`').nth(1) else {
            continue;
        };
        let Some(at) = text[s.start..s.end.min(text.len())].find(name) else {
            continue;
        };
        let best = known
            .iter()
            .map(|k| (distance(name, k), k))
            .filter(|(n, _)| *n <= 2)
            .min();
        if let Some((_, k)) = best {
            let span = Span::new(s.start + at, s.start + at + name.len());
            acts.push(Action {
                title: format!("Replace `{name}` with `{k}`"),
                code: "SMY-E060".into(),
                edits: vec![TextEdit {
                    range: doc.range(span, enc),
                    text: k.clone(),
                }],
            });
        }
    }

    // SMY-W013: an extension relation kind nobody declared. Declare it, after the `@doc` header.
    let declared: BTreeSet<String> = out
        .records
        .iter()
        .filter_map(|r| match r {
            Record::SchemaDecl(d) => Some(
                d.relations
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect();
    // `check` reports it (`SMY-W013`) against the relation record, so it is found here from the
    // relation records themselves, each placed by its span.
    let mut offered = BTreeSet::new();
    for (r, s) in out.records.iter().zip(&out.spans) {
        let (Record::Relation(x), Some(s)) = (r, s) else {
            continue;
        };
        let RelKind::Extension(kind) = &x.kind else {
            continue;
        };
        if !touches(s) || declared.contains(kind.as_str()) {
            continue;
        }
        let Some((domain, _)) = kind.strip_prefix("x.").and_then(|k| k.split_once('/')) else {
            continue;
        };
        if !offered.insert(kind.clone()) {
            continue;
        }
        let insert_at = after_doc_header(text);
        let decl = format!("@schema x.{domain}/1 {{ version: 1, relations: [\"{kind}\"] }}\n\n");
        acts.push(Action {
            title: format!("Declare `{kind}` in an `@schema`"),
            code: "SMY-W013".into(),
            edits: vec![TextEdit {
                range: doc.range(Span::new(insert_at, insert_at), enc),
                text: decl,
            }],
        });
    }
    acts.sort();
    acts
}

/// The byte after the `@doc` header block and the blank lines after it; 0 without one.
fn after_doc_header(text: &str) -> usize {
    let Some(at) = text.find("@doc") else {
        return 0;
    };
    let mut depth = 0i32;
    let mut end = text.len();
    for (i, c) in text[at..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = at + i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    let rest = &text[end..];
    end + (rest.len() - rest.trim_start_matches(['\n', '\r', ' ', '\t']).len())
}

#[cfg(test)]
mod tests {
    use super::super::{AnalysisHost, LineIndex, Position};
    use super::*;

    const URI: &str = "file:///e.smy";

    fn host(text: &str) -> AnalysisHost {
        let mut h = AnalysisHost::new(Encoding::Utf16);
        h.open(URI, Language::Surface, 1, text.to_string());
        h
    }

    /// The position just after the first occurrence of `needle`.
    fn after(text: &str, needle: &str) -> Position {
        let at = text.find(needle).unwrap() + needle.len();
        LineIndex::new(text).position(text, at, Encoding::Utf16)
    }

    fn apply(text: &str, edits: &[TextEdit]) -> String {
        let ix = LineIndex::new(text);
        let mut spans: Vec<(usize, usize, &str)> = edits
            .iter()
            .map(|e| {
                (
                    ix.offset(text, e.range.start, Encoding::Utf16),
                    ix.offset(text, e.range.end, Encoding::Utf16),
                    e.text.as_str(),
                )
            })
            .collect();
        spans.sort_by_key(|a| std::cmp::Reverse(a.0));
        let mut out = text.to_string();
        for (a, b, t) in spans {
            out.replace_range(a..b, t);
        }
        out
    }

    const DOC: &str = "@doc smysl/0.1 { id: v/e }\n\n\
        @evidence e/a { status: speculative }\n~ A guess.\n\n\
        @claim c/b { status: derived, grounds: [e/a] }\n~ Over-claimed.\n\n\
        @claim c/c { status: inferred, grounds: [e/aa] }\n~ A typo.\n\n\
        @rel c/b --x.ops/blocks--> e/a\n";

    #[test]
    fn completion_by_context() {
        let labels = |text: &str, needle: &str| -> Vec<String> {
            host(text)
                .completion(URI, after(text, needle))
                .into_iter()
                .map(|c| c.label)
                .collect()
        };
        let t = "@doc smysl/0.1 { id: v/e }\n\n@cl";
        assert_eq!(labels(t, "@cl"), vec!["claim"]);
        let t = "@doc smysl/0.1 { id: v/e }\n\n@claim c/x { status: d";
        assert_eq!(labels(t, "status: d"), vec!["derived"]);
        let s = labels(DOC, "grounds: [");
        assert_eq!(
            s,
            vec!["c/b", "e/a"],
            "c/c failed to build: E060 aside, its grounds held"
        );
        let t = format!("{DOC}@rel c/b --reb");
        assert_eq!(labels(&t, "--reb"), vec!["rebuts"]);
        let t = format!("{DOC}@rel c/b --x.");
        assert!(
            labels(&t, "@rel c/b --x.").is_empty(),
            "x.ops/blocks is not declared"
        );
        let t = "@doc smysl/0.1 { id: v/e }\n\n@claim c/x { status: derived, gr";
        assert_eq!(labels(t, ", gr"), vec!["grounds"]);
        let mut q = AnalysisHost::new(Encoding::Utf16);
        q.open(
            "file:///q.sq",
            Language::Query,
            1,
            "from library\nwh".into(),
        );
        let got: Vec<String> = q
            .completion("file:///q.sq", Position::new(1, 2))
            .into_iter()
            .map(|c| c.label)
            .collect();
        assert_eq!(got, vec!["where"]);
    }

    #[test]
    fn the_three_quick_fixes_and_what_they_fix() {
        let h = host(DOC);
        let all = h.actions(URI, Range::new(Position::new(0, 0), Position::new(99, 0)));
        let titles: Vec<&str> = all.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(
            titles,
            vec![
                "Declare `x.ops/blocks` in an `@schema`",
                "Lower status to `speculative`, its weakest ground's",
                "Replace `e/aa` with `e/a`",
            ]
        );
        assert_eq!(all.len(), 3, "{titles:?}");
        for a in &all {
            let fixed = apply(DOC, &a.edits);
            let codes: Vec<String> = host(&fixed)
                .diagnostics(URI)
                .into_iter()
                .map(|d| d.code)
                .collect();
            let still = codes.iter().filter(|c| **c == a.code).count();
            let before = host(DOC)
                .diagnostics(URI)
                .into_iter()
                .filter(|d| d.code == a.code)
                .count();
            assert!(still < before, "{}: {codes:?}", a.title);
        }
    }

    #[test]
    fn formatting_refuses_to_lose_a_comment_or_a_record() {
        let clean =
            "@doc smysl/0.1 { id: v/f }\n\n@claim c/a   { status: speculative }\n~ Spaced out.\n";
        let h = host(clean);
        let e = h.format(URI).unwrap().expect("a change");
        let again = apply(clean, &[e]);
        assert!(
            host(&again).format(URI).unwrap().is_none(),
            "canonical is a fixed point"
        );
        let commented = format!("{clean}# a reviewer's note\n");
        assert!(host(&commented)
            .format(URI)
            .unwrap_err()
            .contains("comment"));
        let broken =
            "@doc smysl/0.1 { id: v/f }\n\n@claim c/a { status: derived }\n~ No grounds.\n";
        assert!(host(broken).format(URI).is_err());
    }

    /// splitmix64: generated documents, the same ones every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    fn generate(r: &mut Rng) -> String {
        let n = 2 + r.below(5) as usize;
        let mut t = String::from("@doc smysl/0.1 { id: v/g }\n\n");
        let statuses = ["speculative", "inferred", "derived", "cited", "measured"];
        for i in 0..n {
            if r.below(4) == 0 {
                t.push_str("# a note\n");
            }
            let status = statuses[r.below(5) as usize];
            let mut head = format!("@claim c/u{i} {{ status: {status}");
            if matches!(status, "cited" | "measured") {
                head.push_str(", source: { kind: doc, ref: \"r\" }");
            }
            if i > 0 && (r.below(3) > 0 || matches!(status, "derived" | "inferred")) {
                let g = r.below(i as u64);
                let name = if r.below(6) == 0 {
                    format!("c/u{g}x")
                } else {
                    format!("c/u{g}")
                };
                head.push_str(&format!(", grounds: [{name}]"));
            }
            t.push_str(&format!("{head} }}\n~ Unit {i}.\n\n"));
        }
        for _ in 0..r.below(3) {
            let (a, b) = (r.below(n as u64), r.below(n as u64));
            let kind = ["rebuts", "elaborates", "x.ops/blocks"][r.below(3) as usize];
            t.push_str(&format!("@rel c/u{a} --{kind}--> c/u{b}\n"));
        }
        t
    }

    fn statuses(text: &str) -> BTreeMap<String, Status> {
        let Ok(out) = smysl_core::surface::parse_surface(text) else {
            return BTreeMap::new();
        };
        out.labels
            .iter()
            .filter_map(|(l, u)| {
                out.units()
                    .find(|x| canonical_uid(x) == *u)
                    .map(|x| (l.to_string(), x.status))
            })
            .collect()
    }

    /// LS-P3's gate: over generated documents, no quick fix raises a status, deletes a
    /// rebuttal or writes a `@resolve`, and formatting never runs over a comment.
    #[test]
    fn no_action_raises_a_status_and_no_format_drops_a_comment() {
        let mut r = Rng(6);
        let (mut offered, mut formatted) = (0, 0);
        for _ in 0..400 {
            let text = generate(&mut r);
            let h = host(&text);
            let before = statuses(&text);
            let rebuts = text.matches("--rebuts-->").count();
            for a in h.actions(URI, Range::new(Position::new(0, 0), Position::new(999, 0))) {
                offered += 1;
                let fixed = apply(&text, &a.edits);
                for (label, s) in statuses(&fixed) {
                    if let Some(old) = before.get(&label) {
                        assert!(s <= *old, "{}: {label} {old} -> {s}\n{text}", a.title);
                    }
                }
                assert_eq!(fixed.matches("--rebuts-->").count(), rebuts, "{}", a.title);
                assert!(!fixed.contains("@resolve"), "{}", a.title);
            }
            match h.format(URI) {
                Ok(Some(e)) => {
                    formatted += 1;
                    assert!(
                        !text.contains("# a note"),
                        "formatted over a comment:\n{text}"
                    );
                    let after = apply(&text, &[e]);
                    assert_eq!(statuses(&after), before);
                }
                Ok(None) => {}
                Err(_) => {}
            }
        }
        assert!(
            offered > 100,
            "the generator reaches the actions: {offered}"
        );
        assert!(formatted > 20, "and the formatter: {formatted}");
    }
}
