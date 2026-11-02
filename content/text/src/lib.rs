mod paragraph;
mod rich_text;
mod selection;
mod selection_area;
mod text;
mod text_button;
mod text_source;
pub mod text_span;

mod text_accessibility;

pub use aimer_rubick::ShareRef;
pub use rich_text::{LinkCallback, RawRichText, RichText};
pub use selection::{SelectionParticipant, SelectionRegion, TextSelection};
pub use selection_area::{SelectionArea, SelectionAreaElement};
pub use text::Text;
pub use text::raw_text::RawTextWidget;
pub use text_accessibility::{
    TextAccessibilityCaret, TextAccessibilityCluster, TextAccessibilityLine,
    TextAccessibilitySelectionRect, TextAccessibilitySnapshot,
};
pub use text_button::TextButton;
pub use text_source::TextSource;
pub use text_span::{SpanStyle, TextSpan};
