/// A half-open range expressed in native UTF-16 code units.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NativeTextRange {
    pub start: usize,
    pub end: usize,
}

impl NativeTextRange {
    #[inline]
    pub const fn new(first: usize, second: usize) -> Self {
        if first <= second {
            Self {
                start: first,
                end: second,
            }
        } else {
            Self {
                start: second,
                end: first,
            }
        }
    }
}

/// One native editor mutation based on a mirrored Rust revision.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEditingDelta {
    pub session_id: u64,
    pub revision: u64,
    pub replacement: NativeTextRange,
    pub replacement_text: String,
    pub selection: NativeTextRange,
    pub composing: Option<NativeTextRange>,
}
