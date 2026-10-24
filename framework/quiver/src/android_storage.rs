//! Handing Android's app-data directory to the storage layer.
//!
//! Android has no environment variable for where an app may write; the path
//! comes from the activity. `aimer_storage` therefore cannot find it on its own,
//! and an app that opened its storage before being told would get
//! `Unavailable` where every other platform just works.

use winit::platform::android::activity::AndroidApp;

/// Tells `aimer_storage` where this app keeps its files.
///
/// Safe to call more than once: the first directory wins, and the activity never
/// gives a different one.
pub(crate) fn publish_data_dir(app: &AndroidApp) {
    if let Some(directory) = app.internal_data_path() {
        let _ = aimer_storage::set_android_data_dir(directory);
    }
}

/// Registers the activity's `AndroidApp` for the framework and for storage.
///
/// The generated Android entry point (`#[aimer::main]`) calls this before it
/// runs your code, so `AppStorage::new` and `FileStorage::for_app` work from the
/// first line of your `main`. An entry point written by hand should call it
/// instead of setting [`ANDROID_APP`](crate::aimer_app::ANDROID_APP) directly.
pub fn register_android_app(app: AndroidApp) {
    publish_data_dir(&app);
    let _ = crate::aimer_app::ANDROID_APP.set(app);
}
