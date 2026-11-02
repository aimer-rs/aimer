use std::rc::Rc;

use aimer_widget::base::{BuildContext, ResolvedSize};

use crate::screen_state::{watch_device_orientation, watch_display_features, watch_hinge};
use crate::{DeviceOrientation, DisplayFeature, HingeState, Orientation, Panes, split_panes};

/// The window and the screen as the widget being built sees them.
///
/// Reading this from a `build` registers the widget as depending on what it
/// read, so it — and only it — is rebuilt when that changes: the window is
/// resized or moves to a display with a different scale factor, the device is
/// turned, a foldable is opened. Everything else keeps the subtree it already
/// has and is merely laid out again, which is what keeps a window drag smooth on
/// a large tree.
///
/// Each question has its own dependency. A widget that asked
/// [`MediaQuery::device_orientation_of`] is not rebuilt by a resize, one that
/// asked [`MediaQuery::display_features_of`] is not rebuilt by a rotation, and
/// neither of them is rebuilt for every degree of [`MediaQuery::hinge_of`].
///
/// # Examples
///
/// ```ignore
/// fn build(&self, ctx: &BuildContext) -> impl Widget {
///     if MediaQuery::of(ctx).size.width < 600.0 {
///         phone_layout()
///     } else {
///         desktop_layout()
///     }
/// }
/// ```
pub struct MediaQuery {
    /// Client area of the window in logical pixels.
    pub size: ResolvedSize,
    /// Physical pixels per logical pixel.
    pub scale_factor: f32,
}

impl MediaQuery {
    /// Reads the window metrics, subscribing the widget currently building to
    /// every change in them.
    ///
    /// Prefer [`MediaQuery::select`] whenever the build only needs an answer
    /// derived from the window, which is almost always: this subscribes to the
    /// window itself and so rebuilds the widget on every pixel of a drag.
    pub fn of(ctx: &BuildContext) -> Self {
        Self::from_metrics(&ctx.watch_window_metrics())
    }

    /// Answers one question about the window, rebuilding the widget currently
    /// building only when the answer changes.
    ///
    /// A breakpoint is the same for hundreds of window widths in a row, so a
    /// widget that asks for the breakpoint instead of the width sits out the
    /// entire drag and is rebuilt at the one width where its layout differs.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// pub fn is_mobile(ctx: &BuildContext) -> bool {
    ///     MediaQuery::select(ctx, |media| media.size.width < 600.0)
    /// }
    /// ```
    pub fn select<T: Clone + PartialEq + 'static>(
        ctx: &BuildContext,
        selector: impl Fn(&Self) -> T + 'static,
    ) -> T {
        ctx.select_window_metrics(move |metrics| selector(&Self::from_metrics(metrics)))
    }

    /// Whether the window is taller than wide, or wider than tall.
    ///
    /// This is the shape of the *window*; for how the physical device is turned,
    /// see [`MediaQuery::device_orientation_of`].
    #[inline]
    pub fn orientation(&self) -> Orientation {
        Orientation::of_size(self.size.width, self.size.height)
    }

    /// The window's [`Orientation`], rebuilding the widget currently building
    /// only when it flips between portrait and landscape.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let row = MediaQuery::orientation_of(ctx).is_landscape();
    /// ```
    pub fn orientation_of(ctx: &BuildContext) -> Orientation {
        Self::select(ctx, |media| media.orientation())
    }

    /// Which way the physical device is turned, rebuilding the widget currently
    /// building when it is turned.
    ///
    /// `None` when the platform does not say: a desktop computer does not
    /// rotate, and a device lying flat has no direction. It is not the same as
    /// the window's shape — a phone in split-screen can be held in landscape and
    /// show a portrait window — so use [`MediaQuery::orientation_of`] for layout
    /// and this for what depends on the hardware (a camera preview, a game).
    pub fn device_orientation_of(ctx: &BuildContext) -> Option<DeviceOrientation> {
        watch_device_orientation(ctx)
    }

    /// The folds and hinges in the display, rebuilding the widget currently
    /// building when they change.
    ///
    /// Empty on a device that has none, which is nearly all of them. A widget
    /// that reads this on such a device is never rebuilt on its account.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// for feature in MediaQuery::display_features_of(ctx).iter() {
    ///     keep_clear(feature.bounds);
    /// }
    /// ```
    pub fn display_features_of(ctx: &BuildContext) -> Rc<[DisplayFeature]> {
        watch_display_features(ctx)
    }

    /// How far open the hinge of a foldable device is, rebuilding the widget
    /// currently building whenever it moves.
    ///
    /// `None` on a device without a hinge, and where the platform does not
    /// report one. The angle updates continuously while someone moves the
    /// hinge, so read this only in a widget whose *content* follows the angle
    /// (an instrument, an effect). A layout that only needs to know where the
    /// fold is should read [`MediaQuery::panes_of`] instead, and one that only
    /// needs "is it open" should compare the [`status`](HingeState::status).
    ///
    /// # Examples
    ///
    /// ```ignore
    /// if let Some(hinge) = MediaQuery::hinge_of(ctx) {
    ///     pitch_bend(hinge.degrees());
    /// }
    /// ```
    pub fn hinge_of(ctx: &BuildContext) -> Option<HingeState> {
        watch_hinge(ctx)
    }

    /// The two areas a hinge or half-open fold leaves usable, or `None` when
    /// nothing divides the window.
    ///
    /// The widget is rebuilt when the features change, and — only while a
    /// feature does divide the window — when the window is resized. On a device
    /// without folds it never looks at the window at all, so a desktop window
    /// drag costs it nothing.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// match MediaQuery::panes_of(ctx) {
    ///     Some(panes) => two_pane(panes.first, panes.second),
    ///     None => single_pane(),
    /// }
    /// ```
    pub fn panes_of(ctx: &BuildContext) -> Option<Panes> {
        let features = watch_display_features(ctx);
        if !features.iter().any(|feature| feature.separating) {
            return None;
        }
        let size = Self::of(ctx).size;
        split_panes(size.width, size.height, &features)
    }

    #[inline]
    fn from_metrics(metrics: &aimer_widget::WindowMetrics) -> Self {
        Self {
            size: metrics.logical_size(),
            scale_factor: metrics.scale_factor as f32,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(width: f32, height: f32) -> MediaQuery {
        MediaQuery {
            size: ResolvedSize { width, height },
            scale_factor: 2.0,
        }
    }

    #[test]
    fn the_window_shape_follows_its_size() {
        assert_eq!(media(390.0, 844.0).orientation(), Orientation::Portrait);
        assert_eq!(media(844.0, 390.0).orientation(), Orientation::Landscape);
    }
}
