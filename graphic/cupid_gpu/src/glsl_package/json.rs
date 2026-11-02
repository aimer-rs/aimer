//! A small JSON reader for the transpiler's layout files.
//!
//! The layout is machine-written and tiny, and this crate has no dependencies,
//! so a full serde stack would cost more than it saves. The reader accepts all
//! of JSON (so a newer transpiler adding keys does not break an older runtime)
//! and keeps numbers as their source text so callers can demand an exact `u32`.

/// Containers nest at most this deep, which bounds the recursion on hostile input.
const MAX_DEPTH: usize = 32;

#[derive(Debug)]
pub(super) enum Json<'a> {
    Null,
    /// A boolean; layouts never carry one, so its value is not kept.
    Bool,
    /// The number exactly as written.
    Number(&'a str),
    String(String),
    Array(Vec<Json<'a>>),
    Object(Vec<(String, Json<'a>)>),
}

impl<'a> Json<'a> {
    /// The value of `key` when this is an object that has it.
    pub(super) fn get(&self, key: &str) -> Option<&Json<'a>> {
        match self {
            Json::Object(entries) => entries
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value),
            _ => None,
        }
    }

    pub(super) fn as_str(&self) -> Option<&str> {
        match self {
            Json::String(text) => Some(text),
            _ => None,
        }
    }

    pub(super) fn as_array(&self) -> Option<&[Json<'a>]> {
        match self {
            Json::Array(items) => Some(items),
            _ => None,
        }
    }

    /// This number as an unsigned 32-bit integer; `-1`, `1.5`, `1e3` and
    /// anything over `u32::MAX` are not.
    pub(super) fn as_u32(&self) -> Option<u32> {
        match self {
            Json::Number(text) => text.parse().ok(),
            _ => None,
        }
    }
}

pub(super) fn parse(text: &str) -> Result<Json<'_>, String> {
    let mut reader = Reader {
        text,
        position: 0,
    };
    let value = reader.value(0)?;
    reader.skip_whitespace();
    if reader.position != text.len() {
        return Err(reader.error("unexpected text after the value"));
    }
    Ok(value)
}

struct Reader<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> Reader<'a> {
    fn error(&self, message: &str) -> String {
        format!("{message} at byte {}", self.position)
    }

    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.position).copied()
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        if self.peek() == Some(byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.eat(byte) {
            Ok(())
        } else {
            Err(self.error(&format!("expected `{}`", byte as char)))
        }
    }

    fn keyword(&mut self, word: &str, value: Json<'a>) -> Result<Json<'a>, String> {
        if self.text[self.position..].starts_with(word) {
            self.position += word.len();
            Ok(value)
        } else {
            Err(self.error("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json<'a>, String> {
        if depth > MAX_DEPTH {
            return Err(self.error("nesting is too deep"));
        }
        self.skip_whitespace();
        match self.peek() {
            None => Err(self.error("unexpected end of input")),
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Json::String),
            Some(b't') => self.keyword("true", Json::Bool),
            Some(b'f') => self.keyword("false", Json::Bool),
            Some(b'n') => self.keyword("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json<'a>, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.eat(b'}') {
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_whitespace();
            if self.eat(b',') {
                continue;
            }
            self.expect(b'}')?;
            return Ok(Json::Object(entries));
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json<'a>, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.eat(b']') {
            return Ok(Json::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_whitespace();
            if self.eat(b',') {
                continue;
            }
            self.expect(b']')?;
            return Ok(Json::Array(items));
        }
    }

    fn number(&mut self) -> Result<Json<'a>, String> {
        let start = self.position;
        self.eat(b'-');
        let digits = self.position;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.position += 1;
        }
        if self.position == digits {
            return Err(self.error("a number needs digits"));
        }
        if self.eat(b'.') {
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.position += 1;
            }
            while matches!(self.peek(), Some(b'0'..=b'9')) {
                self.position += 1;
            }
        }
        Ok(Json::Number(&self.text[start..self.position]))
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote or escape in one slice. Both
            // are ASCII, so the run always ends on a character boundary.
            let rest = &self.text[self.position..];
            let run = rest.find(['"', '\\']).unwrap_or(rest.len());
            out.push_str(&rest[..run]);
            self.position += run;
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.position += 1;
                    return Ok(out);
                }
                _ => {
                    self.position += 1;
                    out.push(self.escape()?);
                }
            }
        }
    }

    fn escape(&mut self) -> Result<char, String> {
        let Some(byte) = self.peek() else {
            return Err(self.error("unterminated escape"));
        };
        self.position += 1;
        Ok(match byte {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode_escape(),
            _ => return Err(self.error("unknown escape")),
        })
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self
            .text
            .get(self.position..self.position + 4)
            .ok_or_else(|| self.error("short \\u escape"))?;
        // `from_str_radix` would also take a leading `+`.
        if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(self.error("invalid \\u escape"));
        }
        let value =
            u32::from_str_radix(digits, 16).map_err(|_| self.error("invalid \\u escape"))?;
        self.position += 4;
        Ok(value)
    }

    fn unicode_escape(&mut self) -> Result<char, String> {
        let high = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&high) {
            // A high surrogate must be followed by `\uDC00..DFFF`.
            if !self.text[self.position..].starts_with("\\u") {
                return Err(self.error("lone surrogate"));
            }
            self.position += 2;
            let low = self.hex4()?;
            if !(0xDC00..0xE000).contains(&low) {
                return Err(self.error("lone surrogate"));
            }
            0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00)
        } else {
            high
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid \\u escape"))
    }
}
