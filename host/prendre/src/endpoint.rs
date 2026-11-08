use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::str::FromStr;

/// What a server's log line says before the endpoint, which is then quoted.
const ANNOUNCEMENT: &str = "aimer_prendre: listening, endpoint is `";

/// Where a running [`Server`](crate::Server) listens, and the secret that opens
/// it.
///
/// It prints as `ADDRESS TOKEN` on one line, which is also what [`save`] writes
/// and [`FromStr`] reads, so the process that owns the app can hand it to the
/// process that drives it through a file or a log line.
///
/// [`save`]: Endpoint::save
///
/// # Examples
///
/// ```
/// use aimer_prendre::Endpoint;
///
/// let endpoint: Endpoint = "127.0.0.1:4001 0123abcd".parse().expect("valid endpoint");
/// assert_eq!(endpoint.addr().port(), 4001);
/// assert_eq!(endpoint.token(), "0123abcd");
/// assert_eq!(endpoint.to_string(), "127.0.0.1:4001 0123abcd");
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Endpoint {
    addr: SocketAddr,
    token: String,
}

impl Endpoint {
    /// Pairs an address with its token.
    ///
    /// The token must be non-empty and free of whitespace, or the endpoint could
    /// not be written on one line and read back.
    pub fn new(addr: SocketAddr, token: impl Into<String>) -> Result<Self, InvalidEndpoint> {
        let token = token.into();
        if token.is_empty() || token.contains(char::is_whitespace) {
            return Err(InvalidEndpoint);
        }
        Ok(Self { addr, token })
    }

    /// The address the server listens on.
    ///
    /// For a server bound to an unspecified address (`0.0.0.0`, `::`) this is
    /// where it listens, not where a client connects: use
    /// [`Endpoint::with_ip`] to say how this machine reaches it.
    #[inline]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The same endpoint reached at `ip`, keeping the port and the token.
    ///
    /// The server only knows the address it bound; whoever starts the app knows
    /// the one that leads to it, such as a phone's tunnel address.
    #[inline]
    pub fn with_ip(&self, ip: IpAddr) -> Self {
        Self { addr: SocketAddr::new(ip, self.addr.port()), token: self.token.clone() }
    }

    /// The log line a server writes when it starts listening.
    ///
    /// A launcher that owns the app's console finds the endpoint in it with
    /// [`Endpoint::from_announcement`].
    pub fn announcement(&self) -> String {
        format!("{ANNOUNCEMENT}{self}`")
    }

    /// Finds an announcement anywhere in `line` and reads the endpoint from it.
    ///
    /// The app's logger decorates its lines with a timestamp and a level, so the
    /// announcement is searched for rather than required at the start. Returns
    /// `None` for any line that is not one, or whose endpoint cannot be read.
    pub fn from_announcement(line: &str) -> Option<Self> {
        let start = line.find(ANNOUNCEMENT)? + ANNOUNCEMENT.len();
        let inner = &line[start..];
        inner[..inner.find('`')?].parse().ok()
    }

    /// The secret a client must present before the server accepts input.
    #[inline]
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Writes the endpoint to `path`, replacing the file.
    ///
    /// The token is the only thing keeping other local processes from driving
    /// the app, so put the file somewhere only the current user can read.
    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        std::fs::write(path, self.to_string())
    }

    /// Reads an endpoint written by [`Endpoint::save`].
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        std::fs::read_to_string(path)?
            .trim()
            .parse()
            .map_err(|e: InvalidEndpoint| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.addr, self.token)
    }
}

impl FromStr for Endpoint {
    type Err = InvalidEndpoint;

    fn from_str(text: &str) -> Result<Self, InvalidEndpoint> {
        let mut words = text.split_ascii_whitespace();
        let (Some(addr), Some(token), None) = (words.next(), words.next(), words.next()) else {
            return Err(InvalidEndpoint);
        };
        Self::new(addr.parse().map_err(|_| InvalidEndpoint)?, token)
    }
}

/// The text was not `ADDRESS TOKEN`, or the token cannot be written on a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidEndpoint;

impl fmt::Display for InvalidEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an endpoint is `ADDRESS TOKEN` on one line")
    }
}

impl std::error::Error for InvalidEndpoint {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_round_trips_through_text_and_a_file() {
        let endpoint = Endpoint::new("127.0.0.1:9".parse().expect("addr"), "abc").expect("endpoint");
        assert_eq!(endpoint.to_string().parse(), Ok(endpoint.clone()));

        let path = std::env::temp_dir().join(format!("aimer_prendre_endpoint_{}", std::process::id()));
        endpoint.save(&path).expect("save");
        let loaded = Endpoint::load(&path);
        let _ = std::fs::remove_file(&path);
        assert_eq!(loaded.expect("load"), endpoint);
    }

    #[test]
    fn the_address_can_be_replaced_and_the_token_stays() {
        let endpoint = Endpoint::new("0.0.0.0:4001".parse().expect("addr"), "abc").expect("endpoint");
        let reachable = endpoint.with_ip("fd00::1".parse().expect("ip"));
        assert_eq!(reachable.addr(), "[fd00::1]:4001".parse().expect("addr"));
        assert_eq!(reachable.token(), "abc");
    }

    #[test]
    fn an_announcement_is_found_inside_a_decorated_log_line() {
        let endpoint = Endpoint::new("127.0.0.1:4001".parse().expect("addr"), "abc").expect("endpoint");
        let line = endpoint.announcement();
        assert_eq!(Endpoint::from_announcement(&line), Some(endpoint.clone()));
        // The app's logger adds a timestamp, a level and its own punctuation.
        let decorated = format!("2026-11-09T10:00:01Z INFO [aimer] {line}\r");
        assert_eq!(Endpoint::from_announcement(&decorated), Some(endpoint));
    }

    #[test]
    fn lines_that_are_not_announcements_are_ignored() {
        for line in [
            "",
            "aimer_prendre: could not start a client thread",
            "aimer_prendre: listening, endpoint is `nonsense`",
            "aimer_prendre: listening, endpoint is `127.0.0.1:9 a b`",
            "aimer_prendre: listening, endpoint is `127.0.0.1:9 abc",
        ] {
            assert_eq!(Endpoint::from_announcement(line), None, "{line:?}");
        }
    }

    #[test]
    fn malformed_endpoints_are_rejected() {
        let addr = "127.0.0.1:9".parse().expect("addr");
        assert!(Endpoint::new(addr, "").is_err());
        assert!(Endpoint::new(addr, "a b").is_err());
        for text in ["", "127.0.0.1:9", "nonsense token", "127.0.0.1:9 a b"] {
            assert!(text.parse::<Endpoint>().is_err(), "{text:?}");
        }
    }
}
