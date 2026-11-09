use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::{Command, Endpoint, Role};

/// How long a picture may take: capturing it waits for a frame, and it is then
/// shrunk, encoded and sent.
const PICTURE_TIMEOUT: Duration = Duration::from_secs(20);

/// A picture of the app's window, as the app encoded it.
#[derive(Clone, Debug, PartialEq)]
pub struct Screenshot {
    /// The MIME type of `bytes`: `image/png` or `image/jpeg`.
    pub mime: String,
    /// The picture's width in pixels.
    pub width: u32,
    /// The picture's height in pixels.
    pub height: u32,
    /// How many of the picture's pixels make a logical pixel of the window: `1`
    /// for the default, where a point in the picture is a point to click, and less
    /// when it was asked to be smaller.
    pub scale: f32,
    /// The encoded picture, ready to write to a file.
    pub bytes: Vec<u8>,
}

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
        Self::open(endpoint, None, Role::Agent)
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
        Self::connect_with(endpoint, name, Role::Agent)
    }

    /// Connects, naming this client and saying what it is for.
    ///
    /// The tool that launched the app and holds a connection for the whole run
    /// says [`Role::Launcher`], so the app does not take its presence for someone
    /// controlling it; an agent says [`Role::Agent`], which is what
    /// [`Client::connect_as`] does.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] for a name that is not one, and those of
    /// [`Client::connect`].
    pub fn connect_with(endpoint: &Endpoint, name: &str, role: Role) -> io::Result<Self> {
        if !crate::hello::is_valid_name(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "not a valid client name"));
        }
        Self::open(endpoint, Some(name), role)
    }

    fn open(endpoint: &Endpoint, name: Option<&str>, role: Role) -> io::Result<Self> {
        let writer = TcpStream::connect_timeout(&endpoint.addr(), TIMEOUT)?;
        writer.set_nodelay(true)?;
        writer.set_read_timeout(Some(TIMEOUT))?;
        writer.set_write_timeout(Some(TIMEOUT))?;
        let mut client = Self { reader: BufReader::new(writer.try_clone()?), writer };
        let hello = crate::hello::line(endpoint.token(), name, role);
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

    /// Asks for a picture of the app's window and reads it.
    ///
    /// `width` is the widest it may be; without it the picture is the window's
    /// logical width, so a point in it is a point that can be clicked, and `0`
    /// keeps the real pixels. `jpeg` is a quality from 1 to 100 for a much smaller
    /// JPEG; without it the picture is a lossless PNG.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] with the server's reason when it could not
    /// take the picture — the renderer cannot capture, the window is hidden, or
    /// the picture is too big — and any other kind when the connection failed.
    pub fn screenshot(&mut self, width: Option<u32>, jpeg: Option<u8>) -> io::Result<Screenshot> {
        let line = Command::Screenshot { width, jpeg }.to_string();
        self.writer.write_all(format!("{line}\n").as_bytes())?;
        // A picture is drawn, shrunk, encoded and sent: longer than an input.
        self.writer.set_read_timeout(Some(PICTURE_TIMEOUT))?;
        let outcome = self.read_picture();
        self.writer.set_read_timeout(Some(TIMEOUT))?;
        outcome
    }

    fn read_picture(&mut self) -> io::Result<Screenshot> {
        let header = self.reply()?;
        let bad = || io::Error::new(io::ErrorKind::InvalidData, "a malformed screenshot header");
        let mut words = header.split_ascii_whitespace();
        let (Some("ok"), Some("image")) = (words.next(), words.next()) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                header.strip_prefix("err ").unwrap_or(&header).to_owned(),
            ));
        };
        let mime = words.next().ok_or_else(bad)?.to_owned();
        let length: usize = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
        let width: u32 = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
        let height: u32 = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
        let scale: f32 = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
        if length > crate::command::MAX_IMAGE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a screenshot larger than the limit"));
        }
        let mut bytes = vec![0u8; length];
        self.reader.read_exact(&mut bytes)?;
        Ok(Screenshot { mime, width, height, scale, bytes })
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
