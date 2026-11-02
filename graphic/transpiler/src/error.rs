use std::fmt;

use crate::lexer::LexError;

#[derive(Debug, Clone)]
pub struct Error {
    /// 0 when unknown.
    pub line: usize,
    /// Byte range in the source the error points at, when the parser or lexer knows it.
    pub span: Option<(usize, usize)>,
    pub msg: String,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(msg: impl Into<String>) -> Self {
        Error { line: 0, span: None, msg: msg.into() }
    }

    pub fn at(line: usize, msg: impl Into<String>) -> Self {
        Error { line, span: None, msg: msg.into() }
    }

    pub fn spanned(line: usize, start: usize, end: usize, msg: impl Into<String>) -> Self {
        Error { line, span: Some((start, end)), msg: msg.into() }
    }

    pub fn context(mut self, ctx: impl fmt::Display) -> Self {
        self.msg = format!("{ctx}: {}", self.msg);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line > 0 {
            write!(f, "line {}: {}", self.line, self.msg)
        } else {
            write!(f, "{}", self.msg)
        }
    }
}

impl std::error::Error for Error {}

impl From<LexError> for Error {
    fn from(e: LexError) -> Self {
        Error { line: e.line, span: Some((e.start, e.end)), msg: e.msg }
    }
}

macro_rules! bail {
    ($($arg:tt)*) => { return Err($crate::error::Error::new(format!($($arg)*))) };
}
pub(crate) use bail;
