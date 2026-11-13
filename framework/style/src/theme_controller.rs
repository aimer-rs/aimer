use std::cell::{Cell, RefCell};
use std::ops::Deref;
use std::rc::Rc;

use aimer_provider::{ProviderContext, Snapshot};
use aimer_widget::Brightness;
use aimer_widget::base::BuildContext;

use crate::ThemeMode;

struct ControllerState {
    mode: Cell<ThemeMode>,
    brightness: Cell<Brightness>,
    /// Asks the owning [`crate::AnimatedTheme`] to resolve again. Empty until
    /// the theme's state exists, which is before any descendant can run.
    request: RefCell<Option<Box<dyn Fn()>>>,
}

/// Changes the appearance of the [`crate::AnimatedTheme`] that owns it.
///
/// An `AnimatedTheme` keeps the choice between its light and dark themes as
/// state of its own, and this is the handle to that state. A widget anywhere
/// below the theme reaches it without being given anything: the button that
/// switches the appearance and the settings page that offers the choice need no
/// stateful widget above the theme to carry the decision.
///
/// A change is a theme change like any other. The theme retargets from what is
/// currently on screen and animates there with the duration and curve it was
/// configured with, so switching in the middle of a transition does not snap.
///
/// The handle is cheap to clone and every clone controls the same theme. The
/// mode it starts in is the one given to [`crate::AnimatedTheme::mode`]; a
/// parent that later configures a *different* mode replaces whatever the
/// controller set, since that is an explicit new decision from above.
///
/// # Examples
///
/// ```
/// use aimer_style::{ThemeController, ThemeMode};
/// use aimer_widget::base::BuildContext;
///
/// fn dark_mode_switch(ctx: &BuildContext) -> impl Fn() {
///     let controller = ThemeController::of(ctx);
///     move || controller.set_mode(ThemeMode::Dark)
/// }
/// ```
#[derive(Clone)]
pub struct ThemeController(Rc<ControllerState>);

impl ThemeController {
    #[inline]
    pub(crate) fn new(mode: ThemeMode) -> Self {
        Self(Rc::new(ControllerState {
            mode: Cell::new(mode),
            brightness: Cell::new(Brightness::default()),
            request: RefCell::new(None),
        }))
    }

    /// Returns the controller of the nearest [`crate::AnimatedTheme`] ancestor.
    ///
    /// Looking it up does not subscribe the widget to anything. A widget that
    /// shows the current appearance reads the theme with [`crate::Theme::of`],
    /// which does subscribe and hands the controller back with the theme.
    ///
    /// # Panics
    ///
    /// Panics when there is no `AnimatedTheme` ancestor or when called outside
    /// a widget build.
    #[track_caller]
    pub fn of(context: &BuildContext) -> Self {
        let Some(controller) = context.try_read::<Self>() else {
            panic!("No `AnimatedTheme` found in the current widget scope")
        };
        Self::clone(&controller)
    }

    /// Returns the mode the theme is currently in.

    #[inline]
    pub fn mode(&self) -> ThemeMode {
        self.0.mode.get()
    }

    /// Returns the appearance the theme last resolved to.
    ///
    /// This is the *target*: it is [`ThemeMode::Light`] or [`ThemeMode::Dark`]
    /// as soon as the switch is decided, while the colors are still crossing
    /// over. It reports what [`ThemeController::toggle`] would leave, so it is
    /// what a "switch to light/dark" label should say.
    #[inline]
    pub fn brightness(&self) -> Brightness {
        self.0.brightness.get()
    }

    /// Sets the mode and animates the theme to it.
    ///
    /// [`ThemeMode::System`] hands the decision back to the platform. Setting
    /// the mode the theme is already in does nothing.
    pub fn set_mode(&self, mode: ThemeMode) {
        if self.0.mode.replace(mode) == mode {
            return;
        }
        if let Some(request) = self.0.request.borrow().as_ref() {
            request();
        }
    }

    /// Switches to the opposite of the appearance currently resolved.
    ///
    /// The result is an explicit [`ThemeMode::Light`] or [`ThemeMode::Dark`],
    /// so a toggle on a theme that followed the system stops following it; use
    /// [`ThemeController::set_mode`] with [`ThemeMode::System`] to follow it
    /// again.
    pub fn toggle(&self) {
        self.set_mode(match self.brightness() {
            Brightness::Dark => ThemeMode::Light,
            Brightness::Light => ThemeMode::Dark,
        });
    }

    #[inline]
    pub(crate) fn set_brightness(&self, brightness: Brightness) {
        self.0.brightness.set(brightness);
    }

    /// Replaces the mode without asking for a rebuild, for the theme adopting a
    /// mode its parent configured while it is already rebuilding.
    #[inline]
    pub(crate) fn adopt_mode(&self, mode: ThemeMode) {
        self.0.mode.set(mode);
    }

    #[inline]
    pub(crate) fn bind(&self, request: impl Fn() + 'static) {
        *self.0.request.borrow_mut() = Some(Box::new(request));
    }
}

impl PartialEq for ThemeController {
    /// Two controllers are equal when they control the same theme.
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for ThemeController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThemeController")
            .field("mode", &self.mode())
            .field("brightness", &self.brightness())
            .finish()
    }
}

/// A theme as read by [`crate::Theme::of`]: the value, and the controller of the
/// theme that supplied it.
///
/// It dereferences to the theme, so fields and methods are used directly; the
/// controller is one call away for the widget that wants to change what it just
/// read.
///
/// # Examples
///
/// ```
/// use aimer_style::{Theme, ThemeData, ThemeMode};
/// use aimer_widget::base::BuildContext;
///
/// fn on_press(ctx: &BuildContext) -> impl Fn() {
///     let theme = ThemeData::of(ctx);
///     let _background = theme.background_color;
///     let controller = theme.controller().clone();
///     move || controller.set_mode(ThemeMode::Dark)
/// }
/// ```
pub struct ThemeSnapshot<T> {
    theme: Snapshot<T>,
    controller: ThemeController,
}

impl<T> ThemeSnapshot<T> {
    #[inline]
    pub(crate) fn new(theme: Snapshot<T>, controller: ThemeController) -> Self {
        Self { theme, controller }
    }

    /// Returns the controller of the theme this value came from.
    #[inline]
    pub fn controller(&self) -> &ThemeController {
        &self.controller
    }
}

impl<T> Clone for ThemeSnapshot<T> {
    fn clone(&self) -> Self {
        Self {
            theme: self.theme.clone(),
            controller: self.controller.clone(),
        }
    }
}

impl<T> Deref for ThemeSnapshot<T> {
    type Target = T;

    #[inline]
    fn deref(&self) -> &T {
        &self.theme
    }
}
