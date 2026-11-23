//! Byte offsets and LSP positions (RFC SMYSL-6 §4.3).
//!
//! smysl's spans are UTF-8 byte offsets. An LSP position is a line and a column counted in
//! UTF-16 code units, or in bytes when the client and server agree on `utf-8`. A line ends at
//! `\n`, `\r\n` or a lone `\r`, as LSP 3.17 says. Every conversion clamps rather than panics: a
//! column past the end of its line is the line's end, a line past the last is the end of the
//! text, and an offset inside a character is that character's start.

/// How a position's column is counted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    /// UTF-16 code units, LSP's default.
    #[default]
    Utf16,
    /// Bytes, when the client offers `utf-8`.
    Utf8,
}

impl Encoding {
    /// The protocol's name for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Encoding::Utf16 => "utf-16",
            Encoding::Utf8 => "utf-8",
        }
    }
}

/// A zero-based line and column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Position {
    pub line: u32,
    pub character: u32,
}

impl Position {
    pub fn new(line: u32, character: u32) -> Position {
        Position { line, character }
    }
}

/// A half-open range of positions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Range {
    pub start: Position,
    pub end: Position,
}

impl Range {
    pub fn new(start: Position, end: Position) -> Range {
        Range { start, end }
    }
}

/// One text's line starts, for converting between byte offsets and positions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    /// The byte offset each line starts at; the first is 0.
    starts: Vec<usize>,
    len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let b = text.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => starts.push(i + 1),
                b'\r' if b.get(i + 1) == Some(&b'\n') => {
                    starts.push(i + 2);
                    i += 1;
                }
                b'\r' => starts.push(i + 1),
                _ => {}
            }
            i += 1;
        }
        LineIndex {
            starts,
            len: text.len(),
        }
    }

    /// How many lines the text has; an empty text has one.
    pub fn lines(&self) -> usize {
        self.starts.len()
    }

    /// Line `line`'s content, without its terminator.
    fn line_bounds(&self, text: &str, line: usize) -> (usize, usize) {
        let start = self.starts[line];
        let mut end = self.starts.get(line + 1).copied().unwrap_or(self.len);
        let b = text.as_bytes();
        if end > start && b.get(end - 1) == Some(&b'\n') {
            end -= 1;
        }
        if end > start && b.get(end - 1) == Some(&b'\r') {
            end -= 1;
        }
        (start, end)
    }

    /// The position of byte `offset` in `text`, which this index was built from.
    pub fn position(&self, text: &str, offset: usize, enc: Encoding) -> Position {
        let mut offset = offset.min(self.len).min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        let line = self.starts.partition_point(|s| *s <= offset) - 1;
        let (start, end) = self.line_bounds(text, line);
        let upto = &text[start..offset.min(end)];
        let character = match enc {
            Encoding::Utf8 => upto.len(),
            Encoding::Utf16 => upto.chars().map(char::len_utf16).sum(),
        };
        Position::new(line as u32, character as u32)
    }

    /// The byte offset of `pos` in `text`.
    pub fn offset(&self, text: &str, pos: Position, enc: Encoding) -> usize {
        let line = pos.line as usize;
        if line >= self.starts.len() {
            return self.len;
        }
        let (start, end) = self.line_bounds(text, line);
        let want = pos.character as usize;
        let mut units = 0;
        for (i, c) in text[start..end].char_indices() {
            let w = match enc {
                Encoding::Utf8 => c.len_utf8(),
                Encoding::Utf16 => c.len_utf16(),
            };
            // Inside this character: its start.
            if units + w > want {
                return start + i;
            }
            units += w;
        }
        end
    }

    /// The range of byte span `[from, to)`.
    pub fn range(&self, text: &str, from: usize, to: usize, enc: Encoding) -> Range {
        Range::new(
            self.position(text, from, enc),
            self.position(text, to.max(from), enc),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf16_and_utf8_columns_on_cjk_and_emoji() {
        // `語` is three bytes and one UTF-16 unit; `😀` is four bytes and two units.
        let text = "a語b😀c\nx";
        let ix = LineIndex::new(text);
        let c = text.find('c').unwrap();
        assert_eq!(ix.position(text, c, Encoding::Utf16), Position::new(0, 5));
        assert_eq!(ix.position(text, c, Encoding::Utf8), Position::new(0, 9));
        assert_eq!(ix.offset(text, Position::new(0, 5), Encoding::Utf16), c);
        assert_eq!(ix.offset(text, Position::new(0, 9), Encoding::Utf8), c);
        // A column inside the emoji's surrogate pair is the emoji's start.
        let e = text.find('😀').unwrap();
        assert_eq!(ix.offset(text, Position::new(0, 4), Encoding::Utf16), e);
        // An offset inside a character is that character's start.
        assert_eq!(
            ix.position(text, e + 2, Encoding::Utf16),
            Position::new(0, 3)
        );
        assert_eq!(
            ix.position(text, text.len(), Encoding::Utf16),
            Position::new(1, 1)
        );
    }

    #[test]
    fn every_line_ending_and_every_clamp() {
        let text = "one\r\ntwo\rthree\nfour";
        let ix = LineIndex::new(text);
        assert_eq!(ix.lines(), 4);
        for (line, word) in ["one", "two", "three", "four"].iter().enumerate() {
            let at = text.find(word).unwrap();
            let p = ix.position(text, at, Encoding::Utf16);
            assert_eq!(p, Position::new(line as u32, 0));
            assert_eq!(ix.offset(text, p, Encoding::Utf16), at);
        }
        // The `\r\n` itself belongs to line 0, and a column past the end is the end.
        assert_eq!(ix.position(text, 4, Encoding::Utf16), Position::new(0, 3));
        assert_eq!(ix.offset(text, Position::new(0, 99), Encoding::Utf16), 3);
        assert_eq!(
            ix.offset(text, Position::new(99, 0), Encoding::Utf16),
            text.len()
        );
        assert_eq!(
            ix.position(text, 10_000, Encoding::Utf8),
            Position::new(3, 4)
        );
        let empty = LineIndex::new("");
        assert_eq!(empty.position("", 0, Encoding::Utf16), Position::new(0, 0));
    }

    #[test]
    fn round_trip_on_every_char_boundary() {
        let text = "@claim c/b { status: derived }\n~ Ελληνικά, 中文, 🧪🧪 and more\r\n\rend";
        let ix = LineIndex::new(text);
        for enc in [Encoding::Utf16, Encoding::Utf8] {
            for (i, _) in text.char_indices() {
                // A line terminator's second byte is no position of its own.
                if text[..i].ends_with('\r') && text[i..].starts_with('\n') {
                    continue;
                }
                let p = ix.position(text, i, enc);
                let back = ix.offset(text, p, enc);
                let at_eol = text[i..].starts_with('\n') || text[i..].starts_with('\r');
                assert!(back == i || at_eol, "{enc:?} {i} -> {p:?} -> {back}");
            }
        }
    }
}
