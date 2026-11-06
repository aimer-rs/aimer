use std::fmt;

/// Why a document could not be loaded at all. Problems inside an otherwise
/// usable document are [`Diagnostic`](crate::Diagnostic)s instead.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LottieError {
    /// The text is not valid JSON.
    Syntax { offset: usize, reason: &'static str },
    /// The top-level JSON value is not an object.
    NotAnObject,
    /// A required header field (`w`, `h`, `fr`, `op`) is absent or not a number.
    MissingField(&'static str),
    /// A header field has a value that cannot be played.
    InvalidField(&'static str),
}

impl fmt::Display for LottieError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax { offset, reason } => write!(f, "invalid JSON at byte {offset}: {reason}"),
            Self::NotAnObject => f.write_str("a Lottie document must be a JSON object"),
            Self::MissingField(name) => write!(f, "missing or non-numeric field `{name}`"),
            Self::InvalidField(name) => write!(f, "field `{name}` has an unusable value"),
        }
    }
}

impl std::error::Error for LottieError {}
