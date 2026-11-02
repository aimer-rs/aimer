mod editable_text;
pub mod gesture;
mod input_field;
pub mod mouse_region;
mod text_area;
pub use aimer_text::TextSelection;
/// The generic callback machinery now lives in `aimer_utils` so lower-level
/// crates (e.g. `aimer_container`) can use it without a dependency cycle. It is
/// re-exported here so existing `aimer_input::callback::*` paths keep working.
pub use aimer_utils::callback;
pub use editable_text::{TextEditingController, TextEditingValue, TextRange};
pub use text_area::TextArea;
pub mod button;

pub use aimer_text::TextButton;

pub mod input {
    pub use crate::input_field::caret::*;
    pub use crate::input_field::raw_fields::*;
    pub use crate::input_field::chrome::TextFieldVisualState;
    pub use crate::input_field::{TextField, TextFieldState};
    pub use crate::{TextArea, TextEditingController, TextEditingValue, TextRange, TextSelection};
}
