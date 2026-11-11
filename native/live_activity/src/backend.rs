//! The contract each platform fulfils, and the choice of platform.

use aimer_tray::TrayIcon;

use crate::{LiveActivity, LiveActivityError, LiveActivityId, Presentation};

/// One connection to a platform's live activity surface.
pub(crate) trait Backend {
    /// Starts the activity, or updates it when its id is already running. The
    /// activity has passed validation.
    fn show(&self, activity: &LiveActivity) -> Result<(), LiveActivityError>;
    /// Ends the activity; one that is not running is not an error.
    fn end(&self, id: LiveActivityId) -> Result<(), LiveActivityError>;
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod apple;
#[cfg(any(target_os = "macos", target_os = "ios"))]
use apple as native;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as native;

// The tray stands in wherever there is no native surface, and everywhere when
// the native one cannot be reached.
mod tray;

#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
mod unsupported;
#[cfg(not(any(target_os = "macos", target_os = "ios", target_os = "android")))]
use unsupported as native;

/// Connects to the native surface, or to the tray when the native one is
/// missing and an icon was offered.
pub(crate) fn create(
    tray_icon: Option<TrayIcon>,
) -> Result<(Box<dyn Backend>, Presentation), LiveActivityError> {
    match native::create() {
        Ok(backend) => Ok((backend, Presentation::Native)),
        Err(error) => match tray_icon {
            Some(icon) => Ok((Box::new(tray::TrayBackend::new(icon)), Presentation::Tray)),
            None => Err(error),
        },
    }
}
