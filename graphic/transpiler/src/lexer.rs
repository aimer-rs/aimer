use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    /// Integer literal text, suffix included (`1`, `6u`, `0xFFu`).
    Int(String),
    /// Float literal text (`1.0`, `.5`, `1e-3`, `2.4f32`).
    Float(String),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub line: usize,
    /// Byte offsets of the token in the source (`start..end`).
    pub start: usize,
    pub end: usize,
}

#[derive(Debug)]
pub struct LexError {
    pub line: usize,
    /// Byte offsets of the offending text.
    pub start: usize,
    pub end: usize,
    pub msg: String,
}

impl fmt::Display for LexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.msg)
    }
}

// Longest first so that `..=` wins over `..` and `<<=` over `<<`.
const PUNCTS: &[&str] = &[
    "<<=", ">>=", "..=", "->", "=>", "::", "==", "!=", "<=", ">=", "&&", "||", "<<", ">>", "+=",
    "-=", "*=", "/=", "%=", "&=", "|=", "^=", "..", "#", "[", "]", "(", ")", "{", "}", "<", ">",
    "+", "-", "*", "/", "%", "=", "!", "~", "&", "|", "^", ".", ",", ";", ":", "@",
];

pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    let b = src.as_bytes();
    let mut i = 0;
    let mut line = 1;
    let mut out = Vec::new();

    while i < b.len() {
        let c = b[i];
        if c == b'\n' {
            line += 1;
            i += 1;
        } else if c.is_ascii_whitespace() {
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            let start = line;
            let from = i;
            let mut depth = 1;
            i += 2;
            while depth > 0 {
                if i >= b.len() {
                    return Err(LexError { line: start, start: from, end: from + 2, msg: "unterminated block comment".into() });
                }
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push(Token { tok: Tok::Ident(src[s..i].to_string()), line, start: s, end: i });
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            let (tok, next) = lex_number(src, i);
            out.push(Token { tok, line, start: i, end: next });
            i = next;
        } else if let Some(p) = PUNCTS.iter().find(|p| src[i..].starts_with(**p)) {
            out.push(Token { tok: Tok::Punct(p), line, start: i, end: i + p.len() });
            i += p.len();
        } else {
            let ch = src[i..].chars().next().unwrap_or(' ');
            return Err(LexError { line, start: i, end: i + ch.len_utf8(), msg: format!("unexpected character `{ch}`") });
        }
    }
    out.push(Token { tok: Tok::Eof, line, start: src.len(), end: src.len() });
    Ok(out)
}

fn lex_number(src: &str, start: usize) -> (Tok, usize) {
    let b = src.as_bytes();
    let mut i = start;
    let mut is_float = false;

    if b[i] == b'0' && matches!(b.get(i + 1), Some(b'x') | Some(b'X')) {
        i += 2;
        while i < b.len() && b[i].is_ascii_hexdigit() {
            i += 1;
        }
    } else {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        // `1..2` is a range, not `1.` followed by `.2`.
        if b.get(i) == Some(&b'.') && b.get(i + 1) != Some(&b'.') {
            is_float = true;
            i += 1;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        if matches!(b.get(i), Some(b'e') | Some(b'E')) {
            let mut j = i + 1;
            if matches!(b.get(j), Some(b'+') | Some(b'-')) {
                j += 1;
            }
            if b.get(j).is_some_and(u8::is_ascii_digit) {
                is_float = true;
                i = j;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
            }
        }
    }
    // Suffix: f32, i32, u32, u.
    for suf in ["f32", "i32", "u32", "u"] {
        if src[i..].starts_with(suf) && !b.get(i + suf.len()).is_some_and(|c| c.is_ascii_alphanumeric()) {
            if suf == "f32" {
                is_float = true;
            }
            i += suf.len();
            break;
        }
    }
    let text = src[start..i].to_string();
    (if is_float { Tok::Float(text) } else { Tok::Int(text) }, i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(s: &str) -> Vec<Tok> {
        lex(s).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn numbers() {
        assert_eq!(toks("1 6u 1.0 .5 1e-3 2.4f32")[..6], [
            Tok::Int("1".into()),
            Tok::Int("6u".into()),
            Tok::Float("1.0".into()),
            Tok::Float(".5".into()),
            Tok::Float("1e-3".into()),
            Tok::Float("2.4f32".into()),
        ]);
    }

    #[test]
    fn range_is_not_float() {
        assert_eq!(toks("0..8")[..3], [Tok::Int("0".into()), Tok::Punct(".."), Tok::Int("8".into())]);
    }

    #[test]
    fn nested_comments_and_lines() {
        let t = lex("/* a /* b */ c */ x\n// y\nz").unwrap();
        assert_eq!(t[0].tok, Tok::Ident("x".into()));
        assert_eq!(t[1].line, 3);
    }

    #[test]
    fn attrs_and_builtins() {
        assert_eq!(toks("#[vertex] @position")[..5], [
            Tok::Punct("#"),
            Tok::Punct("["),
            Tok::Ident("vertex".into()),
            Tok::Punct("]"),
            Tok::Punct("@"),
        ]);
    }
}
