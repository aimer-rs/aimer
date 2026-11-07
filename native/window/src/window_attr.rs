use winit::dpi::LogicalSize;
use winit::window::WindowAttributes;

/// Describes a native window: the one an `AimerApp` starts with, or one opened
/// later with [`NewWindow`](crate::NewWindow) or the [`Window`](crate::Window)
/// widget.
///
/// `WindowAttr` is deliberately independent of `winit`, so applications can
/// configure the window without coupling their public API to the windowing
/// backend. Build one with [`Self::new`], apply the desired properties, and
/// pass it to `AimerApp::window`, or to the window that is being opened.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowAttr {
    pub(crate) title: String,
    pub(crate) inner_size: (u32, u32),
    pub(crate) min_inner_size: Option<(u32, u32)>,
    pub(crate) max_inner_size: Option<(u32, u32)>,
    pub(crate) resizable: bool,
    pub(crate) decorations: bool,
    pub(crate) transparent: bool,
    pub(crate) visible: bool,
    pub(crate) maximized: bool,
}

impl WindowAttr {
    /// Creates window attributes with Aimer's standard desktop appearance.
    #[inline]
    pub fn new() -> Self {
        Self {
            title: "Aimer".to_owned(),
            inner_size: (1150, 800),
            min_inner_size: None,
            max_inner_size: None,
            resizable: true,
            decorations: true,
            transparent: false,
            visible: true,
            maximized: false,
        }
    }

    /// Sets the text shown in the native window's title bar.
    #[inline]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the initial logical width and height of the window.
    #[inline]
    pub fn inner_size(mut self, width: u32, height: u32) -> Self {
        self.inner_size = (width, height);
        self
    }

    /// Sets the smallest logical width and height the window may have.
    #[inline]
    pub fn min_inner_size(mut self, width: u32, height: u32) -> Self {
        self.min_inner_size = Some((width, height));
        self
    }

    /// Sets the largest logical width and height the window may have.
    #[inline]
    pub fn max_inner_size(mut self, width: u32, height: u32) -> Self {
        self.max_inner_size = Some((width, height));
        self
    }

    /// Controls whether the user can resize the window.
    #[inline]
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Controls whether the platform window decorations are shown.
    #[inline]
    pub fn decorations(mut self, decorations: bool) -> Self {
        self.decorations = decorations;
        self
    }

    /// Controls whether the native window has a transparent background.
    #[inline]
    pub fn transparent(mut self, transparent: bool) -> Self {
        self.transparent = transparent;
        self
    }

    /// Controls whether the window is visible when it is created.
    #[inline]
    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Controls whether the window starts maximized.
    #[inline]
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// Whether the window is visible when it is created.
    #[inline]
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// The attributes in the form the platform loop creates a window from.
    #[doc(hidden)]
    pub fn to_winit(&self) -> WindowAttributes {
        let mut attributes = WindowAttributes::default()
            .with_inner_size(LogicalSize::new(self.inner_size.0, self.inner_size.1))
            .with_title(self.title.clone())
            .with_resizable(self.resizable)
            .with_decorations(self.decorations)
            .with_transparent(self.transparent)
            .with_visible(self.visible)
            .with_maximized(self.maximized);

        if let Some((width, height)) = self.min_inner_size {
            attributes = attributes.with_min_inner_size(LogicalSize::new(width, height));
        }
        if let Some((width, height)) = self.max_inner_size {
            attributes = attributes.with_max_inner_size(LogicalSize::new(width, height));
        }

        attributes
    }
}

impl Default for WindowAttr {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::WindowAttr;

    #[test]
    fn window_attributes_have_aimer_defaults() {
        let attributes = WindowAttr::new();

        assert_eq!(attributes.title, "Aimer");
        assert_eq!(attributes.inner_size, (1150, 800));
        assert_eq!(attributes.min_inner_size, None);
        assert_eq!(attributes.max_inner_size, None);
        assert!(attributes.resizable);
        assert!(attributes.decorations);
        assert!(!attributes.transparent);
        assert!(attributes.visible);
        assert!(!attributes.maximized);
    }

    #[test]
    fn window_attributes_can_be_configured_with_aimer_methods() {
        let attributes = WindowAttr::new()
            .title("Configured Window")
            .inner_size(900, 600)
            .min_inner_size(480, 320)
            .max_inner_size(1920, 1080)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .visible(false)
            .maximized(true);

        assert_eq!(attributes.title, "Configured Window");
        assert_eq!(attributes.inner_size, (900, 600));
        assert_eq!(attributes.min_inner_size, Some((480, 320)));
        assert_eq!(attributes.max_inner_size, Some((1920, 1080)));
        assert!(!attributes.resizable);
        assert!(!attributes.decorations);
        assert!(attributes.transparent);
        assert!(!attributes.visible);
        assert!(attributes.maximized);
    }
}
