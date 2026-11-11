//! Views for the Dynamic Island and the lock screen, described as data.
//!
//! The SwiftUI of a live activity runs in the widget extension, a separate
//! process with no Aimer runtime, so a Rust widget cannot be drawn there.
//! [`UIKitWidget`] is therefore deliberately *not* an Aimer `Widget`: it has no
//! `to_element`, so the compiler refuses to put one in an Aimer tree. It only
//! describes a small tree of views ([`ViewNode`]), which travels to the
//! extension inside the activity's JSON and is drawn there by a generic SwiftUI
//! interpreter (`AimerNodeView` in `AimerLiveActivityWidget.swift`).

use std::fmt::Write;

use crate::payload::push_string;

/// A widget that can only be shown inside a live activity.
///
/// Implement it to compose the built-in widgets ([`Label`], [`Symbol`],
/// [`Bar`], [`Clock`], [`Row`], [`Column`], [`Gap`]) under one name.
///
/// # Examples
///
/// ```
/// use aimer_live_activity::{Clock, Label, Row, Tint, UIKitWidget, ViewNode};
///
/// struct Ticket(&'static str);
///
/// impl UIKitWidget for Ticket {
///     fn to_node(&self) -> ViewNode {
///         Row::new()
///             .child(Label::new(self.0).tint(Tint::Accent))
///             .child(Clock::new())
///             .to_node()
///     }
/// }
/// ```
pub trait UIKitWidget {
    /// Describes this widget as a view tree.
    fn to_node(&self) -> ViewNode;
}

/// A node of the view tree a [`UIKitWidget`] describes.
///
/// It is opaque on purpose: the shape is the contract with the Swift side, and
/// only the built-in widgets build it.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewNode(Node);

impl UIKitWidget for ViewNode {
    #[inline]
    fn to_node(&self) -> ViewNode {
        self.clone()
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Node {
    Text { text: String, style: TextStyle, tint: Tint, digits: bool, lines: Option<u8> },
    Symbol { name: String, tint: Tint, points: Option<f32> },
    Bar { fraction: Option<f32>, tint: Tint },
    Clock { style: TextStyle, tint: Tint },
    Stack { vertical: bool, spacing: Option<f32>, children: Vec<ViewNode> },
    Gap,
}

/// A slot of the Dynamic Island or the lock screen a view can fill.
///
/// A slot with no view keeps the template's own content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The banner on the lock screen and in the notification center.
    LockScreen,
    /// The leading edge of the expanded island.
    ExpandedLeading,
    /// The trailing edge of the expanded island.
    ExpandedTrailing,
    /// The middle of the expanded island.
    ExpandedCenter,
    /// Under the expanded island.
    ExpandedBottom,
    /// The left of the compact island.
    CompactLeading,
    /// The right of the compact island.
    CompactTrailing,
    /// The dot shown when two activities share the island.
    Minimal,
}

impl Region {
    /// The key the Swift side looks the view up by.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::LockScreen => "lockScreen",
            Self::ExpandedLeading => "expandedLeading",
            Self::ExpandedTrailing => "expandedTrailing",
            Self::ExpandedCenter => "expandedCenter",
            Self::ExpandedBottom => "expandedBottom",
            Self::CompactLeading => "compactLeading",
            Self::CompactTrailing => "compactTrailing",
            Self::Minimal => "minimal",
        }
    }
}

/// The size and weight of a [`Label`] or [`Clock`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextStyle {
    /// Large, for the one thing to read at a glance.
    Title,
    /// Bold and medium sized.
    Headline,
    /// The default.
    #[default]
    Body,
    /// Small.
    Caption,
}

impl TextStyle {
    fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Headline => "headline",
            Self::Body => "body",
            Self::Caption => "caption",
        }
    }
}

/// A colour, by role or by name; each resolves to the system colour of that
/// name, so it follows light and dark appearance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tint {
    /// The default text colour.
    #[default]
    Primary,
    /// The dimmed text colour.
    Secondary,
    /// The application's accent colour.
    Accent,
    /// Red.
    Red,
    /// Orange.
    Orange,
    /// Yellow.
    Yellow,
    /// Green.
    Green,
    /// Blue.
    Blue,
    /// Gray.
    Gray,
}

impl Tint {
    fn key(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
            Self::Accent => "accent",
            Self::Red => "red",
            Self::Orange => "orange",
            Self::Yellow => "yellow",
            Self::Green => "green",
            Self::Blue => "blue",
            Self::Gray => "gray",
        }
    }
}

/// A line of text.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    text: String,
    style: TextStyle,
    tint: Tint,
    digits: bool,
    lines: Option<u8>,
}

impl Label {
    /// A label that says `text`.
    #[inline]
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), style: TextStyle::Body, tint: Tint::Primary, digits: false, lines: None }
    }

    /// The size and weight.
    #[inline]
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The colour.
    #[inline]
    pub fn tint(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }

    /// Gives every digit the same width, so changing numbers do not jitter.
    #[inline]
    pub fn digits(mut self, digits: bool) -> Self {
        self.digits = digits;
        self
    }

    /// The most lines the text may take; `0` means no limit.
    #[inline]
    pub fn lines(mut self, lines: u8) -> Self {
        self.lines = if lines == 0 { None } else { Some(lines) };
        self
    }
}

impl UIKitWidget for Label {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Text {
            text: self.text.clone(),
            style: self.style,
            tint: self.tint,
            digits: self.digits,
            lines: self.lines,
        })
    }
}

/// An SF Symbol.
#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    name: String,
    tint: Tint,
    points: Option<f32>,
}

impl Symbol {
    /// The SF Symbol called `name`, such as `"timer"`. An empty or unknown name
    /// draws a bell.
    #[inline]
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), tint: Tint::Primary, points: None }
    }

    /// The colour.
    #[inline]
    pub fn tint(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }

    /// The size in points. Not finite or not positive keeps the default.
    #[inline]
    pub fn points(mut self, points: f32) -> Self {
        self.points = (points.is_finite() && points > 0.0).then_some(points);
        self
    }
}

impl UIKitWidget for Symbol {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Symbol { name: self.name.clone(), tint: self.tint, points: self.points })
    }
}

/// A progress bar.
///
/// By default it follows the activity: the system-ticked bar of its
/// [countdown](crate::LiveActivity::countdown) when it has one, else its
/// progress. [`Bar::value`] pins it to a fixed fraction instead.
#[derive(Clone, Debug, PartialEq)]
pub struct Bar {
    fraction: Option<f32>,
    tint: Tint,
}

impl Bar {
    /// A bar that follows the activity.
    #[inline]
    pub fn new() -> Self {
        Self { fraction: None, tint: Tint::Accent }
    }

    /// Pins the bar at `fraction` of `0.0..=1.0`; out of range is clamped and
    /// `NaN` counts as `0.0`.
    #[inline]
    pub fn value(mut self, fraction: f32) -> Self {
        self.fraction = Some(if fraction.is_nan() { 0.0 } else { fraction.clamp(0.0, 1.0) });
        self
    }

    /// The colour of the filled part.
    #[inline]
    pub fn tint(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }
}

impl Default for Bar {
    fn default() -> Self {
        Self::new()
    }
}

impl UIKitWidget for Bar {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Bar { fraction: self.fraction, tint: self.tint })
    }
}

/// The clock of the activity's [countdown](crate::LiveActivity::countdown),
/// ticked by the system so it keeps moving while the application is suspended.
/// Without a countdown it shows the activity's status text.
#[derive(Clone, Debug, PartialEq)]
pub struct Clock {
    style: TextStyle,
    tint: Tint,
}

impl Clock {
    /// A clock in the default style.
    #[inline]
    pub fn new() -> Self {
        Self { style: TextStyle::Body, tint: Tint::Primary }
    }

    /// The size and weight.
    #[inline]
    pub fn style(mut self, style: TextStyle) -> Self {
        self.style = style;
        self
    }

    /// The colour.
    #[inline]
    pub fn tint(mut self, tint: Tint) -> Self {
        self.tint = tint;
        self
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::new()
    }
}

impl UIKitWidget for Clock {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Clock { style: self.style, tint: self.tint })
    }
}

/// Views side by side.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Row {
    spacing: Option<f32>,
    children: Vec<ViewNode>,
}

impl Row {
    /// An empty row.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The gap between children, in points.
    #[inline]
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = sane_spacing(spacing);
        self
    }

    /// Adds a child after the existing ones.
    #[inline]
    pub fn child(mut self, child: impl UIKitWidget) -> Self {
        self.children.push(child.to_node());
        self
    }
}

impl UIKitWidget for Row {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Stack { vertical: false, spacing: self.spacing, children: self.children.clone() })
    }
}

/// Views one above the other.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Column {
    spacing: Option<f32>,
    children: Vec<ViewNode>,
}

impl Column {
    /// An empty column.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The gap between children, in points.
    #[inline]
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = sane_spacing(spacing);
        self
    }

    /// Adds a child below the existing ones.
    #[inline]
    pub fn child(mut self, child: impl UIKitWidget) -> Self {
        self.children.push(child.to_node());
        self
    }
}

impl UIKitWidget for Column {
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Stack { vertical: true, spacing: self.spacing, children: self.children.clone() })
    }
}

/// Flexible empty space that pushes its neighbours apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Gap;

impl Gap {
    /// A flexible gap.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl UIKitWidget for Gap {
    #[inline]
    fn to_node(&self) -> ViewNode {
        ViewNode(Node::Gap)
    }
}

#[inline]
fn sane_spacing(spacing: f32) -> Option<f32> {
    (spacing.is_finite() && spacing >= 0.0).then_some(spacing)
}

impl ViewNode {
    /// Appends this node as a JSON object; see the `AimerNode` type of
    /// `AimerLiveActivityAttributes.swift` for the reading side.
    pub(crate) fn write_json(&self, out: &mut String) {
        match &self.0 {
            Node::Text { text, style, tint, digits, lines } => {
                out.push_str("{\"type\":\"text\",\"text\":");
                push_string(out, text);
                let _ = write!(out, ",\"style\":\"{}\",\"tint\":\"{}\"", style.key(), tint.key());
                if *digits {
                    out.push_str(",\"digits\":true");
                }
                if let Some(lines) = lines {
                    let _ = write!(out, ",\"lines\":{lines}");
                }
            }
            Node::Symbol { name, tint, points } => {
                out.push_str("{\"type\":\"symbol\",\"name\":");
                push_string(out, name);
                let _ = write!(out, ",\"tint\":\"{}\"", tint.key());
                if let Some(points) = points {
                    let _ = write!(out, ",\"points\":{points}");
                }
            }
            Node::Bar { fraction, tint } => {
                let _ = write!(out, "{{\"type\":\"bar\",\"tint\":\"{}\"", tint.key());
                if let Some(fraction) = fraction {
                    let _ = write!(out, ",\"fraction\":{fraction}");
                }
            }
            Node::Clock { style, tint } => {
                let _ = write!(out, "{{\"type\":\"clock\",\"style\":\"{}\",\"tint\":\"{}\"", style.key(), tint.key());
            }
            Node::Stack { vertical, spacing, children } => {
                let _ = write!(out, "{{\"type\":\"{}\"", if *vertical { "column" } else { "row" });
                if let Some(spacing) = spacing {
                    let _ = write!(out, ",\"spacing\":{spacing}");
                }
                out.push_str(",\"children\":[");
                for (index, child) in children.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    child.write_json(out);
                }
                out.push(']');
            }
            Node::Gap => out.push_str("{\"type\":\"gap\""),
        }
        out.push('}');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn json(widget: &impl UIKitWidget) -> String {
        let mut out = String::new();
        widget.to_node().write_json(&mut out);
        out
    }

    #[test]
    fn label_writes_its_style_and_options() {
        assert_eq!(
            json(&Label::new("Hi \"you\"")),
            "{\"type\":\"text\",\"text\":\"Hi \\\"you\\\"\",\"style\":\"body\",\"tint\":\"primary\"}"
        );
        assert_eq!(
            json(&Label::new("1").style(TextStyle::Title).tint(Tint::Red).digits(true).lines(2)),
            "{\"type\":\"text\",\"text\":\"1\",\"style\":\"title\",\"tint\":\"red\",\"digits\":true,\"lines\":2}"
        );
        assert!(!json(&Label::new("x").lines(0)).contains("lines"));
    }

    #[test]
    fn symbol_ignores_a_bad_size() {
        assert_eq!(
            json(&Symbol::new("timer").tint(Tint::Orange).points(18.0)),
            "{\"type\":\"symbol\",\"name\":\"timer\",\"tint\":\"orange\",\"points\":18}"
        );
        assert!(!json(&Symbol::new("a").points(f32::NAN)).contains("points"));
        assert!(!json(&Symbol::new("a").points(-1.0)).contains("points"));
    }

    #[test]
    fn bar_follows_the_activity_unless_pinned() {
        assert_eq!(json(&Bar::new()), "{\"type\":\"bar\",\"tint\":\"accent\"}");
        assert_eq!(
            json(&Bar::new().value(7.0).tint(Tint::Green)),
            "{\"type\":\"bar\",\"tint\":\"green\",\"fraction\":1}"
        );
        assert!(json(&Bar::new().value(f32::NAN)).contains("\"fraction\":0}"));
    }

    #[test]
    fn clock_writes_style_and_tint() {
        assert_eq!(
            json(&Clock::new().style(TextStyle::Caption)),
            "{\"type\":\"clock\",\"style\":\"caption\",\"tint\":\"primary\"}"
        );
    }

    #[test]
    fn stacks_nest_their_children_in_order() {
        let tree = Column::new()
            .spacing(4.0)
            .child(Row::new().child(Symbol::new("a").tint(Tint::Gray)).child(Gap::new()).child(Clock::new()))
            .child(Bar::new());
        assert_eq!(
            json(&tree),
            "{\"type\":\"column\",\"spacing\":4,\"children\":[\
             {\"type\":\"row\",\"children\":[\
             {\"type\":\"symbol\",\"name\":\"a\",\"tint\":\"gray\"},{\"type\":\"gap\"},\
             {\"type\":\"clock\",\"style\":\"body\",\"tint\":\"primary\"}]},\
             {\"type\":\"bar\",\"tint\":\"accent\"}]}"
        );
        assert_eq!(json(&Row::new().spacing(f32::INFINITY)), "{\"type\":\"row\",\"children\":[]}");
    }

    #[test]
    fn a_custom_widget_composes_the_built_in_ones() {
        struct Pair;
        impl UIKitWidget for Pair {
            fn to_node(&self) -> ViewNode {
                Row::new().child(Label::new("a")).child(Label::new("b")).to_node()
            }
        }
        assert_eq!(json(&Pair).matches("\"type\":\"text\"").count(), 2);
    }

    #[test]
    fn regions_have_distinct_keys() {
        let all = [
            Region::LockScreen,
            Region::ExpandedLeading,
            Region::ExpandedTrailing,
            Region::ExpandedCenter,
            Region::ExpandedBottom,
            Region::CompactLeading,
            Region::CompactTrailing,
            Region::Minimal,
        ];
        let mut keys: Vec<_> = all.iter().map(|region| region.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), all.len());
    }
}
