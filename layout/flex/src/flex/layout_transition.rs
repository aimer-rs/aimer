use aimer_animation::layout::{
    KeyedLayoutError, KeyedLayoutSnapshot, LayoutGeometry, LayoutTransitionError,
};
use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
use aimer_widget::Key;

/// Geometry published by one materialized Flex child.
#[derive(Clone, Debug, PartialEq)]
pub struct FlexItemGeometry {
    key: Key,
    geometry: LayoutGeometry,
}

impl FlexItemGeometry {
    /// Creates a keyed child geometry record.
    #[inline]
    pub fn new(key: Key, geometry: LayoutGeometry) -> Self {
        Self { key, geometry }
    }

    /// Creates a record from the position and resolved size already produced by
    /// the Flex layout engine.
    pub fn from_resolved(
        key: Key,
        position: Vec2d,
        size: ResolvedSize,
    ) -> Result<Self, KeyedLayoutError> {
        Ok(Self::new(
            key,
            LayoutGeometry::try_new(position.x, position.y, size.width, size.height)?,
        ))
    }

    /// Returns the child identity.
    #[inline]
    pub fn key(&self) -> &Key {
        &self.key
    }

    /// Returns the child rectangle.
    #[inline]
    pub const fn geometry(&self) -> LayoutGeometry {
        self.geometry
    }
}

/// A validated Flex container rectangle plus the keyed child rectangles that
/// were materialized for that frame.
#[derive(Clone, Debug, PartialEq)]
pub struct FlexGeometrySnapshot {
    container: LayoutGeometry,
    children: KeyedLayoutSnapshot<Key>,
}

/// Errors returned by the Flex layout-transition adapter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlexSnapshotError {
    /// A child or container geometry was invalid.
    Keyed(KeyedLayoutError),
    /// A transition configuration was invalid.
    Transition(LayoutTransitionError),
    /// A materialized child did not publish a stable key.
    MissingKey,
}

impl std::fmt::Display for FlexSnapshotError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keyed(error) => error.fmt(formatter),
            Self::Transition(error) => error.fmt(formatter),
            Self::MissingKey => formatter.write_str("Flex transition items require a stable key"),
        }
    }
}

impl std::error::Error for FlexSnapshotError {}

impl From<KeyedLayoutError> for FlexSnapshotError {
    fn from(error: KeyedLayoutError) -> Self {
        Self::Keyed(error)
    }
}

impl From<LayoutTransitionError> for FlexSnapshotError {
    fn from(error: LayoutTransitionError) -> Self {
        Self::Transition(error)
    }
}

impl FlexGeometrySnapshot {
    /// Creates a snapshot from keyed, materialized child geometry.
    pub fn try_new(
        container: LayoutGeometry,
        children: impl IntoIterator<Item = FlexItemGeometry>,
    ) -> Result<Self, FlexSnapshotError> {
        container
            .validate()
            .map_err(KeyedLayoutError::from)
            .map_err(FlexSnapshotError::from)?;
        let children = KeyedLayoutSnapshot::try_new(
            children
                .into_iter()
                .map(|child| aimer_animation::layout::LayoutItem::new(child.key, child.geometry)),
        )?;
        Ok(Self {
            container,
            children,
        })
    }

    /// Creates a snapshot from an optional-key child adapter and reports the
    /// first child without identity instead of guessing by index.
    pub fn try_new_optional(
        container: LayoutGeometry,
        children: impl IntoIterator<Item = Option<FlexItemGeometry>>,
    ) -> Result<Self, FlexSnapshotError> {
        container
            .validate()
            .map_err(KeyedLayoutError::from)
            .map_err(FlexSnapshotError::from)?;
        let children = KeyedLayoutSnapshot::try_new_optional(children.into_iter().map(|child| {
            child.map(|child| aimer_animation::layout::LayoutItem::new(child.key, child.geometry))
        }))?;
        Ok(Self {
            container,
            children,
        })
    }

    /// Returns the container rectangle.
    #[inline]
    pub const fn container(&self) -> LayoutGeometry {
        self.container
    }

    /// Returns the keyed children in Flex paint/order sequence.
    #[inline]
    pub fn children(&self) -> &KeyedLayoutSnapshot<Key> {
        &self.children
    }
}

#[cfg(test)]
mod tests {
    use aimer_animation::layout::LayoutGeometry;
    use aimer_widget::Key;

    use super::*;

    #[test]
    fn flex_snapshot_retains_geometry_and_rejects_missing_identity() {
        let container = LayoutGeometry::try_new(0.0, 0.0, 320.0, 120.0).unwrap();
        let child = LayoutGeometry::try_new(24.0, 8.0, 100.0, 40.0).unwrap();
        let snapshot = FlexGeometrySnapshot::try_new(
            container,
            vec![FlexItemGeometry::new(Key::from("row-1"), child)],
        )
        .unwrap();

        assert_eq!(snapshot.container(), container);
        assert_eq!(snapshot.children().len(), 1);
        assert_eq!(snapshot.children().items()[0].geometry(), child);
        assert!(matches!(
            FlexGeometrySnapshot::try_new_optional(container, vec![None]),
            Err(FlexSnapshotError::Keyed(KeyedLayoutError::MissingKey))
        ));
    }
}
