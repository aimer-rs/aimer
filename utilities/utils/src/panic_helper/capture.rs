use std::borrow::Cow;
use std::cell::Cell;
use std::fmt;
use std::panic::Location;
use std::sync::OnceLock;

use super::source_snippet;

aimer_tls::thread_local! {
    /// How many [`PanicWatch`] guards are alive on this thread. Nested builds
    /// nest their watches, so only the outermost guard disarms the capture.
    static WATCH_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Where the last watched panic of this thread came from.
    static WATCHED_SITE: Cell<Option<PanicSite>> = const { Cell::new(None) };
}

/// The source coordinates a panic came from, rendered the way a compiler points
/// at the offending expression.
///
/// A recovered panic only carries its payload — the message — while the place it
/// happened is known to the panic hook alone. [`PanicSite::watch`] bridges the
/// two, so a framework that catches a panic can still report it as precisely as
/// the runtime does.
///
/// # Examples
///
/// ```
/// use aimer_utils::PanicSite;
///
/// let site = PanicSite::new("app/src/main.rs", 17, 9);
///
/// assert_eq!(site.file(), "app/src/main.rs");
/// assert_eq!(site.line(), 17);
/// assert_eq!(site.column(), 9);
/// ```
///
/// Displaying a site prints the coordinates and, when the source is available,
/// the offending line under them:
///
/// ```text
/// at jaime/src/http_request_button.rs:117:67
///
///         let panic: Option<i32> = Option::None.unwrap();
///                                  ^^^^^^^^^^^^^^^^^^^^^
/// ```
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PanicSite {
    file: Cow<'static, str>,
    line: u32,
    column: u32,
}

impl PanicSite {
    /// Builds a site from explicit coordinates.
    #[inline]
    pub fn new(file: impl Into<Cow<'static, str>>, line: u32, column: u32) -> Self {
        Self {
            file: file.into(),
            line,
            column,
        }
    }

    /// Builds a site from a tracked [`Location`], as returned by
    /// [`Location::caller`] or [`std::panic::PanicHookInfo::location`].
    #[inline]
    pub fn of(location: &Location<'_>) -> Self {
        Self::new(
            location.file().to_owned(),
            location.line(),
            location.column(),
        )
    }

    /// The source file, relative to the workspace root.
    #[inline]
    pub fn file(&self) -> &str {
        &self.file
    }

    /// The one-based line of the panicking expression.
    #[inline]
    pub fn line(&self) -> u32 {
        self.line
    }

    /// The one-based column of the panicking expression.
    #[inline]
    pub fn column(&self) -> u32 {
        self.column
    }

    /// Starts recording the site of panics raised on this thread, and keeps the
    /// default panic handler quiet while they are being recorded.
    ///
    /// The returned guard stops the recording when it is dropped; take the site
    /// out of it with [`PanicWatch::take_site`]. Watches nest, so a build that
    /// recovers a panic inside another recovered build still reports both.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::panic::catch_unwind;
    ///
    /// use aimer_utils::PanicSite;
    ///
    /// let watch = PanicSite::watch();
    /// let failed = catch_unwind(|| panic!("boom")).is_err();
    /// let site = watch.take_site();
    ///
    /// assert!(failed);
    /// assert!(site.is_some());
    /// ```
    #[inline]
    pub fn watch() -> PanicWatch {
        install_hook();
        WATCH_DEPTH.with(|depth| depth.set(depth.get().saturating_add(1)));
        PanicWatch { _private: () }
    }
}

impl fmt::Display for PanicSite {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "at {}:{}:{}", self.file, self.line, self.column)?;
        if let Some(snippet) = source_snippet(&self.file, self.line, self.column) {
            write!(f, "\n\n{snippet}")?;
        }
        Ok(())
    }
}

/// Guard returned by [`PanicSite::watch`], recording where the panics of this
/// thread come from for as long as it is alive.
#[derive(Debug)]
pub struct PanicWatch {
    _private: (),
}

impl PanicWatch {
    /// The site of the last panic raised while this thread was watched, clearing
    /// it so a later watch cannot report a stale location.
    ///
    /// [`None`] when no panic was raised, or when the runtime could not tell
    /// where it came from.
    #[inline]
    pub fn take_site(self) -> Option<PanicSite> {
        WATCHED_SITE.with(Cell::take)
    }
}

impl Drop for PanicWatch {
    #[inline]
    fn drop(&mut self) {
        WATCH_DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

/// Whether panics of this thread are currently being recorded.
#[inline]
fn watching() -> bool {
    WATCH_DEPTH
        .try_with(|depth| depth.get() > 0)
        .unwrap_or(false)
}

/// Installs the panic hook that records watched panics, once per process.
///
/// Unwatched panics are handed to the hook that was installed before, so a
/// crash outside a recovered build still prints exactly what it used to.
fn install_hook() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !watching() {
                previous(info);
                return;
            }
            if let Some(location) = info.location() {
                let site = PanicSite::of(location);
                let _ = WATCHED_SITE.try_with(|slot| slot.set(Some(site)));
            }
        }));
    });
}

#[cfg(test)]
mod tests {
    use std::panic::catch_unwind;

    use super::*;

    #[test]
    fn watch_records_where_the_panic_came_from() {
        let watch = PanicSite::watch();
        let expected_line = line!() + 1;
        let failed = catch_unwind(|| panic!("boom")).is_err();
        let site = watch
            .take_site()
            .expect("the panic site should be recorded");

        assert!(failed);
        assert_eq!(site.file(), file!());
        assert_eq!(site.line(), expected_line);
    }

    #[test]
    fn displayed_site_highlights_the_panicking_expression() {
        let watch = PanicSite::watch();
        let _ = catch_unwind(|| Option::<i32>::None.unwrap());
        let rendered = watch
            .take_site()
            .expect("the panic site should be recorded")
            .to_string();

        assert!(rendered.starts_with("at "), "{rendered}");
        assert!(
            rendered.contains("Option::<i32>::None.unwrap()"),
            "{rendered}"
        );
        assert!(rendered.contains("^^^"), "{rendered}");
    }

    #[test]
    fn taking_the_site_clears_it_for_the_next_watch() {
        let watch = PanicSite::watch();
        let _ = catch_unwind(|| panic!("boom"));
        assert!(watch.take_site().is_some());

        let watch = PanicSite::watch();
        assert!(watch.take_site().is_none());
    }

    #[test]
    fn nested_watches_disarm_with_the_outermost_guard() {
        assert!(!watching());

        let outer = PanicSite::watch();
        let inner = PanicSite::watch();
        assert!(watching());

        drop(inner);
        assert!(watching());

        drop(outer);
        assert!(!watching());
    }

    #[test]
    fn displayed_site_falls_back_to_coordinates_when_source_is_unavailable() {
        let site = PanicSite::new("missing/aimer/source.rs", 17, 9);

        assert_eq!(site.to_string(), "at missing/aimer/source.rs:17:9");
    }
}
