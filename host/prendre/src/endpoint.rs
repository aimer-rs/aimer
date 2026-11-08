use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;

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

    /// The loopback address the server listens on.
    #[inline]
    pub fn addr(&self) -> SocketAddr {
        self.addr
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
    fn malformed_endpoints_are_rejected() {
        let addr = "127.0.0.1:9".parse().expect("addr");
        assert!(Endpoint::new(addr, "").is_err());
        assert!(Endpoint::new(addr, "a b").is_err());
        for text in ["", "127.0.0.1:9", "nonsense token", "127.0.0.1:9 a b"] {
            assert!(text.parse::<Endpoint>().is_err(), "{text:?}");
        }
    }
}
