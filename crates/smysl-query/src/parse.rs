//! Recursive descent over RFC SMYSL-2.9 §4.
//!
//! Hand-written, for SMYSL-2.5 §3.1's four reasons, of which the one that shapes the code is
//! that this grammar is reimplemented in Python, Go and JavaScript: every function below is a
//! production, so a port is a transliteration rather than a redesign.
//!
//! **Stage order is enforced here, not validated afterwards** (§4.2). Each stage class carries
//! a rank, and a stage whose rank has already been passed is refused with a message naming both
//! stages. That is what lets the planner's phase order (SMYSL-2.5 §3.2) be the written order,
//! and it is also why no production ever backtracks.
//!
//! **A stage break inside an incomplete construct is whitespace.** A newline or a `|` ends a
//! stage, but only where a stage could end: after `and`, a comma or an operator the parser is
//! waiting on an operand, so a long `where` may be wrapped across lines without `|`. The rule
//! is the one Go and Swift use for semicolons, and it is what keeps "a separator is required"
//! (OQ-75) from meaning "a stage must fit on one line".

use crate::ast::*;
use crate::lex::{Lexer, Tok, Token};
use smysl_core::diag::{Code, Diagnostic, Span};

/// A parse: the tree, and the warnings the parse itself produced.
///
/// Warnings are returned rather than logged because the only one a parse can raise — `W510`,
/// a literal that is not in NFC — is about a decision the caller may want to act on before
/// running anything (§4.1, QG-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    pub query: Query,
    pub warnings: Vec<Diagnostic>,
}

/// Stage ranks. The numbers are SMYSL-2.9 §4.2's class order, which is SMYSL-2.5 §3.2's
/// planner phase order.
const R_SCOPE: u8 = 1;
const R_SOURCE: u8 = 2;
const R_FILTER: u8 = 3;
const R_TIME: u8 = 4;
const R_TRAVERSAL: u8 = 5;
const R_CLOSURE: u8 = 6;

pub fn parse(src: &str) -> Result<Parsed, Diagnostic> {
    let mut p = Parser {
        lx: Lexer::new(src),
        warnings: Vec::new(),
    };
    let query = p.query()?;
    Ok(Parsed {
        query,
        warnings: p.warnings,
    })
}

struct Parser<'a> {
    lx: Lexer<'a>,
    warnings: Vec<Diagnostic>,
}

impl Parser<'_> {
    // ---- helpers ---------------------------------------------------------------------------

    fn peek(&mut self) -> Result<Token, Diagnostic> {
        self.lx.peek()
    }

    fn bump(&mut self) -> Result<Token, Diagnostic> {
        self.lx.advance()
    }

    /// Consume a stage break if one is next. Used at continuation points, where a newline is
    /// whitespace rather than a separator.
    fn soft_break(&mut self) -> Result<(), Diagnostic> {
        while matches!(self.peek()?.tok, Tok::Break) {
            self.bump()?;
        }
        Ok(())
    }

    /// The word a stage would start with, looking past any break. Does not consume.
    fn head(&mut self) -> Result<Option<String>, Diagnostic> {
        let t = self.peek()?;
        if matches!(t.tok, Tok::Break) {
            // One `Break` token stands for a whole run of newlines and `|`, so looking past it
            // needs no loop: take it and let the next peek fill.
            self.bump()?;
            return self.head();
        }
        Ok(t.word())
    }

    /// Require that a stage has ended: a break, `)`, a set operator, or the end of input.
    fn end_of_stage(&mut self, just_parsed: &str) -> Result<(), Diagnostic> {
        let t = self.peek()?;
        match &t.tok {
            Tok::Break | Tok::Eof => Ok(()),
            Tok::Punct(")") => Ok(()),
            _ => {
                let what = t
                    .word()
                    .map(|w| format!("`{w}`"))
                    .unwrap_or_else(|| "this".to_string());
                Err(self.lx.err(
                    t.span,
                    format!(
                        "{what} begins a new stage on the same line as `{just_parsed}`; \
                         separate them with a newline or `|`"
                    ),
                ))
            }
        }
    }

    fn expect_word(&mut self, want: &str) -> Result<Token, Diagnostic> {
        let t = self.bump()?;
        match t.word() {
            Some(w) if w == want => Ok(t),
            _ => Err(self.lx.err(t.span, format!("expected `{want}`"))),
        }
    }

    fn expect(&mut self, want: &str) -> Result<Token, Diagnostic> {
        let t = self.bump()?;
        if t.is(want) {
            Ok(t)
        } else {
            Err(self.lx.err(t.span, format!("expected `{want}`")))
        }
    }

    fn eat_word(&mut self, want: &str) -> Result<bool, Diagnostic> {
        if self.peek()?.word().as_deref() == Some(want) {
            self.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn string_or_param(&mut self) -> Result<Atom, Diagnostic> {
        let t = self.bump()?;
        match t.tok {
            Tok::Str { value, nfc } => {
                if !nfc {
                    self.warnings
                        .push(Diagnostic::at(Code::W510, t.span).with_message(
                            "this string is not in NFC; it is matched byte-exactly as written \
                             and may match nothing",
                        ));
                }
                Ok(Atom::Str(value))
            }
            Tok::Param(name) => Ok(Atom::Param(name)),
            _ => Err(self.lx.err(t.span, "expected a string or a `$parameter`")),
        }
    }

    fn plain_string(&mut self) -> Result<String, Diagnostic> {
        match self.string_or_param()? {
            Atom::Str(s) => Ok(s),
            Atom::Param(_) => {
                let at = self.lx.here();
                Err(self
                    .lx
                    .err(Span::new(at, at), "a `$parameter` is not allowed here"))
            }
        }
    }

    fn count(&mut self) -> Result<Count, Diagnostic> {
        let t = self.bump()?;
        match t.tok {
            Tok::Int(n) => Ok(Count::Int(n)),
            Tok::Param(p) => Ok(Count::Param(p)),
            _ => Err(self.lx.err(t.span, "expected a whole number")),
        }
    }

    /// A run of `ident` joined by `.`, `/` and `:` with no whitespace between them.
    ///
    /// One scan serves field names, schema ids and view ids, because the three differ only in
    /// which joiners they may use, and a joiner the target forbids is a better diagnostic than
    /// a parse failure one token earlier.
    fn dotted(&mut self) -> Result<(String, Span), Diagnostic> {
        let first = self.bump()?;
        let mut out = match first.word() {
            Some(_) => match first.tok {
                Tok::Word(ref w) => w.clone(),
                _ => unreachable!("word() was Some"),
            },
            None => return Err(self.lx.err(first.span, "expected a name")),
        };
        let start = first.span.start;
        let mut end = first.span.end;
        loop {
            let t = self.peek()?;
            let joiner = match &t.tok {
                Tok::Punct(p @ ("." | "/" | ":")) if !t.ws_before => *p,
                _ => break,
            };
            self.bump()?;
            let seg = self.bump()?;
            match (&seg.tok, seg.ws_before) {
                (Tok::Word(w), false) => {
                    out.push_str(joiner);
                    out.push_str(w);
                    end = seg.span.end;
                }
                (Tok::Int(n), false) => {
                    out.push_str(joiner);
                    out.push_str(&n.to_string());
                    end = seg.span.end;
                }
                _ => {
                    return Err(self
                        .lx
                        .err(seg.span, format!("expected a name after `{joiner}`")))
                }
            }
        }
        Ok((out, Span::new(start, end)))
    }

    fn field_ref(&mut self) -> Result<FieldRef, Diagnostic> {
        let (path, span) = self.dotted()?;
        let t = self.peek()?;
        if t.is("@") && !t.ws_before {
            self.bump()?;
            let rev = self.bump()?;
            let revision = match rev.tok {
                Tok::Int(n) if n >= 0 => n as u32,
                _ => {
                    return Err(self
                        .lx
                        .err(rev.span, "expected a revision number after `@`"))
                }
            };
            self.expect(":")?;
            let (name, nspan) = self.dotted()?;
            return Ok(FieldRef {
                pin: Some(SchemaPin { id: path, revision }),
                name,
                span: span.join(nspan),
            });
        }
        if path.contains('/') {
            return Err(self.lx.err(
                span,
                "a field name may not contain `/`; qualify it as `schema@revision:field`",
            ));
        }
        Ok(FieldRef {
            pin: None,
            name: path,
            span,
        })
    }

    // ---- query -----------------------------------------------------------------------------

    fn query(&mut self) -> Result<Query, Diagnostic> {
        let mut header = Vec::new();
        loop {
            self.soft_break()?;
            if self.peek()?.word().as_deref() != Some("using") {
                break;
            }
            header.push(self.using()?);
        }

        let expr = self.expr()?;

        let mut ordering = Vec::new();
        let mut shaping = Vec::new();
        let mut output = None;
        while let Some(w) = self.head()? {
            match w.as_str() {
                "order" | "in" | "group" | "compare" | "align" => {
                    if !shaping.is_empty() || output.is_some() {
                        let t = self.peek()?;
                        return Err(self.lx.err(
                            t.span,
                            format!("`{w}` orders a result and must come before `limit`, `pack` and the output stage"),
                        ));
                    }
                    ordering.push(self.ordering()?);
                }
                "limit" | "pack" => {
                    if output.is_some() {
                        let t = self.peek()?;
                        return Err(self
                            .lx
                            .err(t.span, format!("`{w}` must come before the output stage")));
                    }
                    shaping.push(self.shaping()?);
                }
                "count" | "units" | "json" | "show" | "spans" | "measure" => {
                    if output.is_some() {
                        let t = self.peek()?;
                        return Err(self.lx.err(t.span, "a query has one output stage"));
                    }
                    output = Some(self.output()?);
                }
                _ => break,
            }
        }

        self.soft_break()?;
        let t = self.peek()?;
        if !matches!(t.tok, Tok::Eof) {
            let what = t
                .word()
                .map(|w| format!("`{w}`"))
                .unwrap_or_else(|| "this".to_string());
            return Err(self
                .lx
                .err(t.span, format!("{what} is not a stage `sq/1` knows")));
        }

        Ok(Query {
            header,
            expr,
            ordering,
            shaping,
            output,
            // Closed where the last token ended, not at `len`: a trailing newline, a trailing
            // separator or a trailing comment is not part of the query, and the printer emits
            // none of them.
            span: Span::new(0, self.lx.prev_end()),
        })
    }

    fn using(&mut self) -> Result<Using, Diagnostic> {
        let kw = self.expect_word("using")?;
        let (path, span) = self.dotted()?;
        if let Some(rest) = path.strip_prefix("sq/") {
            let version: u32 = rest.parse().map_err(|_| {
                Diagnostic::at(Code::E427, kw.span.join(span))
                    .with_message(format!("`sq/{rest}` is not a version this build supports"))
            })?;
            if version != 1 {
                return Err(
                    Diagnostic::at(Code::E427, kw.span.join(span)).with_message(format!(
                        "`sq/{version}` is not supported; this build speaks sq/1"
                    )),
                );
            }
            self.end_of_stage("using")?;
            return Ok(Using::Sq {
                version,
                span: kw.span.join(span),
            });
        }
        let t = self.peek()?;
        if !t.is("@") {
            return Err(self.lx.err(
                span,
                "a `using` clause names `sq/<version>` or `<schema>@<revision>`",
            ));
        }
        self.bump()?;
        let rev = self.bump()?;
        let revision = match rev.tok {
            Tok::Int(n) if n >= 0 => n as u32,
            _ => {
                return Err(self
                    .lx
                    .err(rev.span, "expected a revision number after `@`"))
            }
        };
        self.end_of_stage("using")?;
        Ok(Using::Schema {
            id: path,
            revision,
            span: kw.span.join(rev.span),
        })
    }

    fn expr(&mut self) -> Result<Expr, Diagnostic> {
        let mut left = self.term()?;
        while let Some(w) = self.head()? {
            let op = match w.as_str() {
                "union" => SetOp::Union,
                "intersect" => SetOp::Intersect,
                "except" => SetOp::Except,
                _ => break,
            };
            let kw = self.bump()?;
            self.soft_break()?;
            let right = self.term()?;
            let span = kw.span;
            left = Expr::Set {
                op,
                left: Box::new(left),
                right: Box::new(right),
                span,
            };
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Expr, Diagnostic> {
        self.soft_break()?;
        let t = self.peek()?;
        if t.is("(") {
            self.bump()?;
            let inner = self.expr()?;
            self.soft_break()?;
            self.expect(")")?;
            return Ok(inner);
        }
        Ok(Expr::Pipeline(self.pipeline()?))
    }

    // ---- pipeline --------------------------------------------------------------------------

    fn pipeline(&mut self) -> Result<Pipeline, Diagnostic> {
        let mut pl = Pipeline::default();
        let mut rank = 0u8;
        let mut last = String::new();

        while let Some(w) = self.head()? {
            let (this_rank, name) = match w.as_str() {
                "from" => (R_SCOPE, "from"),
                "find" | "about" | "text" | "uid" | "view" | "pointer" | "metric" | "question"
                | "match" => (R_SOURCE, "a source"),
                "where" => (R_FILTER, "where"),
                "said" | "observed" => (R_TIME, "a time stage"),
                "expand" => (R_TRAVERSAL, "expand"),
                "with" => (R_CLOSURE, "with"),
                _ => break,
            };
            if this_rank < rank {
                let t = self.peek()?;
                return Err(self.lx.err(
                    t.span,
                    format!(
                        "`{w}` may not follow `{last}`: a pipeline reads \
                         from, source, where, time, expand, with — in that order"
                    ),
                ));
            }
            if this_rank == R_SCOPE && pl.scope.is_some() {
                let t = self.peek()?;
                return Err(self.lx.err(
                    t.span,
                    "a pipeline has one `from`; name several scopes with `,`",
                ));
            }
            if this_rank == R_SOURCE && pl.source.is_some() {
                let t = self.peek()?;
                return Err(self.lx.err(
                    t.span,
                    "a pipeline has one source; combine two with `union`, `intersect` or `except`",
                ));
            }
            rank = this_rank;
            last = w.clone();

            match this_rank {
                R_SCOPE => pl.scope = Some(self.scope()?),
                R_SOURCE => pl.source = Some(self.source(&w)?),
                R_FILTER => pl.filters.push(self.filter()?),
                R_TIME => pl.times.push(self.time_stage()?),
                R_TRAVERSAL => pl.traversals.push(self.traversal()?),
                R_CLOSURE => pl.closures.push(self.closure()?),
                _ => unreachable!("ranks are the six above"),
            }
            self.end_of_stage(name)?;
        }

        if pl.is_empty() {
            let at = self.lx.here();
            return Err(self.lx.err(
                Span::new(at, at),
                "a query needs at least one stage: a scope, a source, or a refinement",
            ));
        }
        Ok(pl)
    }

    fn scope(&mut self) -> Result<Scope, Diagnostic> {
        let kw = self.expect_word("from")?;
        let mut items = Vec::new();
        loop {
            items.push(self.scope_item()?);
            if self.peek()?.is(",") {
                self.bump()?;
                self.soft_break()?;
                continue;
            }
            break;
        }
        let end = self.lx.prev_end();
        Ok(Scope {
            items,
            span: Span::new(kw.span.start, end),
        })
    }

    fn scope_item(&mut self) -> Result<ScopeItem, Diagnostic> {
        let t = self.bump()?;
        let Some(w) = t.word() else {
            return Err(self.lx.err(t.span, "expected a scope"));
        };
        let named = |p: &mut Self| -> Result<Name, Diagnostic> {
            p.expect(":")?;
            let n = p.bump()?;
            match n.tok {
                Tok::Word(w) => Ok(Name::Ident(w)),
                Tok::Str { value, .. } => Ok(Name::Str(value)),
                _ => Err(p.lx.err(n.span, "expected a name after `:`")),
            }
        };
        match w.as_str() {
            "store" => Ok(ScopeItem::Store(named(self)?)),
            "collection" => Ok(ScopeItem::Collection(named(self)?)),
            "import" => Ok(ScopeItem::Import(named(self)?)),
            "channel" => Ok(ScopeItem::Channel(named(self)?)),
            "channels" => Ok(ScopeItem::Channels),
            "library" => Ok(ScopeItem::Library),
            "texts" => Ok(ScopeItem::Texts),
            _ => Err(self.lx.err(
                t.span,
                format!(
                    "`{w}` is not a scope; `sq/1` has store:, collection:, import:, \
                     channel:, channels, library and texts"
                ),
            )),
        }
    }

    fn source(&mut self, w: &str) -> Result<SourceStage, Diagnostic> {
        let kw = self.bump()?;
        let source = self.source_kind(w, &kw)?;
        Ok(SourceStage {
            source,
            span: Span::new(kw.span.start, self.lx.prev_end()),
        })
    }

    fn source_kind(&mut self, w: &str, kw: &Token) -> Result<Source, Diagnostic> {
        match w {
            "find" => {
                let text = self.string_or_param()?;
                let mut engine = None;
                let mut lang = None;
                if self.peek()?.is("~") {
                    self.bump()?;
                    let e = self.bump()?;
                    engine = Some(match e.word().as_deref() {
                        Some("lex") => Engine::Lex,
                        Some("sem") => Engine::Sem,
                        Some("hybrid") => Engine::Hybrid,
                        _ => {
                            return Err(self
                                .lx
                                .err(e.span, "an engine is `~lex`, `~sem` or `~hybrid`"))
                        }
                    });
                }
                if self.peek()?.word().as_deref() == Some("lang") {
                    self.bump()?;
                    self.expect(":")?;
                    let l = self.bump()?;
                    lang = Some(match l.word().as_deref() {
                        Some("any") => LangSel::Any,
                        Some(_) => match l.tok {
                            Tok::Word(w) => LangSel::One(w),
                            _ => unreachable!("word() was Some"),
                        },
                        None => {
                            return Err(self.lx.err(l.span, "expected a language after `lang:`"))
                        }
                    });
                }
                Ok(Source::Find { text, engine, lang })
            }
            "about" => {
                let k = self.bump()?;
                match k.word().as_deref() {
                    Some("entity") => {
                        self.expect("(")?;
                        let a = self.string_or_param()?;
                        self.expect(")")?;
                        Ok(Source::AboutEntity(a))
                    }
                    Some("class") => {
                        self.expect("(")?;
                        let u = self.bump()?;
                        let Tok::Uid(uid) = u.tok else {
                            return Err(self.lx.err(u.span, "expected a `b3:` uid"));
                        };
                        self.expect(")")?;
                        Ok(Source::AboutClass(uid))
                    }
                    _ => Err(self
                        .lx
                        .err(k.span, "`about` takes `entity(\"…\")` or `class(b3:…)`")),
                }
            }
            "text" => {
                self.expect("(")?;
                let (raw, span) = self.lx.until_close_paren()?;
                self.expect(")")?;
                if let Some((alias, loc)) = raw.split_once('#') {
                    Ok(Source::Text {
                        alias: alias.trim().to_string(),
                        at: TextAt::Locator(loc.trim().to_string()),
                    })
                } else if let Some((alias, range)) = raw.split_once('@') {
                    Ok(Source::Text {
                        alias: alias.trim().to_string(),
                        at: TextAt::Range(range.trim().to_string()),
                    })
                } else {
                    Err(self
                        .lx
                        .err(span, "`text(…)` names `alias#locator` or `alias @ range`"))
                }
            }
            "uid" => {
                let t = self.bump()?;
                match t.tok {
                    Tok::Uid(u) => Ok(Source::Uid(Atom::Str(u))),
                    Tok::Param(p) => Ok(Source::Uid(Atom::Param(p))),
                    _ => Err(self
                        .lx
                        .err(t.span, "expected a `b3:` uid or a `$parameter`")),
                }
            }
            "view" => {
                let (path, span) = self.dotted()?;
                if !path.starts_with("v/") {
                    return Err(self.lx.err(span, "a view id begins `v/`"));
                }
                Ok(Source::View(path))
            }
            "pointer" | "metric" => {
                self.expect("(")?;
                let s = self.plain_string()?;
                self.expect(")")?;
                Ok(if w == "pointer" {
                    Source::Pointer(s)
                } else {
                    Source::Metric(s)
                })
            }
            "question" => Ok(Source::Question(self.string_or_param()?)),
            "match" => {
                let (raw, _span) = self.lx.balanced_to_stage_end()?;
                Ok(Source::Match { raw })
            }
            _ => Err(self.lx.err(kw.span, "expected a source")),
        }
    }

    // ---- filters ---------------------------------------------------------------------------

    fn filter(&mut self) -> Result<Filter, Diagnostic> {
        let kw = self.expect_word("where")?;
        let pred = self.pred()?;
        let lens_none = self.lens_modifier()?;
        let end = self.lx.prev_end();
        Ok(Filter {
            pred,
            lens_none,
            span: Span::new(kw.span.start, end),
        })
    }

    /// `with lens none`, which attaches to the four stages that can hold a qualified field
    /// reference (§4.7). It is recognised here, before the stage boundary, so `with` on the
    /// next line is still a closure stage.
    fn lens_modifier(&mut self) -> Result<bool, Diagnostic> {
        let t = self.peek()?;
        if matches!(t.tok, Tok::Break) || t.word().as_deref() != Some("with") {
            return Ok(false);
        }
        self.bump()?;
        self.expect_word("lens")?;
        self.expect_word("none")?;
        Ok(true)
    }

    fn pred(&mut self) -> Result<Pred, Diagnostic> {
        let first = self.unary()?;
        let Some(w) = self.peek()?.word() else {
            return Ok(first);
        };
        match w.as_str() {
            "and" => {
                let mut items = vec![first];
                while self.eat_word("and")? {
                    self.soft_break()?;
                    items.push(self.unary()?);
                    if self.peek()?.word().as_deref() == Some("or") {
                        let t = self.peek()?;
                        return Err(self.lx.err(
                            t.span,
                            "`and` and `or` may not be mixed without parentheses; \
                             write `(a and b) or c` or `a and (b or c)`",
                        ));
                    }
                }
                Ok(Pred::And(items))
            }
            "or" => {
                let mut items = vec![first];
                while self.eat_word("or")? {
                    self.soft_break()?;
                    items.push(self.unary()?);
                    if self.peek()?.word().as_deref() == Some("and") {
                        let t = self.peek()?;
                        return Err(self.lx.err(
                            t.span,
                            "`and` and `or` may not be mixed without parentheses; \
                             write `(a or b) and c` or `a or (b and c)`",
                        ));
                    }
                }
                Ok(Pred::Or(items))
            }
            _ => Ok(first),
        }
    }

    fn unary(&mut self) -> Result<Pred, Diagnostic> {
        self.soft_break()?;
        if self.eat_word("not")? {
            self.soft_break()?;
            return Ok(Pred::Not(Box::new(self.atom_pred()?)));
        }
        self.atom_pred()
    }

    fn atom_pred(&mut self) -> Result<Pred, Diagnostic> {
        let t = self.peek()?;
        if t.is("(") {
            self.bump()?;
            let inner = self.pred()?;
            self.soft_break()?;
            self.expect(")")?;
            return Ok(inner);
        }
        match t.word().as_deref() {
            Some("matches") => {
                self.bump()?;
                return Ok(Pred::Matches(self.string_or_param()?));
            }
            Some("near") => {
                self.bump()?;
                self.expect("(")?;
                let text = self.string_or_param()?;
                self.expect(",")?;
                let d = self.bump()?;
                let threshold = match d.tok {
                    Tok::Dec(s) => Decimal::from_scaled(s),
                    // Checked, like the lexer's own decimal path and with the same message: an
                    // integer that fits in `i64` need not fit once scaled, and `near("x", N)`
                    // for a large `N` was a panic rather than a diagnostic. Guarantee A1 is
                    // the whole point of this crate's fuzz target, which is what found it.
                    Tok::Int(n) => {
                        let scaled = n
                            .checked_mul(Decimal::SCALE)
                            .ok_or_else(|| self.lx.err(d.span, "number out of range"))?;
                        Decimal::from_scaled(scaled)
                    }
                    _ => return Err(self.lx.err(d.span, "expected a decimal threshold")),
                };
                self.expect(")")?;
                return Ok(Pred::Near { text, threshold });
            }
            _ => {}
        }
        let field = self.field_ref()?;
        let opt = self.bump()?;
        let op = match &opt.tok {
            Tok::Punct("=") => CmpOp::Eq,
            Tok::Punct("!=") => CmpOp::Ne,
            Tok::Punct("<") => CmpOp::Lt,
            Tok::Punct("<=") => CmpOp::Le,
            Tok::Punct(">") => CmpOp::Gt,
            Tok::Punct(">=") => CmpOp::Ge,
            Tok::Word(w) if w.eq_ignore_ascii_case("in") => CmpOp::In,
            _ => {
                return Err(self
                    .lx
                    .err(opt.span, "expected a comparison: =, !=, <, <=, >, >= or in"))
            }
        };
        let value = self.value()?;
        Ok(Pred::Cmp { field, op, value })
    }

    fn value(&mut self) -> Result<Value, Diagnostic> {
        let t = self.bump()?;
        match t.tok {
            Tok::Str { value, nfc } => {
                if !nfc {
                    self.warnings
                        .push(Diagnostic::at(Code::W510, t.span).with_message(
                            "this string is not in NFC; it is matched byte-exactly as written \
                             and may match nothing",
                        ));
                }
                Ok(Value::Str(value))
            }
            Tok::Int(n) => Ok(Value::Int(n)),
            Tok::Dec(s) => Ok(Value::Dec(Decimal::from_scaled(s))),
            Tok::Uid(u) => Ok(Value::Uid(u)),
            Tok::Param(p) => Ok(Value::Param(p)),
            Tok::Word(w) => Ok(Value::Word(w)),
            Tok::Punct("[") => {
                let mut items = Vec::new();
                loop {
                    self.soft_break()?;
                    if self.peek()?.is("]") {
                        self.bump()?;
                        break;
                    }
                    items.push(self.value()?);
                    self.soft_break()?;
                    if self.peek()?.is(",") {
                        self.bump()?;
                        continue;
                    }
                    self.expect("]")?;
                    break;
                }
                Ok(Value::List(items))
            }
            _ => Err(self.lx.err(t.span, "expected a value")),
        }
    }

    // ---- time, traversal, closure ----------------------------------------------------------

    fn time_stage(&mut self) -> Result<TimeStage, Diagnostic> {
        let kw = self.bump()?;
        let axis = match kw.word().as_deref() {
            Some("said") => TimeAxis::Said,
            Some("observed") => TimeAxis::Observed,
            _ => return Err(self.lx.err(kw.span, "expected `said` or `observed`")),
        };
        let o = self.bump()?;
        let op = match o.word().as_deref() {
            Some("during") => TimeOp::During,
            Some("overlaps") => TimeOp::Overlaps,
            Some("before") => TimeOp::Before,
            Some("after") => TimeOp::After,
            _ => {
                return Err(self
                    .lx
                    .err(o.span, "expected `during`, `overlaps`, `before` or `after`"))
            }
        };
        let (a, b, span) = self.lx.time_value()?;
        let value = match b {
            Some(b) => TimeValue::Range(a, b),
            None => TimeValue::At(a),
        };
        Ok(TimeStage {
            axis,
            op,
            value,
            span: kw.span.join(span),
        })
    }

    fn traversal(&mut self) -> Result<Traversal, Diagnostic> {
        let kw = self.expect_word("expand")?;
        let t = self.peek()?;
        let edge = if t.is("->") {
            self.bump()?;
            EdgeSpec::Out(self.dotted()?.0)
        } else if t.is("<-") {
            self.bump()?;
            EdgeSpec::In(self.dotted()?.0)
        } else if t.is("-[") {
            self.bump()?;
            let k = self.dotted()?.0;
            self.expect("]-")?;
            EdgeSpec::Both(k)
        } else {
            let n = self.bump()?;
            let named = match n.word().as_deref() {
                Some("grounds") => NamedEdge::Grounds,
                Some("deps") => NamedEdge::Deps,
                Some("dependents") => NamedEdge::Dependents,
                Some("lineage") => NamedEdge::Lineage,
                Some("mentions") => NamedEdge::Mentions,
                Some("rests") => {
                    self.expect_word("on")?;
                    NamedEdge::RestsOn
                }
                Some("rests_on") => {
                    return Err(self.lx.err(
                        n.span,
                        "write `rests on`; `rests_on` is a function name, not `sq/1`",
                    ))
                }
                _ => {
                    return Err(self.lx.err(
                        n.span,
                        "expected `->kind`, `<-kind`, `-[kind]-`, or one of \
                         grounds, deps, rests on, dependents, lineage, mentions",
                    ))
                }
            };
            EdgeSpec::Named(named)
        };
        let depth = if self.eat_word("depth")? {
            Some(self.count()?)
        } else {
            None
        };
        // `only` is last, after any depth, and nowhere else: one spelling, as everywhere in
        // sq/1 (QG-2). Nothing could follow a traversal on its line before, so recognising it
        // here takes no query that parsed away from its meaning (QG-9).
        let only = self.eat_word("only")?;
        if only {
            let t = self.peek()?;
            match t.word().as_deref() {
                Some("depth") => {
                    return Err(self.lx.err(
                        t.span,
                        "`only` comes last: write `expand <edge> depth <n> only`",
                    ))
                }
                Some("only") => {
                    return Err(self.lx.err(t.span, "`only` is said once"));
                }
                _ => {}
            }
        }
        let end = self.lx.prev_end();
        Ok(Traversal {
            edge,
            depth,
            only,
            span: Span::new(kw.span.start, end),
        })
    }

    fn closure(&mut self) -> Result<Closure, Diagnostic> {
        let kw = self.expect_word("with")?;
        let mut items = Vec::new();
        loop {
            let t = self.bump()?;
            let item = match t.word().as_deref() {
                Some("rebuttals") => ClosureItem::Rebuttals,
                Some("grounds") => ClosureItem::Grounds,
                Some("holders") => ClosureItem::Holders,
                Some("speakers") => ClosureItem::Speakers,
                Some("spans") => ClosureItem::Spans,
                Some("datings") => ClosureItem::Datings,
                Some("lens") => {
                    return Err(self.lx.err(
                        t.span,
                        "`with lens none` modifies the stage before it and belongs on that \
                         line, not on one of its own",
                    ))
                }
                _ => {
                    return Err(self.lx.err(
                        t.span,
                        "expected rebuttals, grounds, holders, speakers, spans or datings",
                    ))
                }
            };
            items.push(item);
            if self.peek()?.is(",") {
                self.bump()?;
                self.soft_break()?;
                continue;
            }
            break;
        }
        let end = self.lx.prev_end();
        Ok(Closure {
            items,
            span: Span::new(kw.span.start, end),
        })
    }

    // ---- ordering, shape, output -------------------------------------------------------------

    fn ordering(&mut self) -> Result<Ordering, Diagnostic> {
        let kw = self.bump()?;
        let out = match kw.word().as_deref() {
            Some("order") => {
                self.expect_word("by")?;
                let mut keys = Vec::new();
                loop {
                    let target = if self.peek()?.word().as_deref() == Some("salience") {
                        self.bump()?;
                        SortTarget::Salience
                    } else {
                        SortTarget::Field(self.field_ref()?)
                    };
                    let dir = match self.peek()?.word().as_deref() {
                        Some("ascending") => {
                            self.bump()?;
                            Some(Dir::Ascending)
                        }
                        Some("descending") => {
                            self.bump()?;
                            Some(Dir::Descending)
                        }
                        Some("asc") | Some("desc") => {
                            let t = self.peek()?;
                            let w = t.word().unwrap_or_default();
                            let full = if w == "asc" {
                                "ascending"
                            } else {
                                "descending"
                            };
                            return Err(self.lx.err(
                                t.span,
                                format!("write `{full}`; `sq/1` does not abbreviate"),
                            ));
                        }
                        _ => None,
                    };
                    keys.push(SortKey { target, dir });
                    if self.peek()?.is(",") {
                        self.bump()?;
                        self.soft_break()?;
                        continue;
                    }
                    break;
                }
                let lens_none = self.lens_modifier()?;
                Ordering::OrderBy {
                    keys,
                    lens_none,
                    span: Span::new(kw.span.start, self.lx.prev_end()),
                }
            }
            Some("in") => {
                let which = self.bump()?;
                match which.word().as_deref() {
                    Some("reading") => {
                        self.expect_word("order")?;
                        Ordering::ReadingOrder {
                            span: Span::new(kw.span.start, self.lx.prev_end()),
                        }
                    }
                    Some("time") => {
                        self.expect_word("order")?;
                        let as_recorded = if self.peek()?.word().as_deref() == Some("as") {
                            self.bump()?;
                            self.expect_word("recorded")?;
                            true
                        } else {
                            false
                        };
                        Ordering::TimeOrder {
                            as_recorded,
                            span: Span::new(kw.span.start, self.lx.prev_end()),
                        }
                    }
                    _ => {
                        return Err(self
                            .lx
                            .err(which.span, "expected `in reading order` or `in time order`"))
                    }
                }
            }
            Some("group") | Some("compare") => {
                let is_group = kw.word().as_deref() == Some("group");
                self.expect_word("by")?;
                let field = self.field_ref()?;
                let lens_none = self.lens_modifier()?;
                let span = Span::new(kw.span.start, self.lx.prev_end());
                if is_group {
                    Ordering::GroupBy {
                        field,
                        lens_none,
                        span,
                    }
                } else {
                    Ordering::CompareBy {
                        field,
                        lens_none,
                        span,
                    }
                }
            }
            Some("align") => {
                self.expect_word("by")?;
                self.expect_word("class")?;
                let policy = match self.peek()?.tok {
                    Tok::Word(_) => Some(self.dotted()?.0),
                    _ => None,
                };
                Ordering::AlignByClass {
                    policy,
                    span: Span::new(kw.span.start, self.lx.prev_end()),
                }
            }
            _ => return Err(self.lx.err(kw.span, "expected an ordering stage")),
        };
        self.end_of_stage("an ordering stage")?;
        Ok(out)
    }

    fn shaping(&mut self) -> Result<Shaping, Diagnostic> {
        let kw = self.bump()?;
        let out = match kw.word().as_deref() {
            Some("limit") => Shaping::Limit {
                count: self.count()?,
                span: Span::new(kw.span.start, self.lx.prev_end()),
            },
            Some("pack") => {
                let tokens = self.count()?;
                self.expect_word("tokens")?;
                let as_view = if self.peek()?.word().as_deref() == Some("as") {
                    self.bump()?;
                    self.expect_word("view")?;
                    true
                } else {
                    false
                };
                Shaping::Pack {
                    tokens,
                    as_view,
                    span: Span::new(kw.span.start, self.lx.prev_end()),
                }
            }
            _ => return Err(self.lx.err(kw.span, "expected `limit` or `pack`")),
        };
        self.end_of_stage("a shaping stage")?;
        Ok(out)
    }

    fn output(&mut self) -> Result<OutputStage, Diagnostic> {
        let kw = self.bump()?;
        let output = self.output_kind(&kw)?;
        Ok(OutputStage {
            output,
            span: Span::new(kw.span.start, self.lx.prev_end()),
        })
    }

    fn output_kind(&mut self, kw: &Token) -> Result<Output, Diagnostic> {
        let out = match kw.word().as_deref() {
            Some("count") => Output::Count,
            Some("units") => Output::Units,
            Some("json") => Output::Json,
            Some("show") => Output::Show,
            Some("spans") => Output::Spans,
            Some("measure") => {
                let mut ids = Vec::new();
                loop {
                    ids.push(self.dotted()?.0);
                    if self.peek()?.is(",") {
                        self.bump()?;
                        self.soft_break()?;
                        continue;
                    }
                    break;
                }
                Output::Measure(ids)
            }
            _ => return Err(self.lx.err(kw.span, "expected an output stage")),
        };
        self.end_of_stage("an output stage")?;
        Ok(out)
    }
}
