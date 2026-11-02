//! Everything a text field wears around its editing surface: a caller-built
//! frame and decorations that are not a [`BoxDecoration`].
//!
//! The field paints a [`BoxDecoration`] itself, in the same pass as its text,
//! because that decoration also decides how much room the text has. Anything
//! else — a [`Decoration`] of another kind, or a whole widget built from the
//! interaction state — is placed *around* the editing surface instead. The
//! surface is then mounted bare, and this module is what keeps it mounted
//! while the chrome around it is rebuilt.

use std::cell::Cell;
use std::rc::Rc;

use aimer_container::Container;
use aimer_decoration::Decoration;
use aimer_style::BoxDecoration;
use aimer_widget::base::BuildContext;
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, State, StateUpdater, StatefulElement, StatefulWidget,
    Widget,
};

/// Builds the chrome of a field from its interaction state.
pub(crate) type VisualBuilder = Rc<dyn Fn(TextFieldVisualState) -> AnyWidget>;

/// The interaction state handed to a field's [`builder`](crate::TextField::builder).
///
/// A snapshot: the builder runs again, and only the chrome it returns is
/// rebuilt, whenever one of these values changes. The editing surface itself —
/// text, selection, composition, focus — is not part of that rebuild, because
/// [`TextFieldVisualState::input`] hands back the same mounted element each
/// time.
#[derive(Clone)]
pub struct TextFieldVisualState {
    /// Whether the pointer is over the enabled field.
    pub is_hovered: bool,
    /// Whether the field owns the keyboard.
    pub is_focused: bool,
    /// Whether the field accepts input. A disabled field reports neither hover
    /// nor focus.
    pub is_enabled: bool,
    /// Whether the field shows its text but refuses edits.
    pub is_read_only: bool,
    /// Whether the field holds no text.
    pub is_empty: bool,
    input: ChildBuilder,
}

impl TextFieldVisualState {
    /// Returns the editing surface: the text, caret, selection and hint.
    ///
    /// The surface fills the space it is given and paints no background,
    /// border, or shadow of its own; the field's `padding` still insets its
    /// text. Place it exactly once in the returned widget, and keep it at the
    /// same position in the tree from one state to the next: the surface
    /// survives a rebuild of the chrome only while it is found where it was.
    #[inline]
    pub fn input(&self) -> ChildBuilder {
        self.input.clone()
    }
}

/// The values a [`VisualBuilder`] is shown, shared with the mounted field.
///
/// Owned by the field's state so it outlives the raw element, which is replaced
/// whenever the configuration is rebuilt.
pub(crate) struct VisualFlags {
    hovered: Cell<bool>,
    focused: Cell<bool>,
    empty: Cell<bool>,
    updater: Cell<Option<StateUpdater<VisualState>>>,
}

impl VisualFlags {
    #[inline]
    pub(crate) fn new(empty: bool) -> Self {
        Self {
            hovered: Cell::new(false),
            focused: Cell::new(false),
            empty: Cell::new(empty),
            updater: Cell::new(None),
        }
    }

    /// Records what the field now looks like and rebuilds the chrome if that
    /// differs from what it was last built from.
    pub(crate) fn sync(&self, hovered: bool, focused: bool, empty: bool) {
        // Every flag is stored, so the comparisons must not short-circuit.
        let changed = (self.hovered.replace(hovered) != hovered)
            | (self.focused.replace(focused) != focused)
            | (self.empty.replace(empty) != empty);
        // The flags belong to the field's state and outlive the chrome element
        // they feed: a page switch tears the chrome down while the field still
        // reports. A chrome that is gone has nothing to rebuild.
        if changed && let Some(updater) = self.updater.get() {
            let _ = updater.try_set_state(|_| {});
        }
    }
}

/// The chrome a field was configured with.
#[derive(Clone)]
pub(crate) struct Chrome {
    builder: VisualBuilder,
}

impl Chrome {
    /// A frame supplied by the caller. It owns the whole appearance, so the
    /// field's own decorations are not consulted.
    #[inline]
    pub(crate) fn from_builder(builder: VisualBuilder) -> Self {
        Self { builder }
    }

    /// A frame made of `decorations`, in `[normal, hover, focus, disabled]`
    /// order, around the bare editing surface.
    ///
    /// A state without a decoration of its own takes the next one down the
    /// field's precedence — disabled, focused, hovered, then normal — exactly as
    /// it would for a [`BoxDecoration`] painted by the field itself.
    pub(crate) fn from_decorations(decorations: [Option<Rc<dyn Decoration>>; 4]) -> Self {
        let normal = decorations[0]
            .clone()
            .unwrap_or_else(|| Rc::new(BoxDecoration::default()));
        let builder = move |state: TextFieldVisualState| {
            let by_state = [
                (!state.is_enabled, 3),
                (state.is_focused, 2),
                (state.is_hovered, 1),
            ];
            let chosen = by_state
                .into_iter()
                .filter(|(active, _)| *active)
                .find_map(|(_, slot)| decorations[slot].clone())
                .unwrap_or_else(|| normal.clone());
            Container::new()
                .decoration(chosen)
                .child(state.input())
                .boxed()
        };
        Self {
            builder: Rc::new(builder),
        }
    }

    /// Mounts the chrome around `input`.
    pub(crate) fn mount(
        &self,
        input: ChildBuilder,
        flags: Rc<VisualFlags>,
        enabled: bool,
        read_only: bool,
    ) -> AnyWidget {
        Visual {
            builder: self.builder.clone(),
            input,
            flags,
            enabled,
            read_only,
        }
        .boxed()
    }
}

/// Resolves a decoration handed to a builder method.
///
/// A [`BoxDecoration`] stays a plain value, because the field paints it with
/// its text and its public fields are part of the portable wire schema. Any
/// other decoration is erased once, here, and never while painting.
pub(crate) fn split<D: Decoration + 'static>(
    mut decoration: D,
) -> (Option<BoxDecoration>, Option<Rc<dyn Decoration>>) {
    if let Some(decoration) =
        (&mut decoration as &mut dyn std::any::Any).downcast_mut::<BoxDecoration>()
    {
        (Some(std::mem::take(decoration)), None)
    } else {
        (None, Some(Rc::new(decoration)))
    }
}

// The stateful boundary sits around the caller's widget only: a hover or a
// keystroke rebuilds the chrome, never the editing surface inside it.
struct Visual {
    builder: VisualBuilder,
    input: ChildBuilder,
    flags: Rc<VisualFlags>,
    enabled: bool,
    read_only: bool,
}

struct VisualState {
    visual: Visual,
    updater: StateUpdater<Self>,
}

impl aimer_widget::PortableWidget for Visual {}

impl Widget for Visual {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "TextFieldVisual", None)
            .0
            .boxed()
    }
}

impl StatefulWidget for Visual {
    type State = VisualState;

    fn create_state(self) -> Self::State {
        VisualState {
            visual: self,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<Visual> for VisualState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
        self.visual.flags.updater.set(Some(updater));
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.visual = new.visual;
        // A rebuild builds a candidate state first, and the candidate's
        // `init_state` takes the shared flags over before it is discarded in
        // favor of this mounted one. Take them back, or the chrome would never
        // be told about hover and focus again.
        self.visual.flags.updater.set(Some(self.updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        let visual = &self.visual;
        (visual.builder)(TextFieldVisualState {
            is_hovered: visual.enabled && visual.flags.hovered.get(),
            is_focused: visual.enabled && visual.flags.focused.get(),
            is_enabled: visual.enabled,
            is_read_only: visual.read_only,
            is_empty: visual.flags.empty.get(),
            input: visual.input.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input_field::raw_fields::test_support::dummy_build_context;

    /// The field keeps reporting hover and focus after its chrome is torn down.
    #[test]
    fn reporting_after_the_chrome_is_torn_down_does_not_panic() {
        let ctx = dummy_build_context(100.0, 40.0);
        let flags = Rc::new(VisualFlags::new(true));
        let chrome = Chrome::from_builder(Rc::new(|state| state.input().boxed()));
        let input = ChildBuilder::new(|| aimer_widget::ErrorWidget::new("input").boxed());
        let element = chrome
            .mount(input, flags.clone(), true, false)
            .to_element(&ctx);
        drop(element);

        flags.sync(true, true, false);
    }
}
