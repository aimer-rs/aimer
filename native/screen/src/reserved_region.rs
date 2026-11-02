//! Turning UIKit's division regions into [`DisplayFeature`]s.
//!
//! UIKit has no list of folds. It has `reservedRegions`, rectangles the system
//! keeps for itself in a view, of two kinds: an *occlusion* (a camera, the
//! Dynamic Island) and a *division*, where content should split in two — the
//! fold of a foldable iPhone. Only divisions are display features here; the
//! occlusions are not folds.
//!
//! The translation is kept apart from the FFI that fetches the rectangles
//! because it is the part that can be stated exactly, and so tested on any host.

use crate::{Bounds, DisplayFeature, FoldState};

/// A division region as UIKit reports it, in the queried view's points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
pub(crate) struct DivisionRegion {
    /// `frame.origin.x`.
    pub(crate) x: f64,
    /// `frame.origin.y`.
    pub(crate) y: f64,
    /// `frame.size.width`.
    pub(crate) width: f64,
    /// `frame.size.height`.
    pub(crate) height: f64,
    /// `isActive`: whether the device is bent far enough for it to divide.
    pub(crate) active: bool,
}

/// Translates the division regions of a view into display features.
///
/// UIKit hands out the *inactive* regions too (when asked), and those are kept:
/// a flat feature that does not separate tells a layout where the fold will be,
/// so it can leave room and not jump when the device is folded — the same thing
/// Android reports for a foldable that is open flat.
///
/// The frame already includes the region's margins, which keep interactive
/// content clear of the fold, so it is used as it is. A region with no area or a
/// non-finite edge is not a place content can avoid and is skipped on its own.
///
/// A division is a crease in a continuous display, not a gap: it is reported as
/// a fold (it does not occlude), because UIKit documents it as the place to
/// *divide* content, not the place content is hidden.
#[cfg_attr(not(any(target_os = "ios", test)), allow(dead_code))]
pub(crate) fn features_from_divisions(regions: &[DivisionRegion]) -> Vec<DisplayFeature> {
    regions
        .iter()
        .filter_map(|region| {
            let left = region.x as f32;
            let top = region.y as f32;
            let right = (region.x + region.width) as f32;
            let bottom = (region.y + region.height) as f32;
            let finite = [left, top, right, bottom].iter().all(|edge| edge.is_finite());
            if !finite || right <= left || bottom <= top {
                return None;
            }
            Some(DisplayFeature {
                bounds: Bounds::new(left, top, right, bottom),
                occludes: false,
                state: if region.active {
                    FoldState::HalfOpened
                } else {
                    FoldState::Flat
                },
                separating: region.active,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Posture;

    fn region(x: f64, y: f64, width: f64, height: f64, active: bool) -> DivisionRegion {
        DivisionRegion { x, y, width, height, active }
    }

    #[test]
    fn an_active_vertical_division_is_a_separating_book_fold() {
        let features = features_from_divisions(&[region(430.0, 0.0, 20.0, 956.0, true)]);

        assert_eq!(features.len(), 1);
        let fold = features[0];
        assert_eq!(fold.bounds, Bounds::new(430.0, 0.0, 450.0, 956.0));
        assert!(fold.separating && !fold.occludes);
        assert_eq!(fold.state, FoldState::HalfOpened);
        assert_eq!(fold.posture(), Posture::Book);
    }

    #[test]
    fn an_active_horizontal_division_is_a_tabletop_fold() {
        let features = features_from_divisions(&[region(0.0, 440.0, 440.0, 24.0, true)]);

        assert_eq!(features[0].posture(), Posture::Tabletop);
    }

    #[test]
    fn an_inactive_division_marks_where_the_fold_will_be_without_separating() {
        let features = features_from_divisions(&[region(430.0, 0.0, 20.0, 956.0, false)]);

        assert_eq!(features.len(), 1);
        assert_eq!(features[0].state, FoldState::Flat);
        assert!(!features[0].separating);
        assert_eq!(features[0].posture(), Posture::Flat);
    }

    #[test]
    fn a_region_that_reaches_outside_the_view_keeps_its_negative_edges() {
        // UIKit reports a partial overlap in the view's own coordinates.
        let features = features_from_divisions(&[region(-10.0, 0.0, 30.0, 956.0, true)]);

        assert_eq!(features[0].bounds, Bounds::new(-10.0, 0.0, 20.0, 956.0));
    }

    #[test]
    fn a_region_with_no_area_or_a_bad_edge_is_skipped_alone() {
        let features = features_from_divisions(&[
            region(0.0, 0.0, 0.0, 956.0, true),
            region(0.0, 0.0, 20.0, -4.0, true),
            region(f64::NAN, 0.0, 20.0, 956.0, true),
            region(0.0, 0.0, f64::INFINITY, 956.0, true),
            region(430.0, 0.0, 20.0, 956.0, true),
        ]);

        assert_eq!(features.len(), 1);
        assert_eq!(features[0].bounds.left, 430.0);
    }

    #[test]
    fn no_regions_is_no_features() {
        assert!(features_from_divisions(&[]).is_empty());
    }
}
