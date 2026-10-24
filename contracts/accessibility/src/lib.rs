//! Platform-neutral accessibility semantics for Aimer widgets.
//!
//! The crate deliberately owns only portable data and narrow adapter traits. A
//! host can turn a [`SemanticSnapshot`] into a native or browser accessibility
//! tree, while focus ownership, rendering, and platform policy remain outside
//! this crate.

#![deny(missing_docs)]

mod announcement;
mod model;
mod preferences;
mod tree;
mod validation;

pub use announcement::{
    Announcement, AnnouncementError, AnnouncementKind, AnnouncementPort, AnnouncementPriority,
    MAX_ANNOUNCEMENT_CHARS, NoopAnnouncementPort,
};
pub use model::{
    ActionRequest, CheckedState, NodeId, RangeError, Role, SemanticAction, SemanticBehavior,
    SemanticNode, SemanticState, ValueRange,
};
pub use preferences::{
    AccessibilityPreferences, MAX_TEXT_SCALE, MIN_TEXT_SCALE, PreferenceAdapter, PreferenceError,
};
pub use tree::{
    ActionDispatchError, ActionHandler, FocusTraversalSource, SemanticSnapshot, SemanticTraversal,
    SemanticTree, TreeError,
};
pub use validation::{
    Bounds, BoundsError, Color, ColorError, ContrastError, TouchTargetAxis, TouchTargetError,
    TouchTargetPolicy, TouchTargetPolicyError, contrast_ratio, validate_contrast,
    validate_touch_target,
};
