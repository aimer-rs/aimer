//! Keyed collection identity and bounded viewport windows.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::ops::Range;

/// A datum paired with the identity used to retain its row state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionItem<K, T> {
    key: K,
    value: T,
}

impl<K, T> CollectionItem<K, T> {
    /// Creates a collection item with a stable `key` and its `value`.
    pub fn new(key: K, value: T) -> Self {
        Self { key, value }
    }

    /// Returns the stable identity of this item.
    pub fn key(&self) -> &K {
        &self.key
    }

    /// Returns the value carried by this item.
    pub fn value(&self) -> &T {
        &self.value
    }

    /// Splits this item into its identity and value.
    pub fn into_parts(self) -> (K, T) {
        (self.key, self.value)
    }
}

/// The externally observable state of a collection request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionStatus {
    /// There are no items to display.
    Empty,
    /// The collection is waiting for a data source to finish.
    Loading,
    /// The collection has at least one item.
    Ready,
    /// The most recent request failed.
    Error,
}

/// A collection mutation failed before changing the existing collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CollectionError<K> {
    /// Two items in one replacement had the same stable key.
    DuplicateKey(K),
    /// A state operation referred to a key that is not currently present.
    UnknownKey(K),
}

/// A viewport/window specification is invalid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowError {
    /// The item extent must be finite and strictly positive.
    InvalidItemExtent,
    /// The viewport offset must be finite and non-negative.
    InvalidViewportOffset,
    /// The viewport extent must be finite and non-negative.
    InvalidViewportExtent,
    /// A bounded window must allow at least one item.
    InvalidMaxItems,
    /// A visible range was constructed with its end before its start.
    InvalidRange,
}

/// A half-open item-index range, `[start, end)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibleRange {
    start: usize,
    end: usize,
}

impl VisibleRange {
    /// Creates a range from half-open bounds.
    pub fn new(start: usize, end: usize) -> Result<Self, WindowError> {
        if end < start {
            return Err(WindowError::InvalidRange);
        }
        Ok(Self { start, end })
    }

    #[inline]
    fn from_validated(start: usize, end: usize) -> Self {
        debug_assert!(end >= start);
        Self { start, end }
    }

    /// Returns the first item index included in this range.
    pub fn start(&self) -> usize {
        self.start
    }

    /// Returns the exclusive item index at the end of this range.
    pub fn end(&self) -> usize {
        self.end
    }

    /// Returns the number of item indices in this range.
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    /// Returns whether `index` belongs to this half-open range.
    pub fn contains(&self, index: usize) -> bool {
        self.start <= index && index < self.end
    }

    /// Returns whether the range contains no item indices.
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Returns the standard-library range represented by this value.
    pub fn as_range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// Fixed-extent viewport inputs used to calculate a bounded item window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WindowSpec {
    item_extent: f32,
    viewport_offset: f32,
    viewport_extent: f32,
    overscan: usize,
    max_items: Option<usize>,
}

impl WindowSpec {
    /// Creates a viewport specification.
    ///
    /// `item_extent` is the outer extent of one row. `viewport_offset` and
    /// `viewport_extent` are expressed in the same logical units as the
    /// existing scroll viewport.
    pub fn new(
        item_extent: f32,
        viewport_offset: f32,
        viewport_extent: f32,
    ) -> Result<Self, WindowError> {
        if !item_extent.is_finite() || item_extent <= 0.0 {
            return Err(WindowError::InvalidItemExtent);
        }
        if !viewport_offset.is_finite() || viewport_offset < 0.0 {
            return Err(WindowError::InvalidViewportOffset);
        }
        if !viewport_extent.is_finite() || viewport_extent < 0.0 {
            return Err(WindowError::InvalidViewportExtent);
        }
        Ok(Self {
            item_extent,
            viewport_offset,
            viewport_extent,
            overscan: 0,
            max_items: None,
        })
    }

    /// Adds the same number of item indices before and after the visible range.
    pub fn with_overscan(mut self, overscan: usize) -> Self {
        self.overscan = overscan;
        self
    }

    /// Caps the materialized window at `max_items`.
    pub fn with_max_items(mut self, max_items: usize) -> Result<Self, WindowError> {
        if max_items == 0 {
            return Err(WindowError::InvalidMaxItems);
        }
        self.max_items = Some(max_items);
        Ok(self)
    }

    /// Returns the fixed extent used by this specification.
    pub fn item_extent(&self) -> f32 {
        self.item_extent
    }

    /// Returns the logical offset of the viewport.
    pub fn viewport_offset(&self) -> f32 {
        self.viewport_offset
    }

    /// Returns the logical extent of the viewport.
    pub fn viewport_extent(&self) -> f32 {
        self.viewport_extent
    }

    /// Returns the configured item overscan on each side.
    pub fn overscan(&self) -> usize {
        self.overscan
    }

    /// Returns the optional materialization cap.
    pub fn max_items(&self) -> Option<usize> {
        self.max_items
    }

    /// Calculates a bounded half-open window for `item_count` items.
    ///
    /// The calculation is arithmetic in the item extent and does not inspect
    /// any item values. A zero-sized viewport is an empty range at its offset;
    /// an offset beyond the collection ends at `item_count`.
    pub fn range(&self, item_count: usize) -> VisibleRange {
        if item_count == 0 {
            return VisibleRange::from_validated(0, 0);
        }

        let extent = f64::from(self.item_extent);
        let offset = f64::from(self.viewport_offset);
        let viewport = f64::from(self.viewport_extent);
        let total_extent = extent * item_count as f64;
        let base_start = if offset >= total_extent {
            item_count
        } else {
            (offset / extent).floor() as usize
        };

        if viewport == 0.0 || base_start == item_count {
            return VisibleRange::from_validated(base_start, base_start);
        }

        let end_position = offset + viewport;
        let base_end = if end_position >= total_extent {
            item_count
        } else {
            (end_position / extent).ceil() as usize
        }
        .max(base_start)
        .min(item_count);

        let overscanned_start = base_start.saturating_sub(self.overscan);
        let overscanned_end = base_end.saturating_add(self.overscan).min(item_count);
        let (mut start, mut end) = (overscanned_start, overscanned_end);

        if let Some(max_items) = self.max_items {
            if end - start > max_items {
                // Keep the visible leading edge as the deterministic anchor,
                // then move the window back only when the anchor is near the
                // collection end. The cap always wins over overscan.
                start = base_start.min(item_count.saturating_sub(max_items));
                end = start.saturating_add(max_items).min(item_count);
            }
        }

        VisibleRange::from_validated(start, end)
    }
}

/// A borrowed, bounded window into a keyed collection.
#[derive(Debug)]
pub struct CollectionWindow<'a, K, T> {
    range: VisibleRange,
    items: &'a [CollectionItem<K, T>],
}

impl<'a, K, T> CollectionWindow<'a, K, T> {
    /// Returns the item-index range represented by this window.
    pub fn range(&self) -> VisibleRange {
        self.range
    }

    /// Returns the borrowed items in window order.
    pub fn items(&self) -> &'a [CollectionItem<K, T>] {
        self.items
    }

    /// Returns the number of materialized items in this window.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether this window has no materialized items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterates over stable keys in window order.
    pub fn keys(&self) -> impl Iterator<Item = &'a K> {
        self.items.iter().map(CollectionItem::key)
    }
}

/// A keyed collection model with state retained by item identity.
pub struct CollectionModel<K, T, S = ()> {
    items: Vec<CollectionItem<K, T>>,
    states: HashMap<K, S>,
    status: CollectionStatus,
    error: Option<String>,
}

impl<K, T, S> Default for CollectionModel<K, T, S>
where
    K: Eq + Hash + Clone,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<K, T, S> CollectionModel<K, T, S>
where
    K: Eq + Hash + Clone,
{
    /// Creates an empty collection with no retained item state.
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            states: HashMap::new(),
            status: CollectionStatus::Empty,
            error: None,
        }
    }

    /// Replaces the items, preserving state for keys that remain present.
    ///
    /// Duplicate keys are rejected atomically, so callers can safely retry a
    /// failed data update without losing the previous visible collection.
    pub fn set_items(
        &mut self,
        items: impl IntoIterator<Item = CollectionItem<K, T>>,
    ) -> Result<(), CollectionError<K>> {
        self.replace_items(items)
    }

    /// Replaces the items, preserving state for keys that remain present.
    pub fn replace_items(
        &mut self,
        items: impl IntoIterator<Item = CollectionItem<K, T>>,
    ) -> Result<(), CollectionError<K>> {
        let items: Vec<_> = items.into_iter().collect();
        let mut seen = HashSet::with_capacity(items.len());
        for item in &items {
            if !seen.insert(&item.key) {
                return Err(CollectionError::DuplicateKey(item.key.clone()));
            }
        }

        let present: HashSet<K> = items.iter().map(|item| item.key.clone()).collect();
        self.states.retain(|key, _| present.contains(key));
        self.items = items;
        self.error = None;
        self.status = if self.items.is_empty() {
            CollectionStatus::Empty
        } else {
            CollectionStatus::Ready
        };
        Ok(())
    }

    /// Starts a loading state without requiring a second collection model.
    pub fn begin_loading(&mut self) {
        self.status = CollectionStatus::Loading;
        self.error = None;
    }

    /// Records an error state with a user-facing message.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.status = CollectionStatus::Error;
        self.error = Some(message.into());
    }

    /// Returns the current collection status.
    pub fn status(&self) -> CollectionStatus {
        self.status
    }

    /// Returns the current error message, if the collection failed.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Returns the number of logical items, including items outside a window.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Returns whether the logical collection has no items.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Iterates over all stable keys in source order.
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.items.iter().map(CollectionItem::key)
    }

    /// Looks up an item by stable key.
    pub fn item(&self, key: &K) -> Option<&CollectionItem<K, T>> {
        self.items.iter().find(|item| &item.key == key)
    }

    /// Returns the source index of a stable key.
    pub fn position(&self, key: &K) -> Option<usize> {
        self.items.iter().position(|item| &item.key == key)
    }

    /// Sets retained state for an item that is currently present.
    pub fn set_state(&mut self, key: K, state: S) -> Result<(), CollectionError<K>> {
        if !self.items.iter().any(|item| item.key == key) {
            return Err(CollectionError::UnknownKey(key));
        }
        self.states.insert(key, state);
        Ok(())
    }

    /// Returns retained state for an item key.
    pub fn state(&self, key: &K) -> Option<&S> {
        self.states.get(key)
    }

    /// Returns mutable retained state for an item key.
    pub fn state_mut(&mut self, key: &K) -> Option<&mut S> {
        self.states.get_mut(key)
    }

    /// Calculates a bounded borrowed window over the logical items.
    pub fn window(&self, spec: &WindowSpec) -> Result<CollectionWindow<'_, K, T>, WindowError> {
        let range = spec.range(self.items.len());
        Ok(CollectionWindow {
            range,
            items: &self.items[range.as_range()],
        })
    }

    /// Resolves the current loading/empty/error/content presentation slot.
    pub fn slot(&self, spec: &WindowSpec) -> Result<CollectionSlot<'_, K, T>, WindowError> {
        match self.status {
            CollectionStatus::Empty => Ok(CollectionSlot::Empty),
            CollectionStatus::Loading => Ok(CollectionSlot::Loading),
            CollectionStatus::Error => Ok(CollectionSlot::Error(
                self.error.as_deref().unwrap_or("collection failed"),
            )),
            CollectionStatus::Ready => Ok(CollectionSlot::Items(self.window(spec)?)),
        }
    }
}

/// The slot a data-view renderer should present for one collection request.
#[derive(Debug)]
pub enum CollectionSlot<'a, K, T> {
    /// The collection has no items.
    Empty,
    /// The collection is still loading.
    Loading,
    /// The collection failed with a message.
    Error(&'a str),
    /// The collection has ready items in the requested bounded window.
    Items(CollectionWindow<'a, K, T>),
}
