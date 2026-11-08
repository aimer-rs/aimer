//! The environment variables a launcher uses to start an app's server.
//!
//! A tool that starts the app — `aimer run` — cannot call [`Server::start`]
//! itself, and should not need the app to be rebuilt to be driven. It sets these
//! in the app's environment and the app, which calls
//! [`Server::start_from_env`], does the rest. They are plain constants so the
//! launcher and the app cannot disagree about a spelling.
//!
//! [`Server::start`]: crate::Server::start
//! [`Server::start_from_env`]: crate::Server::start_from_env

/// The switch. The server starts only when this is set to something other than
/// `0` or the empty string, so a stray setting below never opens a port.
pub const ENABLE: &str = "AIMER_PRENDRE";

/// The address to listen on. Default `127.0.0.1`.
///
/// Anything but a loopback address is reachable by every device on that
/// network, with the token as the only protection; see
/// [`Config::bind`](crate::Config::bind).
pub const BIND: &str = "AIMER_PRENDRE_BIND";

/// The port to listen on. Default `0`, which lets the OS choose.
pub const PORT: &str = "AIMER_PRENDRE_PORT";

/// The token a client must present. Default: a random one.
///
/// A launcher that makes its own token knows it without reading the app's log.
pub const TOKEN: &str = "AIMER_PRENDRE_TOKEN";
