use std::borrow::Cow;
use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aimer_text_buffer::TextSnapshot;
use unicode_segmentation::UnicodeSegmentation;

use aimer_animation::AnimInstant;
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_ctxmenu::{ContextMenuShape, ModalHandle};
use aimer_events::element::{ElementEvent, KeyAction, Modifiers, NamedKey};
use aimer_events::pointer::{PointerButton, PointerSource};
use aimer_events::text_editing::TextEditingDelta;
use aimer_events::window::get_window;
use aimer_style::{BorderStyle, BoxDecoration, LayoutSpacing, TextAlign, TextOverflow, TextStyle};
use aimer_text::{RawTextWidget, SelectionParticipant, TextSource};
use aimer_venus::ScopeId;
use aimer_widget::base::{BuildContext, Color, Colors};
use aimer_widget::{
    AnyElement, ChildBuilder, Drawable, Element, EventDispatchContext, EventElement, EventResult,
    EventTreeRole, FocusNode, LayoutCache, LayoutElement, PointerKey, Rebuildable, VisitorElement,
    Widget,
};

use crate::editable_text::{
    ControllerAttachment, DisplayText, EditableGeometry, EditableGeometryCache,
    EditableGeometryKey, EmbedBox, LineIndex, LineMetrics, adapt_native_delta, lay_out_lines,
    VisualLine, measure_lines, refine_from_layout, vertical_target, wrap_visual_lines,
};
use crate::input_field::caret::{
    CaretBlink, CaretBuilder, CaretContext, CaretGeometry, DefaultCaret,
};
use crate::input_field::caret_glide::{CaretGlide, CaretRect};
use crate::input_field::hover::TextHover;
use crate::input_field::chrome::{Chrome, VisualFlags};
use crate::input_field::gutter::{GutterMarkers, LineBar, LineBars, LineNumberAlign, LineNumbers, digits_of};
use crate::input_field::embed::{EmbedHeight, TextEmbed, TextEmbedder, embedder_id};
use crate::input_field::inlay::{self, Inlay, Inlays, Placed};
use crate::input_field::underline::{self, Underlines};
use crate::input_field::context_menu::{FieldAction, HoldOutcome, TouchHold};
use crate::input_field::highlight::{
    HighlightCache, HighlightContext, RetainedDisplay, TextHighlighter, highlighter_id,
};
use crate::{TextEditingController, TextEditingValue};

static NEXT_TEXT_EDITING_SESSION: AtomicU64 = AtomicU64::new(1);

/// Traces the native keyboard pipeline in mobile debug builds.
///
/// The delta stream between the hidden platform editor and a field is
/// invisible on a device; these lines are what `aimer run` shows when a
/// keyboard misbehaves. Release builds, and every platform whose input
/// method talks to the field directly, compile the calls away.
#[cfg(all(debug_assertions, any(target_os = "ios", target_os = "android"), feature = "ime_trace"))]
macro_rules! ime_trace {
    ($($arg:tt)*) => { aimer_utils::debug!($($arg)*) };
}
#[cfg(not(all(debug_assertions, any(target_os = "ios", target_os = "android"), feature = "ime_trace")))]
macro_rules! ime_trace {
    ($($arg:tt)*) => {{}};
}

include!("raw_fields/callback.rs");
include!("raw_fields/field.rs");
include!("raw_fields/caret_host.rs");
include!("raw_fields/embed_host.rs");
include!("raw_fields/inlay_host.rs");
include!("raw_fields/underline_paint.rs");
include!("raw_fields/composition.rs");
include!("raw_fields/edit_helpers.rs");
include!("raw_fields/dimensions.rs");
include!("raw_fields/windowing.rs");
include!("raw_fields/navigation.rs");
include!("raw_fields/gutter_paint.rs");
include!("raw_fields/label_paint.rs");
include!("raw_fields/selection_paint.rs");
include!("raw_fields/caret_marks_paint.rs");
include!("raw_fields/hover_resolve.rs");
include!("raw_fields/platform.rs");

mod link;
mod locate;
mod menu;
mod semantics;

include!("raw_fields/event.rs");
include!("raw_fields/layout.rs");

#[cfg(test)]
mod caret_marks_tests;
#[cfg(test)]
mod label_tests;
#[cfg(test)]
mod text_hover_tests;
#[cfg(test)]
mod scroll_memory_tests;
#[cfg(test)]
mod highlight_tests;
#[cfg(test)]
mod inlay_tests;
#[cfg(test)]
mod underline_tests;
#[cfg(test)]
mod embed_tests;
#[cfg(test)]
mod area_selection_tests;
#[cfg(test)]
mod bench_tests;
#[cfg(test)]
mod lookup_tests;
#[cfg(test)]
mod scrolled_paint_tests;
#[cfg(test)]
mod windowing_tests;
#[cfg(test)]
mod tab_tests;
#[cfg(test)]
mod shared_paste_tests;
#[cfg(test)]
mod navigation_tests;
#[cfg(test)]
mod no_wrap_tests;
#[cfg(test)]
mod gutter_tests;
#[cfg(test)]
mod marker_tests;

/// Fixtures shared by the test modules living beside the logic they cover.
#[cfg(test)]
pub(crate) mod test_support;
