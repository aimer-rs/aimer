use aimer_widget::base::{BuildContext, ResolvedSize};

/// The window as the widget being built sees it.
///
/// Reading this from a `build` registers the widget as depending on the window,
/// so it — and only it — is rebuilt when the window is resized or moves to a
/// display with a different scale factor. Everything else keeps the subtree it
/// already has and is merely laid out again, which is what keeps a window drag
/// smooth on a large tree.
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

    #[inline]
    fn from_metrics(metrics: &aimer_widget::WindowMetrics) -> Self {
        Self {
            size: metrics.logical_size(),
            scale_factor: metrics.scale_factor as f32,
        }
    }
}
