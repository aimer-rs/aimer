use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::{Command, Endpoint};

/// How long a connect, a write or a reply may take before the call fails.
const TIMEOUT: Duration = Duration::from_secs(5);

/// Sends synthetic input to an app running a [`Server`](crate::Server).
///
/// Each [`send`](Client::send) waits for the server's answer, so a command that
/// returned `Ok` has been handed to the application's event loop, and one the
/// server refused comes back as an error rather than being lost.
///
/// # Examples
///
/// ```no_run
/// use aimer_prendre::{Client, Command, Endpoint};
///
/// let endpoint = Endpoint::load("aimer-prendre.endpoint")?;
/// let mut app = Client::connect(&endpoint)?;
/// app.send(&"down 120 48".parse::<Command>().expect("valid command"))?;
/// app.send(&Command::Text("hello".into()))?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Client {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
}

impl Client {
    /// Connects and proves knowledge of the endpoint's token.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::PermissionDenied`] when the token is wrong, and
    /// [`io::ErrorKind::ConnectionRefused`] when the server is full.
    pub fn connect(endpoint: &Endpoint) -> io::Result<Self> {
        Self::open(endpoint, None)
    }

    /// Connects like [`Client::connect`], and tells the app who is attached.
    ///
    /// The app shows `name` for as long as this client stays connected, so give
    /// it the tool's name (`aimer-cli`, `mcp:claude`) and keep the client alive
    /// for as long as the tool is attached. A name is one to 32 characters from
    /// letters, digits and `. _ : / @ + -`.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] for a name that is not one, and those of
    /// [`Client::connect`].
    pub fn connect_as(endpoint: &Endpoint, name: &str) -> io::Result<Self> {
        if !crate::hello::is_valid_name(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a valid client name"));
        }
        Self::open(endpoint, Some(name))
    }

    fn open(endpoint: &Endpoint, name: Option<&str>) -> io::Result<Self> {
        let writer = TcpStream::connect_timeout(&endpoint.addr(), TIMEOUT)?;
        writer.set_nodelay(true)?;
        writer.set_read_timeout(Some(TIMEOUT))?;
        writer.set_write_timeout(Some(TIMEOUT))?;
        let mut client = Self { reader: BufReader::new(writer.try_clone()?), writer };
        let hello = crate::hello::line(endpoint.token(), name);
        client.writer.write_all(format!("{hello}\n").as_bytes())?;
        match client.reply()?.as_str() {
            "ok" => Ok(client),
            "err busy" => Err(io::Error::new(io::ErrorKind::ConnectionRefused, "server is busy")),
            _ => Err(io::Error::new(io::ErrorKind::PermissionDenied, "the token was refused")),
        }
    }

    /// Sends one command and waits for the app to take it.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] when the server refused the command (it
    /// was malformed, or the app is not running its event loop yet), carrying the
    /// server's reason; any other kind means the connection failed.
    pub fn send(&mut self, command: &Command) -> io::Result<()> {
        self.writer.write_all(format!("{command}\n").as_bytes())?;
        match self.reply()? {
            ok if ok == "ok" => Ok(()),
            err => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                err.strip_prefix("err ").unwrap_or(&err).to_owned(),
            )),
        }
    }

    fn reply(&mut self) -> io::Result<String> {
        let mut reply = String::new();
        if self.reader.read_line(&mut reply)? == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        reply.truncate(reply.trim_end().len());
        Ok(reply)
    }
}
