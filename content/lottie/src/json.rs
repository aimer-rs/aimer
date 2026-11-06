//! A small, strict JSON reader for Lottie documents.
//!
//! Lottie files are plain JSON, and the crate only ever reads them, so this
//! module parses straight into a borrowed [`Json`] tree and has no writer.
//! Strings without escapes borrow from the input.

use std::borrow::Cow;

/// Maximum nesting of arrays and objects. Lottie documents are shallow, and the
/// cap keeps hostile input from overflowing the stack through recursion.
pub(crate) const MAX_DEPTH: usize = 128;

/// A parsed JSON value that borrows from the source text where it can.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json<'a> {
    Null,
    Bool(bool),
    Number(f64),
    String(Cow<'a, str>),
    Array(Vec<Json<'a>>),
    /// Members in source order. Lottie objects are small, so a linear scan in
    /// [`Json::get`] beats hashing every key.
    Object(Vec<(Cow<'a, str>, Json<'a>)>),
}

/// Why a document failed to parse, with the byte offset where it was noticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct JsonError {
    pub offset: usize,
    pub kind: JsonErrorKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JsonErrorKind {
    UnexpectedEnd,
    UnexpectedByte,
    InvalidNumber,
    InvalidEscape,
    InvalidUnicode,
    ControlInString,
    InvalidUtf8,
    TrailingData,
    TooDeep,
}

impl<'a> Json<'a> {
    /// Parses a complete document. A leading UTF-8 byte-order mark is skipped.
    pub(crate) fn parse(input: &'a [u8]) -> Result<Json<'a>> {
        let pos = if input.starts_with(b"\xEF\xBB\xBF") { 3 } else { 0 };
        let mut parser = Parser { src: input, pos };
        parser.skip_ws();
        let value = parser.value(0)?;
        parser.skip_ws();
        if parser.pos < input.len() {
            return Err(parser.err(JsonErrorKind::TrailingData));
        }
        Ok(value)
    }

    /// Returns the member named `key` of an object. With duplicate keys the last
    /// one wins, as in most JSON readers.
    pub(crate) fn get(&self, key: &str) -> Option<&Json<'a>> {
        match self {
            Json::Object(members) => members.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

type Result<T> = std::result::Result<T, JsonError>;

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    #[inline]
    fn err(&self, kind: JsonErrorKind) -> JsonError {
        JsonError { offset: self.pos, kind }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    #[inline]
    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    /// Consumes `byte`, or reports what was found instead.
    #[inline]
    fn expect(&mut self, byte: u8) -> Result<()> {
        match self.peek() {
            Some(b) if b == byte => {
                self.pos += 1;
                Ok(())
            }
            Some(_) => Err(self.err(JsonErrorKind::UnexpectedByte)),
            None => Err(self.err(JsonErrorKind::UnexpectedEnd)),
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json<'a>> {
        match self.peek() {
            None => Err(self.err(JsonErrorKind::UnexpectedEnd)),
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err(JsonErrorKind::UnexpectedByte)),
        }
    }

    fn literal(&mut self, word: &[u8], value: Json<'a>) -> Result<Json<'a>> {
        for &expected in word {
            self.expect(expected)?;
        }
        Ok(value)
    }

    fn array(&mut self, depth: usize) -> Result<Json<'a>> {
        let depth = self.enter(depth)?;
        self.pos += 1; // '['
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value(depth)?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Array(items));
                }
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedByte)),
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json<'a>> {
        let depth = self.enter(depth)?;
        self.pos += 1; // '{'
        let mut members = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(members));
        }
        loop {
            self.skip_ws();
            if self.peek() != Some(b'"') {
                return Err(self.err(match self.peek() {
                    None => JsonErrorKind::UnexpectedEnd,
                    Some(_) => JsonErrorKind::UnexpectedByte,
                }));
            }
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            self.skip_ws();
            members.push((key, self.value(depth)?));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Object(members));
                }
                Some(_) => return Err(self.err(JsonErrorKind::UnexpectedByte)),
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
            }
        }
    }

    /// Returns the nesting level of a container opened at `depth`.
    #[inline]
    fn enter(&self, depth: usize) -> Result<usize> {
        if depth >= MAX_DEPTH {
            return Err(self.err(JsonErrorKind::TooDeep));
        }
        Ok(depth + 1)
    }

    /// `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`
    fn number(&mut self) -> Result<Json<'a>> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.err(JsonErrorKind::InvalidNumber)),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            self.require_digits()?;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            self.require_digits()?;
        }
        // The grammar above admits ASCII only, so the slice is valid UTF-8.
        let text = std::str::from_utf8(&self.src[start..self.pos])
            .expect("number bytes are ASCII digits and signs");
        match text.parse::<f64>() {
            // "1e999" parses to infinity; JSON has no such number.
            Ok(n) if n.is_finite() => Ok(Json::Number(n)),
            _ => Err(JsonError { offset: start, kind: JsonErrorKind::InvalidNumber }),
        }
    }

    #[inline]
    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
    }

    #[inline]
    fn require_digits(&mut self) -> Result<()> {
        if !matches!(self.peek(), Some(b'0'..=b'9')) {
            return Err(self.err(JsonErrorKind::InvalidNumber));
        }
        self.digits();
        Ok(())
    }

    /// Validates `src[from..to]` as UTF-8, reporting the first bad byte.
    fn utf8(&self, from: usize, to: usize) -> Result<&'a str> {
        let src: &'a [u8] = self.src;
        std::str::from_utf8(&src[from..to]).map_err(|e| JsonError {
            offset: from + e.valid_up_to(),
            kind: JsonErrorKind::InvalidUtf8,
        })
    }

    /// Reads a string at the opening quote. Borrows when there is no escape.
    fn string(&mut self) -> Result<Cow<'a, str>> {
        self.pos += 1; // opening quote
        let start = self.pos;
        // Fast path: scan for the closing quote; any escape drops to the slow path.
        loop {
            match self.peek() {
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(b'"') => {
                    let text = self.utf8(start, self.pos)?;
                    self.pos += 1;
                    return Ok(Cow::Borrowed(text));
                }
                Some(b'\\') => break,
                Some(c) if c < 0x20 => return Err(self.err(JsonErrorKind::ControlInString)),
                Some(_) => self.pos += 1,
            }
        }
        let mut out = String::from(self.utf8(start, self.pos)?);
        let mut run = self.pos;
        loop {
            match self.peek() {
                None => return Err(self.err(JsonErrorKind::UnexpectedEnd)),
                Some(b'"') => {
                    out.push_str(self.utf8(run, self.pos)?);
                    self.pos += 1;
                    return Ok(Cow::Owned(out));
                }
                Some(b'\\') => {
                    out.push_str(self.utf8(run, self.pos)?);
                    self.pos += 1;
                    out.push(self.escape()?);
                    run = self.pos;
                }
                Some(c) if c < 0x20 => return Err(self.err(JsonErrorKind::ControlInString)),
                Some(_) => self.pos += 1,
            }
        }
    }

    /// Reads the part of an escape after the backslash.
    fn escape(&mut self) -> Result<char> {
        let Some(byte) = self.peek() else {
            return Err(self.err(JsonErrorKind::UnexpectedEnd));
        };
        let simple = match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => {
                self.pos += 1;
                return self.unicode_escape();
            }
            _ => return Err(self.err(JsonErrorKind::InvalidEscape)),
        };
        self.pos += 1;
        Ok(simple)
    }

    /// Reads the four hex digits after `\u`, joining a surrogate pair.
    fn unicode_escape(&mut self) -> Result<char> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                // A high surrogate must be followed by `\u` and a low surrogate.
                if self.peek() != Some(b'\\') || self.src.get(self.pos + 1) != Some(&b'u') {
                    return Err(self.err(JsonErrorKind::InvalidUnicode));
                }
                self.pos += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(self.err(JsonErrorKind::InvalidUnicode));
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => return Err(self.err(JsonErrorKind::InvalidUnicode)),
            other => other,
        };
        // Surrogates were handled above, so every remaining value is a scalar.
        char::from_u32(code).ok_or_else(|| self.err(JsonErrorKind::InvalidUnicode))
    }

    fn hex4(&mut self) -> Result<u32> {
        let mut value = 0u32;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(self.err(JsonErrorKind::UnexpectedEnd));
            };
            let digit = match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                b'A'..=b'F' => byte - b'A' + 10,
                _ => return Err(self.err(JsonErrorKind::InvalidUnicode)),
            };
            value = value << 4 | u32::from(digit);
            self.pos += 1;
        }
        Ok(value)
    }
}

impl JsonErrorKind {
    /// A short human-readable cause, for [`crate::LottieError::Syntax`].
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::UnexpectedEnd => "unexpected end of input",
            Self::UnexpectedByte => "unexpected character",
            Self::InvalidNumber => "invalid number",
            Self::InvalidEscape => "invalid escape sequence",
            Self::InvalidUnicode => "invalid unicode escape",
            Self::ControlInString => "control character in string",
            Self::InvalidUtf8 => "invalid UTF-8",
            Self::TrailingData => "data after the document",
            Self::TooDeep => "nesting too deep",
        }
    }
}
