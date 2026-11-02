use aimer_container::Container;
use aimer_style::{LayoutSpacing, Spacing, TextStyle};
use aimer_text::Text;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{AnyElement, PortableWidget, RequiredChild, Widget};

use crate::{Announcement, AnnouncementPriority};

/// The semantic tone of a reusable feedback presentation slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum StatusKind {
    /// Neutral status information.
    #[default]
    Info,
    /// Work is currently in progress.
    Loading,
    /// The operation completed successfully.
    Success,
    /// The operation needs attention but can continue.
    Warning,
    /// The operation failed or needs urgent attention.
    Error,
}

impl StatusKind {
    /// Returns the default background color for this status tone.
    #[inline]
    pub const fn background_color(self) -> Color {
        match self {
            Self::Info => Color::Rgba(80, 88, 104, 255),
            Self::Loading => Color::Rgba(35, 110, 220, 255),
            Self::Success => Color::Rgba(35, 142, 82, 255),
            Self::Warning => Color::Rgba(174, 112, 20, 255),
            Self::Error => Color::Rgba(178, 48, 56, 255),
        }
    }

    /// Returns a short, text-friendly symbol for the status tone.
    #[inline]
    pub const fn symbol(self) -> &'static str {
        match self {
            Self::Info => "i",
            Self::Loading => "…",
            Self::Success => "✓",
            Self::Warning => "!",
            Self::Error => "×",
        }
    }
}

/// A reusable colored presentation slot that can contain any widget.
///
/// The slot carries semantic tone plus optional presentation overrides;
/// applications remain free to supply a custom child, layout, or translated
/// message. No overlay host or global state is created by this widget.
pub struct FeedbackSlot<W = RequiredChild> {
    kind: StatusKind,
    child: W,
    background_color: Option<Color>,
    padding: LayoutSpacing,
}

impl Default for FeedbackSlot {
    fn default() -> Self {
        Self::new(StatusKind::Info)
    }
}

impl FeedbackSlot {
    /// Creates an incomplete slot builder for `kind`.
    #[inline]
    pub fn new(kind: StatusKind) -> Self {
        Self {
            kind,
            child: RequiredChild,
            background_color: None,
            padding: LayoutSpacing::default(),
        }
    }
}

impl<W> FeedbackSlot<W> {
    /// Replaces the slot's child and completes the widget builder.
    #[inline]
    pub fn child<C: Widget>(self, child: C) -> FeedbackSlot<C> {
        FeedbackSlot {
            kind: self.kind,
            child,
            background_color: self.background_color,
            padding: self.padding,
        }
    }

    /// Overrides the default tone color used behind the child.
    #[inline]
    pub const fn background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Sets spacing between the slot edge and its child.
    #[inline]
    pub const fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Returns the slot's semantic tone.
    #[inline]
    pub const fn kind(&self) -> StatusKind {
        self.kind
    }
}

impl<W: Widget + 'static> Widget for FeedbackSlot<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        Container::new()
            .padding(self.padding)
            .color(
                self.background_color
                    .unwrap_or_else(|| self.kind.background_color()),
            )
            .child(self.child)
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "FeedbackSlot"
    }
}

impl<W: Widget + 'static> PortableWidget for FeedbackSlot<W> {}

/// A compact status banner backed by [`FeedbackSlot`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBanner {
    kind: StatusKind,
    message: String,
    background_color: Option<Color>,
    foreground_color: Color,
    padding: LayoutSpacing,
}

impl StatusBanner {
    /// Creates a neutral banner containing `message`.
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: StatusKind::Info,
            message: message.into(),
            background_color: None,
            foreground_color: Color::WHITE,
            padding: LayoutSpacing::all(Spacing::Px(12)),
        }
    }

    /// Creates a banner with an explicit semantic tone.
    #[inline]
    pub fn with_kind(kind: StatusKind, message: impl Into<String>) -> Self {
        Self::new(message).kind(kind)
    }

    /// Creates a loading banner.
    #[inline]
    pub fn loading(message: impl Into<String>) -> Self {
        Self::with_kind(StatusKind::Loading, message)
    }

    /// Creates a success banner.
    #[inline]
    pub fn success(message: impl Into<String>) -> Self {
        Self::with_kind(StatusKind::Success, message)
    }

    /// Creates a warning banner.
    #[inline]
    pub fn warning(message: impl Into<String>) -> Self {
        Self::with_kind(StatusKind::Warning, message)
    }

    /// Creates an error banner.
    #[inline]
    pub fn error(message: impl Into<String>) -> Self {
        Self::with_kind(StatusKind::Error, message)
    }

    /// Sets the banner's semantic tone.
    #[inline]
    pub const fn kind(mut self, kind: StatusKind) -> Self {
        self.kind = kind;
        self
    }

    /// Overrides the default tone color used behind the banner.
    #[inline]
    pub const fn background_color(mut self, color: Color) -> Self {
        self.background_color = Some(color);
        self
    }

    /// Sets the text and symbol color drawn inside the banner.
    #[inline]
    pub const fn foreground_color(mut self, color: Color) -> Self {
        self.foreground_color = color;
        self
    }

    /// Sets spacing between the banner edge and its message.
    #[inline]
    pub const fn padding(mut self, padding: LayoutSpacing) -> Self {
        self.padding = padding;
        self
    }

    /// Returns the banner tone.
    #[inline]
    pub const fn kind_value(&self) -> StatusKind {
        self.kind
    }

    /// Returns the banner message.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the optional custom background color.
    #[inline]
    pub const fn background_color_value(&self) -> Option<Color> {
        self.background_color
    }

    /// Returns the banner's text and symbol color.
    #[inline]
    pub const fn foreground_color_value(&self) -> Color {
        self.foreground_color
    }

    /// Returns the spacing between the banner edge and its message.
    #[inline]
    pub const fn padding_value(&self) -> LayoutSpacing {
        self.padding
    }

    /// Creates the accessibility announcement for this banner.
    #[inline]
    pub fn announcement(&self) -> Announcement {
        let priority = if self.kind == StatusKind::Error {
            AnnouncementPriority::Assertive
        } else {
            AnnouncementPriority::Polite
        };
        Announcement::new(self.message.clone()).with_priority(priority)
    }
}

impl Widget for StatusBanner {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let text = format!("{}  {}", self.kind.symbol(), self.message);
        FeedbackSlot::new(self.kind)
            .background_color(
                self.background_color
                    .unwrap_or_else(|| self.kind.background_color()),
            )
            .padding(self.padding)
            .child(Text::new(text).text_style(TextStyle::new().color(self.foreground_color)))
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "StatusBanner"
    }
}

impl PortableWidget for StatusBanner {}
