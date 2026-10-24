use aimer_widget::Brightness;

/// Whether a theme follows the system appearance or overrides it.
///
/// [`ThemeMode::System`] is what an application wants by default: the user
/// already told the operating system how they want to read a screen, and every
/// platform lets them change that answer while the application runs. The two
/// explicit modes exist for the application that offers its own light/dark
/// switch — choosing one of them ignores the system for as long as it is set.
///
/// # Examples
///
/// ```
/// use aimer_style::ThemeMode;
/// use aimer_widget::Brightness;
///
/// // Following the system means answering with whatever it reports.
/// assert_eq!(ThemeMode::System.resolve(Brightness::Dark), Brightness::Dark);
///
/// // An explicit mode ignores it.
/// assert_eq!(ThemeMode::Light.resolve(Brightness::Dark), Brightness::Light);
/// ```
#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ThemeMode {
    /// Use the appearance the platform reports, and follow it when it changes.
    #[default]
    System,
    /// Always use the light theme, whatever the platform reports.
    Light,
    /// Always use the dark theme, whatever the platform reports.
    Dark,
}

impl ThemeMode {
    /// Returns the appearance to draw in, given what the platform reports.
    ///
    /// `system` is only consulted by [`ThemeMode::System`].
    #[inline]
    pub const fn resolve(self, system: Brightness) -> Brightness {
        match self {
            Self::System => system,
            Self::Light => Brightness::Light,
            Self::Dark => Brightness::Dark,
        }
    }

    /// Whether the mode has to be told about system appearance changes.
    ///
    /// Only a mode that follows the system does; an explicit mode is answered
    /// without reading the platform at all, so a widget using one never
    /// registers as a follower and is never rebuilt by a switch it ignores.
    #[inline]
    pub const fn follows_system(self) -> bool {
        matches!(self, Self::System)
    }
}

/// A theme, or a light/dark pair of themes and the mode that chooses between
/// them.
///
/// A theme that adapts is two themes plus one decision, and that decision is
/// worth keeping separate from the widget that animates the result: it is pure,
/// so it is the part that can be stated exactly. A selection without a dark
/// counterpart resolves to its single theme in every mode — an application that
/// supplies one theme means to use it.
///
/// # Examples
///
/// ```
/// use aimer_style::{ThemeData, ThemeMode, ThemeSelection};
/// use aimer_widget::Brightness;
///
/// let adaptive = ThemeSelection::adaptive(ThemeData::light(), ThemeData::dark());
///
/// assert_eq!(adaptive.resolve(Brightness::Dark), ThemeData::dark());
/// assert_eq!(adaptive.resolve(Brightness::Light), ThemeData::light());
///
/// // Overriding the system pins the answer.
/// let pinned = adaptive.mode(ThemeMode::Light);
/// assert_eq!(pinned.resolve(Brightness::Dark), ThemeData::light());
/// ```
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ThemeSelection<T> {
    /// The theme used for [`Brightness::Light`], and the only theme when there
    /// is no dark counterpart.
    pub light: T,
    /// The theme used for [`Brightness::Dark`], when one was supplied.
    pub dark: Option<T>,
    /// How the two are chosen between.
    pub mode: ThemeMode,
}

impl<T> ThemeSelection<T> {
    /// Creates a selection of one theme, used in every appearance.
    #[inline]
    pub const fn fixed(theme: T) -> Self {
        Self {
            light: theme,
            dark: None,
            mode: ThemeMode::System,
        }
    }

    /// Creates a selection that follows the system between `light` and `dark`.
    #[inline]
    pub const fn adaptive(light: T, dark: T) -> Self {
        Self {
            light,
            dark: Some(dark),
            mode: ThemeMode::System,
        }
    }

    /// Replaces the mode, overriding or restoring the system appearance.
    #[inline]
    pub fn mode(mut self, mode: ThemeMode) -> Self {
        self.mode = mode;
        self
    }

    /// Replaces the dark counterpart.
    #[inline]
    pub fn dark(mut self, dark: T) -> Self {
        self.dark = Some(dark);
        self
    }

    /// Whether resolving this selection depends on what the platform reports.
    ///
    /// A selection with no dark counterpart, or one whose mode overrides the
    /// system, answers the same way whatever the platform does — and so must
    /// not be registered as a follower of it.
    #[inline]
    pub const fn follows_system(&self) -> bool {
        self.mode.follows_system() && self.dark.is_some()
    }
}

impl<T: Clone> ThemeSelection<T> {
    /// Returns the theme to use, given the appearance the platform reports.
    #[inline]
    pub fn resolve(&self, system: Brightness) -> T {
        match (self.mode.resolve(system), &self.dark) {
            (Brightness::Dark, Some(dark)) => dark.clone(),
            _ => self.light.clone(),
        }
    }
}

impl<T: Default> Default for ThemeSelection<T> {
    fn default() -> Self {
        Self::fixed(T::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_the_system_answers_with_what_it_reports() {
        let selection = ThemeSelection::adaptive("light", "dark");

        assert_eq!(selection.resolve(Brightness::Light), "light");
        assert_eq!(selection.resolve(Brightness::Dark), "dark");
    }

    #[test]
    fn an_explicit_mode_ignores_the_system() {
        let selection = ThemeSelection::adaptive("light", "dark");

        assert_eq!(
            selection.mode(ThemeMode::Light).resolve(Brightness::Dark),
            "light"
        );
        assert_eq!(
            selection.mode(ThemeMode::Dark).resolve(Brightness::Light),
            "dark"
        );
    }

    #[test]
    fn a_single_theme_is_used_in_every_appearance() {
        let selection = ThemeSelection::fixed("only");

        assert_eq!(selection.resolve(Brightness::Dark), "only");
        assert_eq!(
            selection.mode(ThemeMode::Dark).resolve(Brightness::Light),
            "only"
        );
    }

    #[test]
    fn only_an_adaptive_selection_follows_the_system() {
        assert!(ThemeSelection::adaptive("light", "dark").follows_system());
        assert!(!ThemeSelection::fixed("only").follows_system());
        assert!(
            !ThemeSelection::adaptive("light", "dark")
                .mode(ThemeMode::Dark)
                .follows_system()
        );
    }

    #[test]
    fn a_dark_counterpart_can_be_added_to_a_single_theme() {
        let selection = ThemeSelection::fixed("light").dark("dark");

        assert!(selection.follows_system());
        assert_eq!(selection.resolve(Brightness::Dark), "dark");
    }
}
