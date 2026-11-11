//! The handle an application shows live activities through.

use std::fmt;
use std::marker::PhantomData;

use aimer_tray::TrayIcon;

use crate::backend::{self, Backend};
use crate::{LiveActivity, LiveActivityId};

/// Why a live activity operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveActivityError {
    /// The activity has neither a title nor a body.
    Empty,
    /// This platform has no live activities this library can reach, and no tray
    /// icon was given to stand in for them.
    Unsupported,
    /// The user turned live activities off for the application.
    Disabled,
    /// The platform refused, with its own explanation.
    Platform(String),
}

impl fmt::Display for LiveActivityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("a live activity needs a title or a body"),
            Self::Unsupported => f.write_str("this platform has no live activities"),
            Self::Disabled => f.write_str("live activities are turned off for this application"),
            Self::Platform(reason) => write!(f, "the live activity service failed: {reason}"),
        }
    }
}

impl std::error::Error for LiveActivityError {}

/// Where a [`LiveActivities`] puts its activities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Presentation {
    /// The platform's own surface: a Live Update on Android, a Live Activity
    /// on iOS.
    Native,
    /// A tray icon whose tooltip carries the text of the activities. Only
    /// chosen when the platform has no native surface and
    /// [`LiveActivitiesBuilder::tray_icon`] was given.
    Tray,
}

/// Describes a [`LiveActivities`]; [`LiveActivitiesBuilder::build`] connects it
/// to the platform.
#[derive(Default)]
pub struct LiveActivitiesBuilder {
    tray_icon: Option<TrayIcon>,
}

impl LiveActivitiesBuilder {
    /// Opts into the tray as a stand-in where the platform has no live
    /// activities of its own, which is macOS, Windows and Linux, and an iOS
    /// application that ships without the Swift bridge.
    ///
    /// The icon appears when the first activity is shown and goes away with the
    /// last one. Its tooltip carries one line per running activity. Without
    /// this call such platforms report [`LiveActivityError::Unsupported`].
    #[inline]
    pub fn tray_icon(mut self, icon: TrayIcon) -> Self {
        self.tray_icon = Some(icon);
        self
    }

    /// Connects to the platform's live activity service.
    ///
    /// On macOS and iOS call this on the main thread, which is where the event
    /// loop of an Aimer application runs.
    ///
    /// # Errors
    ///
    /// [`LiveActivityError::Unsupported`] where there is neither a native
    /// surface nor a tray icon, and [`LiveActivityError::Platform`] when the
    /// system refuses.
    pub fn build(self) -> Result<LiveActivities, LiveActivityError> {
        let (backend, presentation) = backend::create(self.tray_icon)?;
        Ok(LiveActivities { backend, presentation, _not_send: PhantomData })
    }
}

/// A connection to the platform's live activity service. Dropping it does not
/// end the activities already running on a native surface, which belong to the
/// system; a tray stand-in disappears with it.
///
/// It stays on the thread that built it.
///
/// # Examples
///
/// ```no_run
/// use aimer_live_activity::{LiveActivities, LiveActivity, LiveActivityId};
///
/// let activities = LiveActivities::new().build().unwrap();
/// let id = LiveActivityId(1);
///
/// activities
///     .show(&LiveActivity::new(id).title("Downloading").progress(0.1))
///     .unwrap();
/// // The same id updates it.
/// activities
///     .show(&LiveActivity::new(id).title("Downloading").progress(0.8))
///     .unwrap();
/// activities.end(id).unwrap();
/// ```
pub struct LiveActivities {
    backend: Box<dyn Backend>,
    presentation: Presentation,
    _not_send: PhantomData<*const ()>,
}

impl LiveActivities {
    /// Starts describing a connection.
    #[inline]
    #[allow(clippy::new_ret_no_self)]
    pub fn new() -> LiveActivitiesBuilder {
        LiveActivitiesBuilder::default()
    }

    /// Where this connection puts its activities.
    #[inline]
    pub fn presentation(&self) -> Presentation {
        self.presentation
    }

    /// Starts `activity`, or updates it when one with the same id is running.
    ///
    /// # Errors
    ///
    /// [`LiveActivityError::Empty`] without a title or body,
    /// [`LiveActivityError::Disabled`] when the user turned live activities
    /// off, and [`LiveActivityError::Platform`] when the system refuses.
    pub fn show(&self, activity: &LiveActivity) -> Result<(), LiveActivityError> {
        activity.validate()?;
        self.backend.show(activity)
    }

    /// Ends the activity named `id`. Ending one that is not running is not an
    /// error.
    ///
    /// # Errors
    ///
    /// [`LiveActivityError::Platform`] when the system refuses.
    pub fn end(&self, id: LiveActivityId) -> Result<(), LiveActivityError> {
        self.backend.end(id)
    }
}

impl fmt::Debug for LiveActivities {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveActivities").field("presentation", &self.presentation).finish_non_exhaustive()
    }
}
