//! The one place that knows a drag is happening.
//!
//! A drag has to be visible to three parties that are nowhere near each other in
//! the widget tree: the widget that was picked up, the widget being dragged
//! *onto*, and the overlay painting the feedback above everything. Threading a
//! value through the tree between them would mean an inherited widget on every
//! frame of every application, dragging or not. Instead the drag lives here, in
//! one thread-local slot, and the widgets read it.
//!
//! The session knows nothing about widgets and nothing about what is being
//! dragged. It holds a [`DragPayload`] — a value plus the [`TypeId`] it was
//! stored under — so a `DragTarget<T>` can ask "is this mine?" with one integer
//! comparison and never see a payload it did not ask for.
//!
//! Exactly one drag is live at a time. The slot is keyed by [`PointerKey`] so a
//! second pointer cannot silently steal a drag in progress; callers that need
//! multi-touch semantics must compose a separate higher-level coordinator.

use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::path::PathBuf;

use aimer_attribute::position::Vec2d;
use aimer_widget::PointerKey;

/// A dragged value, erased to its type identity.
///
/// The session stores payloads uniformly so that there is one registry rather
/// than one per payload type. Recovering the value costs one `TypeId`
/// comparison, and a mismatch is not an error — it is how a target learns the
/// drag passing over it belongs to somebody else.
///
/// # Examples
///
/// ```
/// use aimer_dnd::DragPayload;
///
/// #[derive(Debug, PartialEq)]
/// struct CardId(u32);
///
/// let payload = DragPayload::new(CardId(7));
///
/// assert!(payload.is::<CardId>());
/// assert!(!payload.is::<u32>());
/// assert_eq!(payload.downcast::<CardId>().ok(), Some(CardId(7)));
/// ```
pub struct DragPayload {
    type_id: TypeId,
    value: Box<dyn Any>,
}

impl DragPayload {
    /// Erases `value` into a payload.
    #[inline]
    pub fn new<T: 'static>(value: T) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            value: Box::new(value),
        }
    }

    /// Returns whether the payload carries a `T`.
    #[inline]
    pub fn is<T: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<T>()
    }

    /// Returns the [`TypeId`] the payload was stored under.
    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    /// Borrows the payload as a `T`, or `None` if it carries another type.
    #[inline]
    pub fn downcast_ref<T: 'static>(&self) -> Option<&T> {
        self.value.downcast_ref::<T>()
    }

    /// Recovers the value, giving the payload back unchanged if it carries
    /// another type.
    #[inline]
    pub fn downcast<T: 'static>(self) -> Result<T, Self> {
        match self.value.downcast::<T>() {
            Ok(value) => Ok(*value),
            Err(value) => Err(Self {
                type_id: self.type_id,
                value,
            }),
        }
    }
}

/// The paths an operating-system file drag carries.
///
/// This is an ordinary drag payload, which is the point: a `DropZone` is a drag
/// target whose payload happens to be `FileDrop`, so hover highlighting, hit
/// testing and the overlay are written once and shared with in-application
/// drags.
///
/// # Examples
///
/// ```
/// use std::path::PathBuf;
///
/// use aimer_dnd::FileDrop;
///
/// let drop = FileDrop::new(vec![PathBuf::from("/tmp/a.png")]);
///
/// assert_eq!(drop.paths.len(), 1);
/// ```
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileDrop {
    /// Every path in the drag, in the order the platform reported them.
    pub paths: Vec<PathBuf>,
}

impl FileDrop {
    /// Creates a file drag carrying `paths`.
    #[inline]
    pub fn new(paths: Vec<PathBuf>) -> Self {
        Self { paths }
    }
}

/// The live drag, if there is one.
struct ActiveDrag {
    pointer: PointerKey,
    payload: DragPayload,
    origin: Vec2d,
    pos: Vec2d,
}

thread_local! {
    /// The single in-flight drag.
    ///
    /// `None` while nothing is being dragged, which is the overwhelmingly common
    /// case: reading it is one thread-local access and one discriminant test,
    /// with no allocation and no traversal.
    static ACTIVE: RefCell<Option<ActiveDrag>> = const { RefCell::new(None) };
}

/// The in-flight drag, as a namespace rather than a value.
///
/// Every method is associated: there is one drag per thread and no handle to
/// pass around, which keeps `Draggable`, `DragTarget` and the overlay from
/// having to find each other in the tree.
///
/// # Examples
///
/// ```
/// use aimer_attribute::position::Vec2d;
/// use aimer_dnd::{DragPayload, DragSession};
/// use aimer_events::pointer::PointerSource;
/// use aimer_widget::PointerKey;
///
/// #[derive(Debug, PartialEq)]
/// struct CardId(u32);
///
/// let pointer = PointerKey::new(PointerSource::Mouse, 0);
/// let origin = Vec2d { x: 10.0, y: 10.0 };
///
/// assert!(DragSession::begin(pointer, DragPayload::new(CardId(7)), origin));
/// assert!(DragSession::is_active());
///
/// DragSession::update(pointer, Vec2d { x: 40.0, y: 12.0 });
/// assert_eq!(DragSession::with_payload(|id: &CardId| id.0), Some(7));
///
/// let dropped = DragSession::take(pointer).expect("the drag is still live");
/// assert_eq!(dropped.downcast::<CardId>().ok(), Some(CardId(7)));
/// assert!(!DragSession::is_active());
/// ```
pub struct DragSession;

impl DragSession {
    /// Opens a drag for `pointer`, carrying `payload` from `origin`.
    ///
    /// Returns `false` and changes nothing if a drag is already live. Only one
    /// drag runs at a time, and refusing is the honest answer: silently
    /// replacing the drag would strand the widget that opened the first one.
    #[inline]
    pub fn begin(pointer: PointerKey, payload: DragPayload, origin: Vec2d) -> bool {
        ACTIVE.with_borrow_mut(|active| {
            if active.is_some() {
                return false;
            }
            *active = Some(ActiveDrag {
                pointer,
                payload,
                origin,
                pos: origin,
            });
            true
        })
    }

    /// Moves the live drag to `pos`.
    ///
    /// A pointer that does not own the drag is ignored, so a stray move from a
    /// second finger cannot drag somebody else's payload around.
    #[inline]
    pub fn update(pointer: PointerKey, pos: Vec2d) {
        ACTIVE.with_borrow_mut(|active| {
            if let Some(drag) = active.as_mut()
                && drag.pointer == pointer
            {
                drag.pos = pos;
            }
        });
    }

    /// Ends the drag owned by `pointer` and returns what it carried.
    ///
    /// Returns `None` if there is no drag or if it belongs to another pointer,
    /// in which case the drag is left running.
    #[inline]
    pub fn take(pointer: PointerKey) -> Option<DragPayload> {
        ACTIVE.with_borrow_mut(|active| {
            if active.as_ref().is_none_or(|drag| drag.pointer != pointer) {
                return None;
            }
            active.take().map(|drag| drag.payload)
        })
    }

    /// Ends the drag owned by `pointer`, discarding the payload.
    #[inline]
    pub fn cancel(pointer: PointerKey) {
        let _ = Self::take(pointer);
    }

    /// Ends whatever drag is live, whoever owns it.
    ///
    /// This is the window-level escape hatch — focus loss, a cancelled gesture —
    /// where there is no pointer left to name.
    #[inline]
    pub fn cancel_any() {
        ACTIVE.with_borrow_mut(|active| *active = None);
    }

    /// Returns whether a drag is in flight.
    #[inline]
    pub fn is_active() -> bool {
        ACTIVE.with_borrow(|active| active.is_some())
    }

    /// Returns the pointer carrying the live drag.
    #[inline]
    pub fn pointer() -> Option<PointerKey> {
        ACTIVE.with_borrow(|active| active.as_ref().map(|drag| drag.pointer))
    }

    /// Returns where the live drag currently is.
    #[inline]
    pub fn position() -> Option<Vec2d> {
        ACTIVE.with_borrow(|active| active.as_ref().map(|drag| drag.pos))
    }

    /// Returns where the live drag was picked up.
    #[inline]
    pub fn origin() -> Option<Vec2d> {
        ACTIVE.with_borrow(|active| active.as_ref().map(|drag| drag.origin))
    }

    /// Runs `f` on the payload if the live drag carries a `T`.
    ///
    /// Returns `None` when nothing is being dragged *and* when the drag carries
    /// another type — a target that only understands `T` cannot tell those
    /// apart and does not need to.
    #[inline]
    pub fn with_payload<T: 'static, R>(f: impl FnOnce(&T) -> R) -> Option<R> {
        ACTIVE.with_borrow(|active| {
            active
                .as_ref()
                .and_then(|drag| drag.payload.downcast_ref::<T>())
                .map(f)
        })
    }

    /// Replaces the live payload if it carries a `T`.
    ///
    /// This is what lets a file drag accumulate: each path the platform reports
    /// is appended to the [`FileDrop`] already in flight rather than opening a
    /// second drag.
    #[inline]
    pub fn with_payload_mut<T: 'static, R>(f: impl FnOnce(&mut T) -> R) -> Option<R> {
        ACTIVE.with_borrow_mut(|active| {
            active
                .as_mut()
                .and_then(|drag| drag.payload.value.downcast_mut::<T>())
                .map(f)
        })
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::pointer::PointerSource;

    use super::*;

    #[derive(Debug, PartialEq)]
    struct CardId(u32);

    #[derive(Debug, PartialEq)]
    struct Unrelated;

    fn mouse() -> PointerKey {
        PointerKey::new(PointerSource::Mouse, 0)
    }

    fn touch() -> PointerKey {
        PointerKey::new(PointerSource::Touch, 1)
    }

    fn origin() -> Vec2d {
        Vec2d { x: 1.0, y: 2.0 }
    }

    /// Every test shares one thread-local slot, so each starts from nothing.
    fn fresh() {
        DragSession::cancel_any();
    }

    #[test]
    fn a_drag_round_trips_the_value_it_was_given() {
        fresh();

        assert!(DragSession::begin(
            mouse(),
            DragPayload::new(CardId(7)),
            origin()
        ));
        let payload = DragSession::take(mouse()).expect("the drag is live");

        assert_eq!(payload.downcast::<CardId>().ok(), Some(CardId(7)));
        assert!(!DragSession::is_active());
    }

    #[test]
    fn a_payload_of_another_type_is_invisible() {
        fresh();
        DragSession::begin(mouse(), DragPayload::new(CardId(7)), origin());

        assert_eq!(DragSession::with_payload(|_: &Unrelated| ()), None);
        assert_eq!(DragSession::with_payload(|id: &CardId| id.0), Some(7));

        fresh();
    }

    #[test]
    fn a_second_drag_is_refused_while_one_is_live() {
        fresh();
        DragSession::begin(mouse(), DragPayload::new(CardId(1)), origin());

        assert!(!DragSession::begin(
            touch(),
            DragPayload::new(CardId(2)),
            origin()
        ));
        assert_eq!(DragSession::with_payload(|id: &CardId| id.0), Some(1));

        fresh();
    }

    #[test]
    fn only_the_owning_pointer_moves_or_ends_a_drag() {
        fresh();
        DragSession::begin(mouse(), DragPayload::new(CardId(1)), origin());

        DragSession::update(touch(), Vec2d { x: 99.0, y: 99.0 });
        assert_eq!(DragSession::position(), Some(origin()));
        assert!(DragSession::take(touch()).is_none());
        assert!(DragSession::is_active());

        DragSession::update(mouse(), Vec2d { x: 5.0, y: 6.0 });
        assert_eq!(DragSession::position(), Some(Vec2d { x: 5.0, y: 6.0 }));
        assert_eq!(DragSession::origin(), Some(origin()));

        fresh();
    }

    #[test]
    fn cancelling_clears_everything() {
        fresh();
        DragSession::begin(mouse(), DragPayload::new(CardId(1)), origin());

        DragSession::cancel(mouse());

        assert!(!DragSession::is_active());
        assert_eq!(DragSession::position(), None);
        assert_eq!(DragSession::pointer(), None);
        assert_eq!(DragSession::with_payload(|id: &CardId| id.0), None);
    }

    #[test]
    fn a_file_drag_accumulates_its_paths() {
        fresh();
        DragSession::begin(
            mouse(),
            DragPayload::new(FileDrop::new(vec![PathBuf::from("a.png")])),
            origin(),
        );

        DragSession::with_payload_mut(|drop: &mut FileDrop| {
            drop.paths.push(PathBuf::from("b.png"))
        });

        assert_eq!(
            DragSession::with_payload(|drop: &FileDrop| drop.paths.len()),
            Some(2)
        );

        fresh();
    }
}
