//! Keyboard shortcuts bound to a region of the widget tree.

use aimer_attribute::size::{ResolvedSize, Size};
use aimer_events::element::ElementEvent;
use aimer_shortcut::{KeyChord, ShortcutScope};
use aimer_widget_core::base::BuildContext;
use aimer_widget_core::components::event_element::{EventElement, EventResult, EventTreeRole};
use aimer_widget_core::components::layout_element::LayoutElement;
use aimer_widget_core::components::rebuildable::Rebuildable;
use aimer_widget_core::components::visitor_element::VisitorElement;
use aimer_widget_core::{AnyElement, AnyWidget, Drawable, Element, RequiredChild, Widget};

/// Binds keyboard shortcuts to the subtree it wraps.
///
/// `Shortcuts` changes nothing about layout or painting. While it is in the
/// tree, a key chord such as `Ctrl+S` or `F5` that the focused element did not
/// consume runs the action bound to it here, provided keyboard focus is
/// somewhere inside the subtree. The chord is then spent: it never reaches the
/// widget under the pointer as well.
///
/// Scopes nest, and the innermost scope around the focused element that binds
/// the chord wins, so a text editor can override the page's `Ctrl+S` while
/// leaving its other shortcuts alone. A [`window`](Shortcuts::window) scope is
/// consulted wherever focus is, after the focus-scoped ones, which is what an
/// application-wide `Cmd+K` wants.
///
/// The focused element always gets the key first: a text field keeps its own
/// `Ctrl+C`, and a shortcut only sees what the field declined.
///
/// A held key does not run an action over and over unless the scope
/// [repeats](Shortcuts::repeat), because a shortcut that opens a dialog must not
/// open forty of them.
///
/// Shortcuts that should also work when the application is in the background
/// are *global*; see `aimer_shortcut::global`.
///
/// # Examples
///
/// ```
/// use aimer_focusable::Shortcuts;
///
/// struct Editor;
/// # impl aimer_widget_core::Widget for Editor {
/// #     fn to_element(self, _ctx: &aimer_widget_core::base::BuildContext) -> aimer_widget_core::AnyElement {
/// #         unreachable!("this example only builds the widget")
/// #     }
/// # }
///
/// // `CmdOrCtrl` is Command on macOS and Control everywhere else.
/// let page = Shortcuts::new()
///     .bind_str("CmdOrCtrl+S", || println!("save"))
///     .window(true)
///     .bind_str("CmdOrCtrl+K", || println!("command palette"))
///     .child(Editor);
/// ```
pub struct Shortcuts<W = RequiredChild> {
    child: W,
    scope: ShortcutScope,
}

impl Shortcuts {
    /// Creates an empty, focus-scoped builder.
    ///
    /// Finish the builder with [`Shortcuts::child`] or
    /// [`Shortcuts::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self { child: RequiredChild, scope: ShortcutScope::new() }
    }

    /// Binds `chord` to `action`. Binding a chord twice keeps the last action.
    #[inline]
    pub fn bind(mut self, chord: KeyChord, action: impl Fn() + 'static) -> Self {
        self.scope = self.scope.bind(chord, action);
        self
    }

    /// Binds the chord written as text, such as `"CmdOrCtrl+Shift+P"`, to
    /// `action`.
    ///
    /// # Panics
    ///
    /// Panics if `chord` does not parse. A chord is a literal in the source, so
    /// a typo is a bug to find at once; use [`Shortcuts::bind`] with
    /// [`KeyChord::parse`] for text that comes from a user or a file.
    #[inline]
    pub fn bind_str(self, chord: &str, action: impl Fn() + 'static) -> Self {
        let chord = KeyChord::parse(chord).expect("`bind_str` needs a valid key chord");
        self.bind(chord, action)
    }

    /// Makes the shortcuts active wherever focus is, not only while it is
    /// inside the subtree. Off by default.
    #[inline]
    pub fn window(mut self, window: bool) -> Self {
        self.scope = self.scope.window(window);
        self
    }

    /// Lets a held key run an action repeatedly. Off by default.
    #[inline]
    pub fn repeat(mut self, repeat: bool) -> Self {
        self.scope = self.scope.repeat(repeat);
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<W: Widget>(self, child: W) -> Shortcuts<W> {
        Shortcuts { child, scope: self.scope }
    }

    /// Attaches `child` and erases the resulting widget's concrete type.
    #[inline]
    pub fn box_child<W: Widget + 'static>(self, child: W) -> AnyWidget {
        self.child(child).boxed()
    }
}

impl Default for Shortcuts {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W: Widget + 'static> Widget for Shortcuts<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        RawShortcuts { scope: self.scope, child: self.child.to_element(ctx) }.boxed()
    }

    fn debug_name(&self) -> &'static str {
        "Shortcuts"
    }
}

struct RawShortcuts {
    child: AnyElement,
    scope: ShortcutScope,
}

impl Rebuildable for RawShortcuts {}

impl EventElement for RawShortcuts {
    fn event_tree_role(&self) -> EventTreeRole {
        // Scopes are found by walking the structural tree, never by hit testing.
        EventTreeRole::Transparent
    }

    #[inline]
    fn shortcut_scope(&self) -> Option<&ShortcutScope> {
        Some(&self.scope)
    }

    fn on_event(&self, _event: &ElementEvent) -> EventResult {
        EventResult::ignored()
    }

    fn event_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}

impl Drawable for RawShortcuts {
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
    }

    fn can_paint(&self, _ctx: &BuildContext) -> bool {
        true
    }

    fn paint(&self, _ctx: &BuildContext) {}

    fn sync_paint_geometry(&self, ctx: &BuildContext) {
        self.child.forward_paint_geometry(ctx);
    }
}

impl LayoutElement for RawShortcuts {
    fn size(&self) -> Option<Size> {
        self.child.size()
    }

    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }

    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }

    fn layer(&self) -> u32 {
        self.child.layer()
    }

    fn flex(&self) -> Option<f32> {
        self.child.flex()
    }

    fn get_size_from_child(&self) -> Option<Size> {
        self.child.get_size_from_child()
    }
}

impl VisitorElement for RawShortcuts {
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }

    fn debug_name(&self) -> &'static str {
        "Shortcuts"
    }
}

#[cfg(test)]
mod tests {
    use aimer_shortcut::Activation;

    use super::*;

    struct TestChild;

    impl VisitorElement for TestChild {
        fn debug_name(&self) -> &'static str {
            "TestChild"
        }
    }

    impl EventElement for TestChild {}
    impl LayoutElement for TestChild {}
    impl Rebuildable for TestChild {}

    impl Drawable for TestChild {
        fn update(&self, _ctx: &BuildContext) {}
    }

    #[test]
    fn the_element_exposes_its_scope_and_is_otherwise_transparent() {
        let hit = std::rc::Rc::new(std::cell::Cell::new(false));
        let seen = hit.clone();
        let chord = KeyChord::parse_with_primary("Ctrl+S", aimer_shortcut::ChordMods::CTRL).unwrap();
        let element = RawShortcuts {
            child: TestChild.boxed(),
            scope: ShortcutScope::new().bind(chord, move || seen.set(true)),
        };

        assert_eq!(element.event_tree_role(), EventTreeRole::Transparent);
        assert!(!element.on_event(&ElementEvent::FocusGained).is_consumed());
        let scope = element.shortcut_scope().expect("a Shortcuts element has a scope");
        assert_eq!(scope.activate(&chord, false), Activation::Ran);
        assert!(hit.get());
    }

    #[test]
    #[should_panic(expected = "valid key chord")]
    fn bind_str_rejects_a_chord_that_cannot_parse() {
        let _ = Shortcuts::new().bind_str("Ctrl+", || {});
    }
}
