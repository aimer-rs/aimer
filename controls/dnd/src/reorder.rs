//! Stable-key reorder transactions for drag and alternate input paths.
//!
//! The model is deliberately independent of widgets and layout. A drag holds
//! a pointer-bound transaction, previews an insertion slot without mutating
//! the list, and commits or cancels exactly once. Moving an item moves its
//! value with its stable key, so state stored in the value cannot accidentally
//! follow a list index instead.

use std::collections::HashSet;
use std::fmt;

use aimer_widget::PointerKey;

/// Maximum UTF-8 byte length accepted for one stable item key.
pub const MAX_STABLE_KEY_BYTES: usize = 256;

/// A validated identity for a reorderable item.
///
/// Keys are compared literally. They must be non-empty, no longer than
/// [`MAX_STABLE_KEY_BYTES`], and contain no control characters. The validation
/// is intentionally small and deterministic: callers remain responsible for
/// choosing an identity that stays stable across rebuilds.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct StableKey(String);

impl StableKey {
    /// Validates and stores `value` as a stable key.
    pub fn try_new(value: impl AsRef<str>) -> Result<Self, StableKeyError> {
        let value = value.as_ref();
        if value.is_empty() {
            return Err(StableKeyError::Empty);
        }
        if value.len() > MAX_STABLE_KEY_BYTES {
            return Err(StableKeyError::TooLong);
        }
        if value.chars().any(char::is_control) {
            return Err(StableKeyError::ControlCharacter);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the literal key text.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for StableKey {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for StableKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a stable key failed validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StableKeyError {
    /// The key has no identity text.
    Empty,
    /// The key exceeds [`MAX_STABLE_KEY_BYTES`].
    TooLong,
    /// The key contains a control character.
    ControlCharacter,
}

impl fmt::Display for StableKeyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "stable key must not be empty",
            Self::TooLong => "stable key is too long",
            Self::ControlCharacter => "stable key contains a control character",
        };
        f.write_str(message)
    }
}

impl std::error::Error for StableKeyError {}

/// One value in a [`ReorderableList`].
#[derive(Clone, Debug, PartialEq)]
pub struct ReorderItem<T> {
    key: StableKey,
    value: T,
}

impl<T> ReorderItem<T> {
    /// Creates an item with an already validated stable key.
    #[inline]
    pub fn new(key: StableKey, value: T) -> Self {
        Self { key, value }
    }

    /// Validates a text key and creates an item.
    #[inline]
    pub fn try_new(key: impl AsRef<str>, value: T) -> Result<Self, StableKeyError> {
        Ok(Self::new(StableKey::try_new(key)?, value))
    }

    /// Returns the item's stable identity.
    #[inline]
    pub fn key(&self) -> &StableKey {
        &self.key
    }

    /// Returns the value retained with the item.
    #[inline]
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Returns a mutable reference to the retained value.
    #[inline]
    pub fn value_mut(&mut self) -> &mut T {
        &mut self.value
    }

    /// Splits the item into its stable identity and retained value.
    #[inline]
    pub fn into_parts(self) -> (StableKey, T) {
        (self.key, self.value)
    }
}

/// A target relative to the current item order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropLocation {
    /// Insert before the item at `index`.
    Before(usize),
    /// Insert after the item at `index`.
    After(usize),
    /// Insert at a post-removal slot in the range `0..=len`.
    Index(usize),
}

/// The insertion line a drag should paint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InsertionIndicator {
    source_index: usize,
    insertion_index: usize,
    location: DropLocation,
}

impl InsertionIndicator {
    /// The index occupied by the dragged item after a commit.
    #[inline]
    pub const fn insertion_index(self) -> usize {
        self.insertion_index
    }

    /// The item's index before the drag started.
    #[inline]
    pub const fn source_index(self) -> usize {
        self.source_index
    }

    /// The location used to produce this indicator.
    #[inline]
    pub const fn location(self) -> DropLocation {
        self.location
    }

    /// Whether the indicator leaves the item at its current index.
    #[inline]
    pub const fn is_noop(self) -> bool {
        self.source_index == self.insertion_index
    }
}

/// A pointer-bound reorder transaction handle.
///
/// It is copyable so cleanup paths can safely attempt cancellation after a
/// pointer-up, pointer-exit, or global cancel without consuming bookkeeping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReorderDrag {
    id: u64,
    pointer: PointerKey,
}

impl ReorderDrag {
    /// The pointer that owns this transaction.
    #[inline]
    pub const fn pointer(self) -> PointerKey {
        self.pointer
    }
}

/// The result of a committed or keyboard reorder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReorderOutcome {
    key: StableKey,
    from: usize,
    to: usize,
    moved: bool,
}

impl ReorderOutcome {
    /// The stable key that moved or was confirmed in place.
    #[inline]
    pub fn key(&self) -> &StableKey {
        &self.key
    }

    /// The source index.
    #[inline]
    pub const fn from(&self) -> usize {
        self.from
    }

    /// The destination index after removal and insertion.
    #[inline]
    pub const fn to(&self) -> usize {
        self.to
    }

    /// Whether the order changed.
    #[inline]
    pub const fn moved(&self) -> bool {
        self.moved
    }
}

/// A keyboard/alternate-input reorder command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyboardReorder {
    /// Move one slot toward the start.
    Previous,
    /// Move one slot toward the end.
    Next,
    /// Move to the first slot.
    First,
    /// Move to the final slot.
    Last,
}

/// Errors returned by reorder operations.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReorderError {
    /// Two entries use the same stable key.
    DuplicateKey(StableKey),
    /// The requested key is not in the list.
    UnknownKey(StableKey),
    /// An item is already being reordered by another pointer.
    ConcurrentDrag,
    /// A transaction no longer names the active drag.
    StaleDrag,
    /// A before/after target is outside the current list, or an index is past
    /// the inclusive end slot.
    InvalidLocation,
}

impl fmt::Display for ReorderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateKey(key) => write!(f, "duplicate stable key: {key}"),
            Self::UnknownKey(key) => write!(f, "unknown stable key: {key}"),
            Self::ConcurrentDrag => f.write_str("another pointer owns the reorder"),
            Self::StaleDrag => f.write_str("reorder transaction is no longer active"),
            Self::InvalidLocation => f.write_str("reorder location is outside the list"),
        }
    }
}

impl std::error::Error for ReorderError {}

struct ActiveDrag {
    id: u64,
    pointer: PointerKey,
    key: StableKey,
    source_index: usize,
}

/// A stable-key list with one pointer-bound transaction at a time.
pub struct ReorderableList<T> {
    items: Vec<ReorderItem<T>>,
    active: Option<ActiveDrag>,
    next_drag_id: u64,
}

impl<T> ReorderableList<T> {
    /// Creates a list, rejecting duplicate stable keys before any drag starts.
    pub fn try_new(items: Vec<ReorderItem<T>>) -> Result<Self, ReorderError> {
        let mut keys = HashSet::with_capacity(items.len());
        for item in &items {
            if !keys.insert(item.key.clone()) {
                return Err(ReorderError::DuplicateKey(item.key.clone()));
            }
        }
        Ok(Self {
            items,
            active: None,
            next_drag_id: 1,
        })
    }

    /// The current item order.
    #[inline]
    pub fn items(&self) -> &[ReorderItem<T>] {
        &self.items
    }

    /// The number of items.
    #[inline]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether the list is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Finds the current index for `key`.
    #[inline]
    pub fn position_of(&self, key: &StableKey) -> Option<usize> {
        self.items.iter().position(|item| item.key == *key)
    }

    /// The pointer currently owning a reorder, if any.
    #[inline]
    pub fn active_pointer(&self) -> Option<PointerKey> {
        self.active.as_ref().map(|drag| drag.pointer)
    }

    /// Begins a single-pointer reorder transaction for `key`.
    pub fn begin_drag(
        &mut self,
        pointer: PointerKey,
        key: &StableKey,
    ) -> Result<ReorderDrag, ReorderError> {
        if self.active.is_some() {
            return Err(ReorderError::ConcurrentDrag);
        }
        let source_index = self
            .position_of(key)
            .ok_or_else(|| ReorderError::UnknownKey(key.clone()))?;
        let id = self.next_drag_id;
        self.next_drag_id = self.next_drag_id.wrapping_add(1).max(1);
        self.active = Some(ActiveDrag {
            id,
            pointer,
            key: key.clone(),
            source_index,
        });
        Ok(ReorderDrag { id, pointer })
    }

    /// Computes an insertion indicator without changing list order.
    pub fn preview(
        &self,
        drag: ReorderDrag,
        location: DropLocation,
    ) -> Result<InsertionIndicator, ReorderError> {
        let active = self.active_drag(drag)?;
        let insertion_index = self.insertion_index(location)?;
        let insertion_index = if insertion_index > active.source_index {
            insertion_index - 1
        } else {
            insertion_index
        };
        Ok(InsertionIndicator {
            source_index: active.source_index,
            insertion_index,
            location,
        })
    }

    /// Commits a transaction at `location` and moves the value with its key.
    pub fn commit_drag(
        &mut self,
        drag: ReorderDrag,
        location: DropLocation,
    ) -> Result<ReorderOutcome, ReorderError> {
        let active = self.active_drag(drag)?;
        let destination = self.preview(drag, location)?.insertion_index;
        let source = active.source_index;
        let key = active.key.clone();
        self.active = None;

        let moved = source != destination;
        if moved {
            let item = self.items.remove(source);
            self.items.insert(destination, item);
        }
        Ok(ReorderOutcome {
            key,
            from: source,
            to: destination,
            moved,
        })
    }

    /// Cancels the active transaction if `drag` still owns it.
    #[inline]
    pub fn cancel_drag(&mut self, drag: ReorderDrag) -> bool {
        if self.active_drag(drag).is_ok() {
            self.active = None;
            true
        } else {
            false
        }
    }

    /// Cancels the transaction owned by `pointer`, for pointer-exit and lost-
    /// pointer cleanup paths.
    #[inline]
    pub fn cancel_pointer(&mut self, pointer: PointerKey) -> bool {
        if self
            .active
            .as_ref()
            .is_some_and(|drag| drag.pointer == pointer)
        {
            self.active = None;
            true
        } else {
            false
        }
    }

    /// Cancels any active transaction, for window-level cancellation.
    #[inline]
    pub fn cancel_all(&mut self) -> bool {
        self.active.take().is_some()
    }

    /// Applies an immediate keyboard/alternate-input move.
    pub fn keyboard_move(
        &mut self,
        key: &StableKey,
        command: KeyboardReorder,
    ) -> Result<ReorderOutcome, ReorderError> {
        if self.active.is_some() {
            return Err(ReorderError::ConcurrentDrag);
        }
        let source = self
            .position_of(key)
            .ok_or_else(|| ReorderError::UnknownKey(key.clone()))?;
        let last = self.items.len().saturating_sub(1);
        let destination = match command {
            KeyboardReorder::Previous => source.saturating_sub(1),
            KeyboardReorder::Next => source.saturating_add(1).min(last),
            KeyboardReorder::First => 0,
            KeyboardReorder::Last => last,
        };
        let moved = source != destination;
        if moved {
            let item = self.items.remove(source);
            self.items.insert(destination, item);
        }
        Ok(ReorderOutcome {
            key: key.clone(),
            from: source,
            to: destination,
            moved,
        })
    }

    fn active_drag(&self, drag: ReorderDrag) -> Result<&ActiveDrag, ReorderError> {
        self.active
            .as_ref()
            .filter(|active| active.id == drag.id && active.pointer == drag.pointer)
            .ok_or(ReorderError::StaleDrag)
    }

    fn insertion_index(&self, location: DropLocation) -> Result<usize, ReorderError> {
        match location {
            DropLocation::Before(index) | DropLocation::After(index)
                if index >= self.items.len() =>
            {
                Err(ReorderError::InvalidLocation)
            }
            DropLocation::Before(index) => Ok(index),
            DropLocation::After(index) => Ok(index + 1),
            DropLocation::Index(index) if index > self.items.len() => {
                Err(ReorderError::InvalidLocation)
            }
            DropLocation::Index(index) => Ok(index),
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_events::pointer::PointerSource;
    use aimer_widget::PointerKey;

    use super::*;

    fn pointer(id: u64) -> PointerKey {
        PointerKey::new(PointerSource::Touch, id)
    }

    fn key(value: &str) -> StableKey {
        StableKey::try_new(value).expect("test keys are valid")
    }

    fn list() -> ReorderableList<String> {
        ReorderableList::try_new(vec![
            ReorderItem::new(key("a"), "alpha".to_owned()),
            ReorderItem::new(key("b"), "bravo".to_owned()),
            ReorderItem::new(key("c"), "charlie".to_owned()),
        ])
        .expect("test keys are unique")
    }

    fn keys(list: &ReorderableList<String>) -> Vec<&str> {
        list.items()
            .iter()
            .map(|item| item.key().as_str())
            .collect()
    }

    #[test]
    fn preview_reports_before_after_and_boundary_insertion_slots() {
        let mut list = list();
        let drag = list
            .begin_drag(pointer(1), &key("b"))
            .expect("b can be dragged");

        assert_eq!(
            list.preview(drag, DropLocation::Before(0))
                .expect("before is valid")
                .insertion_index(),
            0
        );
        assert_eq!(
            list.preview(drag, DropLocation::After(2))
                .expect("after is valid")
                .insertion_index(),
            2
        );
        assert_eq!(
            list.preview(drag, DropLocation::Index(3))
                .expect("the end slot is valid")
                .insertion_index(),
            2
        );
    }

    #[test]
    fn committing_moves_the_key_and_its_state_as_one_item() {
        let mut list = list();
        let drag = list
            .begin_drag(pointer(1), &key("b"))
            .expect("b can be dragged");

        let outcome = list
            .commit_drag(drag, DropLocation::After(2))
            .expect("the drop is valid");

        assert_eq!(keys(&list), vec!["a", "c", "b"]);
        assert_eq!(list.items()[2].value(), "bravo");
        assert_eq!(outcome.from(), 1);
        assert_eq!(outcome.to(), 2);
        assert!(outcome.moved());
        assert_eq!(list.active_pointer(), None);
    }

    #[test]
    fn dropping_at_the_current_slot_is_a_valid_no_op() {
        let mut list = list();
        let drag = list
            .begin_drag(pointer(1), &key("a"))
            .expect("a can be dragged");

        let outcome = list
            .commit_drag(drag, DropLocation::Before(0))
            .expect("the first slot is valid");

        assert_eq!(keys(&list), vec!["a", "b", "c"]);
        assert!(!outcome.moved());
        assert_eq!(outcome.from(), 0);
        assert_eq!(outcome.to(), 0);
    }

    #[test]
    fn cancellation_restores_the_original_order_and_invalidates_the_handle() {
        let mut list = list();
        let drag = list
            .begin_drag(pointer(1), &key("b"))
            .expect("b can be dragged");

        assert!(list.cancel_drag(drag));
        assert_eq!(keys(&list), vec!["a", "b", "c"]);
        assert!(!list.cancel_drag(drag));
        assert!(matches!(
            list.commit_drag(drag, DropLocation::Index(0)),
            Err(ReorderError::StaleDrag)
        ));
    }

    #[test]
    fn duplicate_and_invalid_keys_are_rejected_before_dragging() {
        assert!(matches!(StableKey::try_new(""), Err(StableKeyError::Empty)));
        assert!(matches!(
            StableKey::try_new("has\ncontrol"),
            Err(StableKeyError::ControlCharacter)
        ));
        assert!(matches!(
            ReorderableList::try_new(vec![
                ReorderItem::new(key("same"), 1),
                ReorderItem::new(key("same"), 2),
            ]),
            Err(ReorderError::DuplicateKey(_))
        ));
    }

    #[test]
    fn a_second_pointer_is_refused_and_a_lost_pointer_can_be_cancelled() {
        let mut list = list();
        let first = list
            .begin_drag(pointer(1), &key("a"))
            .expect("the first pointer owns the list");

        assert!(matches!(
            list.begin_drag(pointer(2), &key("b")),
            Err(ReorderError::ConcurrentDrag)
        ));
        assert!(list.cancel_pointer(pointer(1)));
        assert_eq!(list.active_pointer(), None);
        assert!(!list.cancel_pointer(pointer(1)));
        assert!(list.begin_drag(pointer(2), &key("b")).is_ok());
        assert!(!list.cancel_drag(first));
        assert!(list.cancel_pointer(pointer(2)));
    }

    #[test]
    fn keyboard_reordering_is_a_bounded_alternate_input_path() {
        let mut list = list();

        let outcome = list
            .keyboard_move(&key("b"), KeyboardReorder::Next)
            .expect("b can move down");
        assert_eq!(keys(&list), vec!["a", "c", "b"]);
        assert!(outcome.moved());

        let outcome = list
            .keyboard_move(&key("b"), KeyboardReorder::Last)
            .expect("last is a valid destination");
        assert!(!outcome.moved(), "b is already last");

        let outcome = list
            .keyboard_move(&key("a"), KeyboardReorder::Previous)
            .expect("the first item has a valid no-op");
        assert!(!outcome.moved());
    }
}
