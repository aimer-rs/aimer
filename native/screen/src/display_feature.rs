//! Folds and hinges in the display.

/// A rectangle in the window's logical pixels, measured from its top-left.
///
/// Edges, not origin and size, because that is how the platforms report a
/// display feature (`android.graphics.Rect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    /// Left edge.
    pub left: f32,
    /// Top edge.
    pub top: f32,
    /// Right edge.
    pub right: f32,
    /// Bottom edge.
    pub bottom: f32,
}

impl Bounds {
    /// Creates bounds from their four edges.
    #[inline]
    pub const fn new(left: f32, top: f32, right: f32, bottom: f32) -> Self {
        Self { left, top, right, bottom }
    }

    /// Distance between the left and right edges.
    #[inline]
    pub fn width(&self) -> f32 {
        self.right - self.left
    }

    /// Distance between the top and bottom edges.
    #[inline]
    pub fn height(&self) -> f32 {
        self.bottom - self.top
    }
}

/// How far a foldable device is open.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FoldState {
    /// Fully open: one continuous surface.
    Flat,
    /// Open part of the way, standing on its own like a laptop or a book.
    HalfOpened,
}

/// What kind of discontinuity a [`DisplayFeature`] is.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum DisplayFeatureKind {
    /// A bend in a continuous flexible display. Nothing is hidden by it.
    Fold,
    /// A physical hinge between two displays, or a gap in one. The content under
    /// it is hidden: the Microsoft Surface Duo, and any foldable whose hinge
    /// covers part of the screen.
    Hinge,
}

/// The posture a foldable device is in.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Posture {
    /// Fully open, or not folded at all.
    Flat,
    /// Half open with a horizontal fold: standing like a laptop, one half on a
    /// surface and the other facing the user.
    Tabletop,
    /// Half open with a vertical fold: held like a book.
    Book,
}

/// A fold or hinge in the display, in the window's logical pixels.
///
/// Content under a [`Hinge`](DisplayFeatureKind::Hinge) cannot be seen, so a
/// layout should keep text and controls clear of [`bounds`](Self::bounds); when
/// the feature [`separates`](Self::separating) the display into two usable
/// areas, [`split_panes`] gives them.
///
/// # Examples
///
/// ```
/// use aimer_screen::{Bounds, DisplayFeature, FoldState, Posture};
///
/// // A vertical hinge down the middle of a dual-screen phone.
/// let hinge = DisplayFeature {
///     bounds: Bounds::new(1350.0, 0.0, 1434.0, 1800.0),
///     occludes: true,
///     state: FoldState::HalfOpened,
///     separating: true,
/// };
///
/// assert!(hinge.is_vertical());
/// assert_eq!(hinge.posture(), Posture::Book);
/// ```
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DisplayFeature {
    /// Where the feature is in the window.
    pub bounds: Bounds,
    /// Whether it hides the content beneath it.
    pub occludes: bool,
    /// How far open the device is.
    pub state: FoldState,
    /// Whether it divides the display into two areas that should be laid out
    /// independently. A hinge always does; a fold does only while half open.
    pub separating: bool,
}

impl DisplayFeature {
    /// Whether this is a [`Hinge`](DisplayFeatureKind::Hinge) or a
    /// [`Fold`](DisplayFeatureKind::Fold).
    #[inline]
    pub fn kind(&self) -> DisplayFeatureKind {
        if self.occludes {
            DisplayFeatureKind::Hinge
        } else {
            DisplayFeatureKind::Fold
        }
    }

    /// Whether the feature is a tall line, dividing the display left from right.
    #[inline]
    pub fn is_vertical(&self) -> bool {
        self.bounds.height() > self.bounds.width()
    }

    /// Whether the feature is a wide line, dividing the display top from bottom.
    #[inline]
    pub fn is_horizontal(&self) -> bool {
        self.bounds.width() > self.bounds.height()
    }

    /// The posture the device is in, as far as this feature says.
    pub fn posture(&self) -> Posture {
        match self.state {
            FoldState::Flat => Posture::Flat,
            FoldState::HalfOpened if self.is_horizontal() => Posture::Tabletop,
            FoldState::HalfOpened => Posture::Book,
        }
    }
}

/// The two areas a separating feature leaves usable.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Panes {
    /// The left area for a vertical feature, the top one for a horizontal one.
    pub first: Bounds,
    /// The right area for a vertical feature, the bottom one for a horizontal one.
    pub second: Bounds,
}

/// Divides a `width` by `height` window at the first separating feature.
///
/// Returns `None` when no feature separates the window, which is every
/// non-foldable device and a foldable that is fully open. The panes exclude the
/// feature itself, and are clamped to the window so a feature reported slightly
/// outside it cannot produce a pane that is.
///
/// # Examples
///
/// ```
/// use aimer_screen::{Bounds, DisplayFeature, FoldState, split_panes};
///
/// let hinge = DisplayFeature {
///     bounds: Bounds::new(1350.0, 0.0, 1434.0, 1800.0),
///     occludes: true,
///     state: FoldState::HalfOpened,
///     separating: true,
/// };
/// let panes = split_panes(2784.0, 1800.0, &[hinge]).unwrap();
///
/// assert_eq!(panes.first.right, 1350.0);
/// assert_eq!(panes.second.left, 1434.0);
/// ```
pub fn split_panes(width: f32, height: f32, features: &[DisplayFeature]) -> Option<Panes> {
    let feature = features.iter().find(|feature| {
        feature.separating && (feature.bounds.width() > 0.0 || feature.bounds.height() > 0.0)
    })?;
    let bounds = feature.bounds;
    Some(if feature.is_horizontal() {
        Panes {
            first: Bounds::new(0.0, 0.0, width, bounds.top.clamp(0.0, height)),
            second: Bounds::new(0.0, bounds.bottom.clamp(0.0, height), width, height),
        }
    } else {
        Panes {
            first: Bounds::new(0.0, 0.0, bounds.left.clamp(0.0, width), height),
            second: Bounds::new(bounds.right.clamp(0.0, width), 0.0, width, height),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feature(bounds: Bounds, occludes: bool, state: FoldState, separating: bool) -> DisplayFeature {
        DisplayFeature { bounds, occludes, state, separating }
    }

    /// A Surface Duo spanned across both screens: a vertical hinge mid-window.
    fn duo_hinge() -> DisplayFeature {
        feature(Bounds::new(1350.0, 0.0, 1434.0, 1800.0), true, FoldState::HalfOpened, true)
    }

    /// A foldable with its fold laid horizontally, half open.
    fn tabletop_fold() -> DisplayFeature {
        feature(Bounds::new(0.0, 1100.0, 2200.0, 1100.0), false, FoldState::HalfOpened, true)
    }

    #[test]
    fn a_hinge_occludes_and_a_fold_does_not() {
        assert_eq!(duo_hinge().kind(), DisplayFeatureKind::Hinge);
        assert_eq!(tabletop_fold().kind(), DisplayFeatureKind::Fold);
    }

    #[test]
    fn a_feature_is_vertical_or_horizontal_by_its_longer_side() {
        assert!(duo_hinge().is_vertical() && !duo_hinge().is_horizontal());
        assert!(tabletop_fold().is_horizontal() && !tabletop_fold().is_vertical());
    }

    #[test]
    fn posture_follows_state_and_direction() {
        assert_eq!(duo_hinge().posture(), Posture::Book);
        assert_eq!(tabletop_fold().posture(), Posture::Tabletop);
        let flat = feature(Bounds::new(0.0, 1100.0, 2200.0, 1100.0), false, FoldState::Flat, false);
        assert_eq!(flat.posture(), Posture::Flat);
        // A flat hinge is still flat: the posture is the state, not the geometry.
        let flat_hinge = feature(Bounds::new(1350.0, 0.0, 1434.0, 1800.0), true, FoldState::Flat, true);
        assert_eq!(flat_hinge.posture(), Posture::Flat);
    }

    #[test]
    fn a_vertical_hinge_leaves_a_left_and_a_right_pane() {
        let panes = split_panes(2784.0, 1800.0, &[duo_hinge()]).unwrap();
        assert_eq!(panes.first, Bounds::new(0.0, 0.0, 1350.0, 1800.0));
        assert_eq!(panes.second, Bounds::new(1434.0, 0.0, 2784.0, 1800.0));
    }

    #[test]
    fn a_horizontal_fold_leaves_a_top_and_a_bottom_pane() {
        let panes = split_panes(2200.0, 1800.0, &[tabletop_fold()]).unwrap();
        assert_eq!(panes.first, Bounds::new(0.0, 0.0, 2200.0, 1100.0));
        assert_eq!(panes.second, Bounds::new(0.0, 1100.0, 2200.0, 1800.0));
    }

    #[test]
    fn nothing_separates_a_window_without_a_separating_feature() {
        assert_eq!(split_panes(1000.0, 800.0, &[]), None);
        let seam = feature(Bounds::new(500.0, 0.0, 500.0, 800.0), false, FoldState::Flat, false);
        assert_eq!(split_panes(1000.0, 800.0, &[seam]), None);
    }

    #[test]
    fn the_first_separating_feature_wins_and_others_are_skipped() {
        let seam = feature(Bounds::new(100.0, 0.0, 100.0, 800.0), false, FoldState::Flat, false);
        let panes = split_panes(2784.0, 1800.0, &[seam, duo_hinge()]).unwrap();
        assert_eq!(panes.first.right, 1350.0);
    }

    #[test]
    fn panes_never_extend_past_the_window() {
        // Reported a little outside the window, as rounding can do.
        let hinge = feature(Bounds::new(-4.0, 0.0, 10.0, 800.0), true, FoldState::HalfOpened, true);
        let panes = split_panes(1000.0, 800.0, &[hinge]).unwrap();
        assert_eq!(panes.first, Bounds::new(0.0, 0.0, 0.0, 800.0));
        assert_eq!(panes.second, Bounds::new(10.0, 0.0, 1000.0, 800.0));
    }

    #[test]
    fn a_feature_with_no_extent_cannot_separate() {
        let point = feature(Bounds::new(5.0, 5.0, 5.0, 5.0), true, FoldState::HalfOpened, true);
        assert_eq!(split_panes(1000.0, 800.0, &[point]), None);
    }
}
