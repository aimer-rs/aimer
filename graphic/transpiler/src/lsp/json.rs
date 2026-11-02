//! A small JSON value, parser and writer. The crate has no dependencies, and the language
//! services only need to exchange simple documents with the editor.

use std::fmt::Write;

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    /// Keys keep their insertion order.
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn obj() -> Json {
        Json::Obj(Vec::new())
    }

    /// Builder: add a field to an object.
    pub fn with(mut self, key: &str, value: impl Into<Json>) -> Json {
        if let Json::Obj(fields) = &mut self {
            fields.push((key.to_string(), value.into()));
        }
        self
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        self.as_f64().filter(|n| *n >= 0.0).map(|n| n as usize)
    }

    pub fn as_array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn parse(src: &str) -> Result<Json, String> {
        let mut p = Parser { b: src.as_bytes(), src, i: 0 };
        let v = p.value(0)?;
        p.ws();
        if p.i != p.b.len() {
            return Err(p.fail("trailing characters"));
        }
        Ok(v)
    }
}

impl From<&str> for Json {
    fn from(s: &str) -> Json {
        Json::Str(s.to_string())
    }
}

impl From<String> for Json {
    fn from(s: String) -> Json {
        Json::Str(s)
    }
}

impl From<bool> for Json {
    fn from(b: bool) -> Json {
        Json::Bool(b)
    }
}

impl From<usize> for Json {
    fn from(n: usize) -> Json {
        Json::Num(n as f64)
    }
}

impl From<u32> for Json {
    fn from(n: u32) -> Json {
        Json::Num(n as f64)
    }
}

impl<T: Into<Json>> From<Vec<T>> for Json {
    fn from(v: Vec<T>) -> Json {
        Json::Arr(v.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<Json>> From<Option<T>> for Json {
    fn from(v: Option<T>) -> Json {
        v.map_or(Json::Null, Into::into)
    }
}

impl std::fmt::Display for Json {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Json::Null => f.write_str("null"),
            Json::Bool(b) => write!(f, "{b}"),
            Json::Num(n) if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 => write!(f, "{}", *n as i64),
            Json::Num(n) if n.is_finite() => write!(f, "{n}"),
            Json::Num(_) => f.write_str("null"),
            Json::Str(s) => write_str(f, s),
            Json::Arr(a) => {
                f.write_char('[')?;
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        f.write_char(',')?;
                    }
                    write!(f, "{v}")?;
                }
                f.write_char(']')
            }
            Json::Obj(o) => {
                f.write_char('{')?;
                for (i, (k, v)) in o.iter().enumerate() {
                    if i > 0 {
                        f.write_char(',')?;
                    }
                    write_str(f, k)?;
                    write!(f, ":{v}")?;
                }
                f.write_char('}')
            }
        }
    }
}

fn write_str(f: &mut std::fmt::Formatter<'_>, s: &str) -> std::fmt::Result {
    f.write_char('"')?;
    for c in s.chars() {
        match c {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if (c as u32) < 0x20 => write!(f, "\\u{:04x}", c as u32)?,
            c => f.write_char(c)?,
        }
    }
    f.write_char('"')
}

struct Parser<'a> {
    b: &'a [u8],
    src: &'a str,
    i: usize,
}

const MAX_DEPTH: usize = 64;

impl Parser<'_> {
    fn fail(&self, msg: &str) -> String {
        format!("invalid JSON at byte {}: {msg}", self.i)
    }

    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn lit(&mut self, word: &str, v: Json) -> Result<Json, String> {
        if self.src[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.fail("unexpected token"))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        if depth > MAX_DEPTH {
            return Err(self.fail("nested too deeply"));
        }
        self.ws();
        match self.b.get(self.i) {
            None => Err(self.fail("unexpected end")),
            Some(b'n') => self.lit("null", Json::Null),
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                if self.eat(b']') {
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b']') {
                        return Ok(Json::Arr(items));
                    }
                    return Err(self.fail("expected `,` or `]`"));
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut fields = Vec::new();
                if self.eat(b'}') {
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    if self.b.get(self.i) != Some(&b'"') {
                        return Err(self.fail("expected a string key"));
                    }
                    let key = self.string()?;
                    if !self.eat(b':') {
                        return Err(self.fail("expected `:`"));
                    }
                    fields.push((key, self.value(depth + 1)?));
                    if self.eat(b',') {
                        continue;
                    }
                    if self.eat(b'}') {
                        return Ok(Json::Obj(fields));
                    }
                    return Err(self.fail("expected `,` or `}`"));
                }
            }
            Some(_) => self.number(),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let s = self.i;
        while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
            self.i += 1;
        }
        self.src[s..self.i].parse::<f64>().map(Json::Num).map_err(|_| self.fail("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.src.get(self.i..self.i + 4).ok_or_else(|| self.fail("bad \\u escape"))?;
        let v = u32::from_str_radix(h, 16).map_err(|_| self.fail("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, String> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            let rest = &self.src[self.i..];
            let c = rest.chars().next().ok_or_else(|| self.fail("unterminated string"))?;
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.b.get(self.i).copied().ok_or_else(|| self.fail("unterminated escape"))?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) && self.src[self.i..].starts_with("\\u") {
                                self.i += 2;
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xD800) << 10) + lo.wrapping_sub(0xDC00)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.fail("unknown escape")),
                    }
                }
                c => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let src = r#"{"a":[1,2.5,-3],"b":"x\ny\"z\u00e9\ud83d\ude00","c":null,"d":true,"e":{}}"#;
        let v = Json::parse(src).unwrap();
        assert_eq!(v.get("b").unwrap().as_str().unwrap(), "x\ny\"zé😀");
        assert_eq!(Json::parse(&v.to_string()).unwrap(), v);
    }

    #[test]
    fn rejects_garbage() {
        assert!(Json::parse("{").is_err());
        assert!(Json::parse("[1,]").is_err());
        assert!(Json::parse("1 2").is_err());
        assert!(Json::parse(&"[".repeat(500)).is_err());
    }

    #[test]
    fn builder_and_numbers() {
        let v = Json::obj().with("n", 3usize).with("s", "é").with("l", vec![1u32, 2]);
        assert_eq!(v.to_string(), r#"{"n":3,"s":"é","l":[1,2]}"#);
    }
}
