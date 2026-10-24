//! Drag and drop for Aimer.
//!
//! Picking something up and putting it somewhere else involves three widgets
//! that cannot see each other: the one being carried, the one being dropped
//! onto, and the feedback painted above every clip boundary in the application.
//! This crate connects them through a single [`DragSession`] and the routed
//! event dispatch the framework already has, so nothing here re-implements hit
//! testing and nothing outside here learns what a drag is.
//!
//! # The drag session
//!
//! A drag is not a value an application holds; it is a state the *thread* is in,
//! and [`DragSession`] is that state. It holds one type-erased payload, keyed by
//! the pointer that opened it. [`Draggable`] opens it on a press that travels,
//! [`DragTarget`] reads it while the pointer passes overhead and takes it on a
//! drop, and [`DragOverlay`] reads its position to paint the feedback. Nothing
//! is threaded through the widget tree, so an application that never drags pays
//! nothing: no inherited widget, no per-frame work, no extra traversal.
//!
//! # Pairing a `Draggable` with a `DragTarget`
//!
//! ```no_run
//! use aimer_container::{Container, ZeroSizedBox};
//! use aimer_dnd::{DragTarget, DragTargetState, Draggable};
//!
//! #[derive(Clone, Copy)]
//! struct CardId(u32);
//!
//! // What is picked up.
//! let card = Draggable::new()
//!     .data(CardId(7))
//!     .feedback(|| Container::new().child(ZeroSizedBox))
//!     .child(Container::new().child(ZeroSizedBox));
//!
//! // Where it may land. A `DragTarget<CardId>` is invisible to a drag carrying
//! // anything else, so an unrelated drag passing overhead cannot disturb it.
//! let column = DragTarget::<CardId>::new()
//!     .will_accept(|id: &CardId| id.0 != 0)
//!     .on_accept(|_id: CardId| { /* move the card */ })
//!     .child(|state: DragTargetState| {
//!         let _highlight = state.is_hovered && state.will_accept;
//!         Container::new().child(ZeroSizedBox)
//!     });
//! ```
//!
//! [`DragTarget::child`] runs on hover-enter and hover-leave, not on every
//! pointer move, and only for the target the pointer actually crossed.
//!
//! # `DropZone` is the same thing with a different source
//!
//! [`DropZone`] receives files dragged in from the operating system. It looks
//! like a [`DragTarget`] whose payload is [`FileDrop`], and differs in exactly
//! two ways: the drag is not started by any widget, and the platform reports one
//! event per file with no marker for the end of the batch. The zone reassembles
//! the batch, so `on_drop` is called once with every path.
//!
//! # When a press becomes a drag
//!
//! [`DragStartMode`] defaults per input device rather than per widget, because
//! the right answer differs: a mouse press that travels is unambiguously a drag,
//! while a finger press that travels is much more likely to be a scroll that an
//! enclosing scrollable wants. So a mouse drags immediately and a finger has to
//! long-press first. Override it with [`Draggable::start_on`].
//!
//! # Examples
//!
//! ```
//! use aimer_attribute::position::Vec2d;
//! use aimer_dnd::{DragPayload, DragSession};
//! use aimer_events::pointer::PointerSource;
//! use aimer_widget::PointerKey;
//!
//! #[derive(Debug, PartialEq)]
//! struct CardId(u32);
//!
//! let pointer = PointerKey::new(PointerSource::Mouse, 0);
//! DragSession::begin(pointer, DragPayload::new(CardId(7)), Vec2d { x: 0.0, y: 0.0 });
//!
//! // A target that understands `CardId` sees it; one that does not, does not.
//! assert_eq!(DragSession::with_payload(|id: &CardId| id.0), Some(7));
//! assert_eq!(DragSession::with_payload(|s: &String| s.len()), None);
//!
//! DragSession::cancel(pointer);
//! ```
//!
//! # Completion seams
//!
//! * **One drag at a time.** The session is keyed by [`PointerKey`], but the
//!   policy is explicitly single-pointer: a second active drag is refused and
//!   a second `Draggable` does not capture its pointer while the first is live.
//! * **Bounded auto-scroll.** [`AutoScroller`] computes an edge-triggered,
//!   clamped logical delta and sends it through [`ScrollTarget`]. The adapter
//!   owns the actual scroll engine, so this crate does not replace scroll
//!   physics.
//! * **Stable-key reordering.** [`ReorderableList`] keeps preview, commit, and
//!   cancellation in one pointer-bound transaction, and also exposes a
//!   keyboard move path.
//! * **Safe file fallback.** [`FileDropPolicy`] validates file-drop metadata
//!   with file-count, size, type, path, directory, and symlink bounds before a
//!   caller processes the complete batch. winit's web backend still emits no
//!   native file-drag events; browser adapters can feed the validator directly.
//!
//! [`PointerKey`]: aimer_widget::PointerKey

mod auto_scroll;
mod draggable;
mod drop_zone;
mod file_drop;
mod overlay;
mod reorder;
mod session;
mod target;
#[cfg(test)]
mod test_support;

pub use crate::auto_scroll::{
    AutoScrollEdges, AutoScrollPolicy, AutoScrollPolicyError, AutoScrollStep, AutoScroller,
    ScrollTarget, ScrollViewport,
};
pub use crate::draggable::{DragStartMode, Draggable};
pub use crate::drop_zone::DropZone;
pub use crate::file_drop::{
    FileDropEntry, FileDropOutcome, FileDropPolicy, FileDropRejectReason, FileDropRejection,
    ValidatedFileDrop,
};
pub use crate::overlay::{DragAxis, DragOverlay};
pub use crate::reorder::{
    DropLocation, InsertionIndicator, KeyboardReorder, ReorderDrag, ReorderError, ReorderItem,
    ReorderOutcome, ReorderableList, StableKey, StableKeyError,
};
pub use crate::session::{DragPayload, DragSession, FileDrop};
pub use crate::target::{DragTarget, DragTargetState};
