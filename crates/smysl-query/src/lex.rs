//! A mode-aware lexer for `sq/1` (RFC SMYSL-2.9 §4.1).
//!
//! Pull-based rather than a token vector, because three constructs cannot be lexed without
//! knowing what the parser is in the middle of:
//!
//! - a **time value** (`2026-09-01..2026-09-15`) begins with a digit and holds `-`, `:` and
//!   `T`, so a general tokeniser would shred it into arithmetic-looking pieces;
//! - a **locator** (`text(kjv-1769#Ps.14)`) keeps its `#`, which is a comment anywhere else;
//! - a **`match` pattern** is captured whole and not interpreted at all (§4.9).
//!
//! So the parser asks for the mode it needs. SMYSL-2.5 §3.1 gives the same three reasons for a
//! hand-written lexer over a generated one.
//!
//! Two rules in here are about the *other three implementations* of this grammar rather than
//! this one. Keywords fold with ASCII-only lowercasing (QG-4): all four runtimes case-fold
//! independently of locale, so that is not the hazard — the hazard is that Unicode's folding
//! tables change between Unicode versions and four runtimes will not ship the same one, which
//! would let a field name collide with a keyword in one port and not another. And a string
//! literal's bytes are kept exactly (QG-5), with `SMY-W510` when they are not in NFC.

use smysl_core::diag::{Code, Diagnostic, Span};

/// What a token is. Punctuation carries its own spelling so a parser can match on it directly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Tok {
    /// An ASCII identifier. Keyword recognition is the parser's, after ASCII folding.
    Word(String),
    Int(i64),
    /// A decimal, already scaled by `Decimal::SCALE`.
    Dec(i64),
    Str {
        value: String,
        /// False when the literal is not in NFC; the parser raises `SMY-W510` and keeps the
        /// bytes.
        nfc: bool,
    },
    Uid(String),
    Param(String),
    Punct(&'static str),
    /// A stage separator. `|` lexes as this too, so the parser has one thing to look for
    /// (SMYSL-2.9 OQ-75).
    Break,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub tok: Tok,
    pub span: Span,
    /// Whether whitespace preceded this token. Field names, schema ids and uids are built by
    /// joining adjacent tokens, and `text : mode` must not become `text:mode`.
    pub ws_before: bool,
}

impl Token {
    pub fn is(&self, p: &str) -> bool {
        matches!(self.tok, Tok::Punct(q) if q == p)
    }

    /// The ASCII-folded word, when this token is one (QG-4).
    pub fn word(&self) -> Option<String> {
        match &self.tok {
            Tok::Word(w) => Some(w.to_ascii_lowercase()),
            _ => None,
        }
    }
}

/// Punctuation, longest first so `<=` wins over `<` and `-[` over `-`.
const PUNCT: &[&str] = &[
    "!=", "<=", ">=", "<-", "->", "-[", "]-", "..", "=", "<", ">", "(", ")", "[", "]", ",", ":",
    ".", "/", "@", "~", "$",
];

pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    peeked: Option<Token>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str) -> Lexer<'a> {
        Lexer {
            src,
            pos: 0,
            peeked: None,
        }
    }

    pub fn here(&self) -> usize {
        self.peeked.as_ref().map_or(self.pos, |t| t.span.start)
    }

    pub fn peek(&mut self) -> Result<Token, Diagnostic> {
        if self.peeked.is_none() {
            self.peeked = Some(self.scan()?);
        }
        Ok(self.peeked.clone().expect("just filled"))
    }

    /// Named `advance` rather than `next`: this is not an iterator, and a `next` that
    /// returns a `Result` and never ends would read like one.
    pub fn advance(&mut self) -> Result<Token, Diagnostic> {
        match self.peeked.take() {
            Some(t) => Ok(t),
            None => self.scan(),
        }
    }

    /// Un-peek, so a raw-mode scan starts where the parser actually is. Every mode method
    /// calls this first: a peeked token has already consumed bytes that the raw scan must see.
    fn rewind(&mut self) {
        if let Some(t) = self.peeked.take() {
            self.pos = t.span.start;
        }
    }

    // ---- the three raw modes ---------------------------------------------------------------

    /// A time value: one EDTF string, or two around `..` (§4.6). The text is handed to
    /// SMYSL-2.4's EDTF parser later; this only decides where it ends.
    pub fn time_value(&mut self) -> Result<(String, Option<String>, Span), Diagnostic> {
        self.rewind();
        let start = self.skip_trivia_same_stage();
        let first = self.raw_until_range_or_space();
        if first.is_empty() {
            return Err(self.err(Span::new(start, start), "expected a time value"));
        }
        let save = self.pos;
        self.skip_blanks();
        if self.src[self.pos..].starts_with("..") {
            self.pos += 2;
            self.skip_blanks();
            let second = self.raw_until_range_or_space();
            if second.is_empty() {
                return Err(self.err(
                    Span::new(self.pos, self.pos),
                    "expected the second half of a time range after `..`",
                ));
            }
            let span = Span::new(start, self.pos);
            return Ok((first, Some(second), span));
        }
        self.pos = save;
        Ok((first, None, Span::new(start, self.pos)))
    }

    /// Everything to the matching `)`, kept verbatim. This is why `text(kjv-1769#Ps.14)` keeps
    /// its `#`: the general lexer never sees it.
    pub fn until_close_paren(&mut self) -> Result<(String, Span), Diagnostic> {
        self.rewind();
        let start = self.pos;
        let mut depth = 0usize;
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'(' => depth += 1,
                b')' if depth == 0 => {
                    let text = self.src[start..self.pos].trim().to_string();
                    return Ok((text, Span::new(start, self.pos)));
                }
                b')' => depth -= 1,
                b'\n' => break,
                _ => {}
            }
            self.pos += 1;
        }
        Err(self.err(Span::new(start, self.pos), "unclosed `(`"))
    }

    /// A `match` pattern: tokens to the end of the stage with `()`, `[]` and `{}` balanced,
    /// captured and not interpreted (§4.9). The planner refuses it with `SMY-E461`.
    pub fn balanced_to_stage_end(&mut self) -> Result<(String, Span), Diagnostic> {
        self.rewind();
        self.skip_blanks();
        let start = self.pos;
        let mut stack: Vec<u8> = Vec::new();
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            match b {
                b'(' | b'[' | b'{' => stack.push(b),
                b')' | b']' | b'}' => {
                    let want = match b {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    if stack.last() != Some(&want) {
                        return Err(self.err(
                            Span::new(self.pos, self.pos + 1),
                            "unbalanced bracket in a `match` pattern",
                        ));
                    }
                    stack.pop();
                }
                b'\n' if stack.is_empty() => break,
                b'|' if stack.is_empty() => break,
                _ => {}
            }
            self.pos += 1;
        }
        if !stack.is_empty() {
            return Err(self.err(
                Span::new(start, self.pos),
                "unclosed bracket in a `match` pattern",
            ));
        }
        let text = self.src[start..self.pos].trim_end().to_string();
        if text.is_empty() {
            return Err(self.err(
                Span::new(start, self.pos),
                "expected a pattern after `match`",
            ));
        }
        let span = Span::new(start, start + text.len());
        Ok((text, span))
    }

    // ---- the general mode ------------------------------------------------------------------

    fn scan(&mut self) -> Result<Token, Diagnostic> {
        let (ws, saw_break) = self.skip_trivia();
        let start = self.pos;
        if saw_break {
            return Ok(Token {
                tok: Tok::Break,
                span: Span::new(start, start),
                ws_before: ws,
            });
        }
        let bytes = self.src.as_bytes();
        if self.pos >= bytes.len() {
            return Ok(Token {
                tok: Tok::Eof,
                span: Span::new(start, start),
                ws_before: ws,
            });
        }
        let b = bytes[self.pos];
        let tok = if b == b'"' {
            self.scan_string()?
        } else if b == b'$' {
            self.pos += 1;
            let name = self.take_while(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
            if name.is_empty() {
                return Err(self.err(
                    Span::new(start, self.pos),
                    "expected a parameter name after `$`",
                ));
            }
            Tok::Param(name)
        } else if b.is_ascii_digit() {
            self.scan_number(start)?
        } else if b.is_ascii_alphabetic() {
            let w = self.take_while(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-');
            // `b3:<base32>` is one token: its payload may begin with a digit, so joining it
            // from a word and a colon in the parser would not always work.
            if w == "b3" && self.src.as_bytes().get(self.pos) == Some(&b':') {
                self.pos += 1;
                let body = self.take_while(|c| c.is_ascii_alphanumeric());
                if body.is_empty() {
                    return Err(self.err(Span::new(start, self.pos), "expected a uid after `b3:`"));
                }
                Tok::Uid(format!("b3:{body}"))
            } else {
                Tok::Word(w)
            }
        } else {
            match PUNCT.iter().find(|p| self.src[self.pos..].starts_with(**p)) {
                Some(p) => {
                    self.pos += p.len();
                    Tok::Punct(p)
                }
                None => {
                    // Step by the character and not by the byte: the span of an unexpected
                    // character must land on a boundary, and everything here that is not a
                    // keyword or a number may be any scalar value at all.
                    let ch = self.src[self.pos..]
                        .chars()
                        .next()
                        .expect("in bounds and on a boundary");
                    let end = self.pos + ch.len_utf8();
                    return Err(self.err(
                        Span::new(self.pos, end),
                        format!("unexpected character `{ch}`"),
                    ));
                }
            }
        };
        Ok(Token {
            tok,
            span: Span::new(start, self.pos),
            ws_before: ws,
        })
    }

    fn scan_string(&mut self) -> Result<Tok, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let bytes = self.src.as_bytes();
        loop {
            if self.pos >= bytes.len() {
                return Err(self.err(Span::new(start, self.pos), "unterminated string"));
            }
            match bytes[self.pos] {
                b'"' => {
                    self.pos += 1;
                    break;
                }
                b'\n' => {
                    return Err(self.err(Span::new(start, self.pos), "unterminated string"));
                }
                b'\\' => {
                    self.pos += 1;
                    let e = bytes.get(self.pos).copied().ok_or_else(|| {
                        self.err(Span::new(start, self.pos), "unterminated string")
                    })?;
                    out.push(match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'n' => '\n',
                        b't' => '\t',
                        _ => {
                            return Err(self.err(
                                Span::new(self.pos - 1, self.pos + 1),
                                "unknown escape; `sq/1` has `\\\"`, `\\\\`, `\\n` and `\\t`",
                            ))
                        }
                    });
                    self.pos += 1;
                }
                _ => {
                    let ch = self.src[self.pos..]
                        .chars()
                        .next()
                        .expect("in bounds and on a boundary");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
        let nfc = unicode_normalization::is_nfc(&out);
        Ok(Tok::Str { value: out, nfc })
    }

    fn scan_number(&mut self, start: usize) -> Result<Tok, Diagnostic> {
        let whole = self.take_while(|c| c.is_ascii_digit());
        let bytes = self.src.as_bytes();
        // A `.` is a decimal point only with a digit behind it: `..` is a range and `1.foo`
        // is not a number at all.
        let is_decimal = bytes.get(self.pos) == Some(&b'.')
            && bytes.get(self.pos + 1).is_some_and(u8::is_ascii_digit);
        if !is_decimal {
            return whole
                .parse::<i64>()
                .map(Tok::Int)
                .map_err(|_| self.err(Span::new(start, self.pos), "integer out of range"));
        }
        self.pos += 1;
        let frac = self.take_while(|c| c.is_ascii_digit());
        if frac.len() > 4 {
            return Err(self.err(
                Span::new(start, self.pos),
                "a decimal carries at most four fractional places",
            ));
        }
        let whole: i64 = whole
            .parse()
            .map_err(|_| self.err(Span::new(start, self.pos), "number out of range"))?;
        let mut scaled = frac.parse::<i64>().unwrap_or(0);
        for _ in frac.len()..4 {
            scaled *= 10;
        }
        let total = whole
            .checked_mul(10_000)
            .and_then(|w| w.checked_add(scaled))
            .ok_or_else(|| self.err(Span::new(start, self.pos), "number out of range"))?;
        Ok(Tok::Dec(total))
    }

    // ---- trivia ----------------------------------------------------------------------------

    /// Returns `(whitespace seen, a stage break seen)`. A newline and a `|` are both breaks
    /// (OQ-75); runs of them collapse, so a blank line between stages is not an empty stage.
    fn skip_trivia(&mut self) -> (bool, bool) {
        let mut ws = false;
        let mut brk = false;
        loop {
            let bytes = self.src.as_bytes();
            if self.pos >= bytes.len() {
                return (ws, brk);
            }
            match bytes[self.pos] {
                b'\n' => {
                    brk = true;
                    ws = true;
                    self.pos += 1;
                }
                b'|' => {
                    brk = true;
                    self.pos += 1;
                }
                c if c.is_ascii_whitespace() => {
                    ws = true;
                    self.pos += 1;
                }
                // `#` opens a comment only at the start of a token: at line start, or after
                // whitespace (SMYSL-2.5 §3.1). Mid-token it belongs to a locator.
                b'#' if ws || self.pos == 0 => {
                    while self.pos < bytes.len() && bytes[self.pos] != b'\n' {
                        self.pos += 1;
                    }
                }
                _ => return (ws, brk),
            }
        }
    }

    /// Whitespace and comments, but **not** a stage break: used by the raw modes, which belong
    /// to the stage they were called from.
    fn skip_trivia_same_stage(&mut self) -> usize {
        self.skip_blanks();
        self.pos
    }

    fn skip_blanks(&mut self) {
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            match bytes[self.pos] {
                b'\n' | b'|' => return,
                c if c.is_ascii_whitespace() => self.pos += 1,
                _ => return,
            }
        }
    }

    fn raw_until_range_or_space(&mut self) -> String {
        let start = self.pos;
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if b.is_ascii_whitespace() || b == b'|' || b == b')' {
                break;
            }
            if b == b'.' && bytes.get(self.pos + 1) == Some(&b'.') {
                break;
            }
            self.pos += 1;
        }
        self.src[start..self.pos].to_string()
    }

    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> String {
        let start = self.pos;
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() && f(bytes[self.pos]) {
            self.pos += 1;
        }
        self.src[start..self.pos].to_string()
    }

    pub fn err(&self, span: Span, msg: impl Into<String>) -> Diagnostic {
        Diagnostic::at(Code::E460, span).with_message(msg)
    }
}
