mod paragraph;
mod rich_text;
mod text;
mod text_button;
pub mod text_span;

pub use aimer_rubick::ShareRef;
pub use rich_text::{LinkCallback, RawRichText, RichText};
pub(crate) use aimer_text_selection as selection;
pub use aimer_text_selection::{
    SelectionArea, SelectionAreaElement, SelectionParticipant, SelectionRegion, TextSelection,
};
pub use text::Text;
pub use text::raw_text::RawTextWidget;
pub use aimer_text_selection::{
    TextAccessibilityCaret, TextAccessibilityCluster, TextAccessibilityLine,
    TextAccessibilitySelectionRect, TextAccessibilitySnapshot,
};
pub use text_button::TextButton;
pub use aimer_text_selection::TextSource;
pub use text_span::{SpanStyle, TextSpan};
