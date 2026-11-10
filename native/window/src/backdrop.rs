use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::sync::{Arc, Weak};

use aimer_platform::window_backdrop::set_window_material;
use aimer_widget::base::{BuildContext, WindowHandle};
use aimer_widget::{
    AnyElement, AnyWidget, ChildBuilder, RequiredChild, State, StateUpdater, StatefulElement,
    StatefulWidget, Widget,
};

use crate::WindowMaterial;

/// Blurs what is behind the window this widget is in.
///
/// Wrap the window's content in a backdrop and the window itself turns into the
/// frosted glass of a Finder sidebar or a Spotlight panel: the system blurs the
/// desktop and the windows behind it, and shows that through everything the
/// content draws with some transparency. Content that is drawn opaquely stays
/// opaque, so a backdrop usually goes under a *translucent* background, such as
/// a color with an alpha, or none at all.
///
/// The blur is drawn by the system compositor, not by Aimer, so it costs a
/// frame nothing and keeps up with whatever moves behind the window. It applies
/// to the whole window; to blur only what is behind a part of the window's
/// content, use a backdrop filter.
///
/// The widget takes no room and paints nothing of its own: it is the child,
/// untouched. It asks for the effect when it enters the tree, changes the
/// [material](WindowBackdrop::material) when that changes, and takes the effect
/// away, restoring the window, when it leaves.
///
/// The effect needs a platform that has it. macOS draws every
/// [`WindowMaterial`] with an `NSVisualEffectView`, except the glass materials,
/// which use the `NSGlassEffectView` of macOS 26 and later and fall back to the
/// sidebar blur before that. Windows 11 draws Mica and Acrylic through DWM.
/// Everywhere else, and in a window that is not a
/// native one, the widget is inert and the child is shown as it is.
///
/// # Examples
///
/// ```
/// use aimer_window::{WindowBackdrop, WindowMaterial};
///
/// struct Sidebar;
/// # impl aimer_widget::Widget for Sidebar {
/// #     fn to_element(self, _ctx: &aimer_widget::base::BuildContext) -> aimer_widget::AnyElement {
/// #         unreachable!("this example only builds the widget")
/// #     }
/// # }
///
/// // The whole window frosted like a sidebar.
/// let frosted = WindowBackdrop::new().child(Sidebar);
///
/// // A darker, stronger blur, switched off by a flag.
/// let hud = WindowBackdrop::new()
///     .material(WindowMaterial::Hud)
///     .enabled(false)
///     .child(Sidebar);
/// ```
pub struct WindowBackdrop<W = RequiredChild> {
    material: WindowMaterial,
    enabled: bool,
    /// The content, kept as a builder because the widget is rebuilt whenever its
    /// material changes and needs the same child each time.
    child: ChildBuilder,
    /// Records which child type completed the builder without storing it.
    marker: PhantomData<W>,
}

impl WindowBackdrop {
    /// Creates a backdrop of the sidebar material.
    ///
    /// Finish the builder with [`WindowBackdrop::child`] or
    /// [`WindowBackdrop::box_child`].
    #[inline]
    pub fn new() -> Self {
        Self {
            material: WindowMaterial::default(),
            enabled: true,
            child: ChildBuilder::required(),
            marker: PhantomData,
        }
    }
}

impl Default for WindowBackdrop {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<W> WindowBackdrop<W> {
    /// Sets how the system blurs what is behind the window.
    ///
    /// The default is [`WindowMaterial::Sidebar`].
    #[inline]
    pub fn material(mut self, material: WindowMaterial) -> Self {
        self.material = material;
        self
    }

    /// Sets whether the window has the backdrop. The default is `true`.
    ///
    /// A widget that is sometimes frosted keeps one build path and flips this,
    /// instead of adding and removing the wrapper.
    #[inline]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Attaches the required child and completes this builder.
    #[inline]
    pub fn child<C: Widget + 'static>(self, child: C) -> WindowBackdrop<C> {
        WindowBackdrop {
            material: self.material,
            enabled: self.enabled,
            child: ChildBuilder::from_widget(child),
            marker: PhantomData,
        }
    }

    /// Attaches `child` and erases the completed widget's concrete type.
    ///
    /// Exactly equivalent to `self.child(child).boxed()`.
    #[inline]
    pub fn box_child<C: Widget + 'static>(self, child: C) -> AnyWidget {
        self.child(child).boxed()
    }
}

/// The retained side of [`WindowBackdrop`]: what it asked of its window, so the
/// window can be given back.
pub struct WindowBackdropState<W: Widget + 'static> {
    material: WindowMaterial,
    enabled: bool,
    child: ChildBuilder,
    /// The window the backdrop was applied to, held weakly so the widget does
    /// not keep a closed window alive.
    window: RefCell<Option<Weak<winit::window::Window>>>,
    /// The material the window has, if it has one because of this state.
    ///
    /// The framework builds the state of a widget that was emitted again before
    /// it reconciles it with the one already in the tree, and then drops the new
    /// one. Only a state that applied a backdrop takes it away, so that one
    /// dropped unused leaves the live one's alone.
    applied: Cell<Option<WindowMaterial>>,
    marker: PhantomData<W>,
}

impl<W: Widget + 'static> WindowBackdropState<W> {
    /// Makes the window match what the widget asks for.
    fn sync(&self, window: &WindowHandle) {
        let WindowHandle::Native(window) = window else {
            return;
        };
        let wanted = self.enabled.then_some(self.material);
        if self.applied.get() == wanted {
            return;
        }
        if set_window_material(window, wanted) {
            self.applied.set(wanted);
            *self.window.borrow_mut() = Some(Arc::downgrade(window));
        }
    }
}

impl<W: Widget + 'static> StatefulWidget for WindowBackdrop<W> {
    type State = WindowBackdropState<W>;

    fn create_state(self) -> Self::State {
        WindowBackdropState {
            material: self.material,
            enabled: self.enabled,
            child: self.child,
            window: RefCell::new(None),
            applied: Cell::new(None),
            marker: PhantomData,
        }
    }
}

impl<W: Widget + 'static> Widget for WindowBackdrop<W> {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "WindowBackdrop", None)
            .0
            .boxed()
    }

    fn debug_name(&self) -> &'static str {
        "WindowBackdrop"
    }
}

impl<W: Widget + 'static> State<WindowBackdrop<W>> for WindowBackdropState<W> {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
    }

    fn adopt_config_from(&mut self, new: Self) {
        self.material = new.material;
        self.enabled = new.enabled;
        // A clone, because the state cannot be taken apart: it restores the
        // window when it drops. Cloning a retained child is a reference count.
        self.child = new.child.clone();
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        self.sync(&ctx.window);
        self.child.clone()
    }
}

impl<W: Widget + 'static> Drop for WindowBackdropState<W> {
    /// The widget left the tree: its window goes back to what it was.
    fn drop(&mut self) {
        if self.applied.get().is_none() {
            return;
        }
        let window = self.window.borrow_mut().take().and_then(|window| window.upgrade());
        if let Some(window) = window {
            set_window_material(&window, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use aimer_container::SizedBox;

    use super::*;

    #[test]
    fn a_backdrop_is_enabled_and_sidebar_by_default() {
        let state = WindowBackdrop::new().child(SizedBox::new()).create_state();

        assert_eq!(state.material, WindowMaterial::Sidebar);
        assert!(state.enabled);
        assert_eq!(state.applied.get(), None, "nothing is applied before a build");
    }

    #[test]
    fn the_builder_carries_material_and_flag_into_the_state() {
        let state = WindowBackdrop::new()
            .material(WindowMaterial::Hud)
            .enabled(false)
            .child(SizedBox::new())
            .create_state();

        assert_eq!(state.material, WindowMaterial::Hud);
        assert!(!state.enabled);
    }

    #[test]
    fn a_headless_window_is_left_alone() {
        let state = WindowBackdrop::new().child(SizedBox::new()).create_state();
        let window = WindowHandle::headless(winit::dpi::PhysicalSize::new(800, 600), 1.0);

        state.sync(&window);

        assert_eq!(state.applied.get(), None);
    }
}
