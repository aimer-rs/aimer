//! Lets `aimer run` drive an app that was not changed for the occasion.
//!
//! A launcher sets `AIMER_PRENDRE` (see [`aimer_prendre::env`]) in the app's
//! environment, and the app then accepts synthetic input from the launcher.
//! Injected input is full control of the app, so this exists in debug builds
//! only: a release build never starts a server, whatever its environment says.

#[cfg(debug_assertions)]
static SERVER: std::sync::OnceLock<aimer_prendre::Server> = std::sync::OnceLock::new();

/// Starts the input server if the launcher asked for one. Does nothing in a
/// release build, and nothing when the environment does not ask.
///
/// Called once the event loop's proxy is installed, which is what a command that
/// arrives needs to reach the loop. A failure is logged and the app runs on: an
/// app that cannot be driven is still an app.
pub(crate) fn start() {
    #[cfg(debug_assertions)]
    match aimer_prendre::Server::start_from_env() {
        // Held for the life of the process, so it keeps listening.
        Ok(Some(server)) => {
            let _ = SERVER.set(server);
        }
        Ok(None) => {}
        Err(error) => aimer_utils::error!("aimer_prendre: could not start: {error}"),
    }
}
