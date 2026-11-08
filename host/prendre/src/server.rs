use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{Command, Endpoint};

/// The longest line accepted, in bytes. Room for a large paste, small enough that
/// a misbehaving peer cannot make the app buffer without bound.
const MAX_LINE: usize = 64 * 1024;
/// How many clients may be connected at once; a driver needs one.
const MAX_CLIENTS: usize = 4;
/// How long a new connection has to present its token.
const HANDSHAKE: Duration = Duration::from_secs(5);
/// How often a blocked read wakes up to notice the server was stopped.
const POLL: Duration = Duration::from_millis(100);

type Sink = dyn Fn(Command) -> bool + Send + Sync;

/// How a [`Server`] is set up.
///
/// # Examples
///
/// ```
/// use aimer_prendre::Config;
///
/// // A fixed token, so a CI script can be written before the app starts.
/// let config = Config::new().port(0).token("ci-secret");
/// ```
#[derive(Clone, Debug, Default)]
pub struct Config {
    port: u16,
    token: Option<String>,
}

impl Config {
    /// An ephemeral port and a random token.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The loopback port to listen on; `0` (the default) lets the OS choose.
    #[inline]
    pub fn port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Uses `token` instead of a random one.
    ///
    /// It must be non-empty and free of whitespace, or [`Server::start`] fails.
    #[inline]
    pub fn token(mut self, token: impl Into<String>) -> Self {
        self.token = Some(token.into());
        self
    }
}

/// Receives synthetic input from another process and hands it to the app.
///
/// The server listens on the loopback interface only, and a client must present
/// the [`Endpoint`]'s token before anything it sends is acted on. Both matter:
/// on a shared machine, and on Android, other local processes can reach a
/// loopback port, and input injection is full control of the app. Start it
/// deliberately, for development and automation, rather than by default.
///
/// Dropping the server stops listening and ends its connections.
///
/// # Examples
///
/// ```no_run
/// use aimer_prendre::Server;
///
/// // Inside the app, after the event loop has started.
/// let server = Server::start(Default::default())?;
/// server.endpoint().save("aimer-prendre.endpoint")?;
/// # Ok::<(), std::io::Error>(())
/// ```
pub struct Server {
    endpoint: Endpoint,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server that delivers every accepted command to the running
    /// application through the platform event loop.
    ///
    /// A command that arrives before the loop exists is answered with an error
    /// and dropped, so a driver can retry.
    pub fn start(config: Config) -> io::Result<Self> {
        Self::start_with(config, |command| match aimer_platform::EVENT_PROXY.get() {
            Some(proxy) => proxy.send_event(command.into_platform_event()).is_ok(),
            None => false,
        })
    }

    /// Starts a server that delivers every accepted command to `sink`, which
    /// runs on a server thread and returns whether the command was taken.
    ///
    /// This is what [`Server::start`] is built on; use it to route input
    /// somewhere other than the platform loop, such as a test double.
    pub fn start_with(
        config: Config,
        sink: impl Fn(Command) -> bool + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, config.port))?;
        let token = config.token.unwrap_or_else(random_token);
        let endpoint = Endpoint::new(listener.local_addr()?, token)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            token: endpoint.token().to_owned(),
            sink: Box::new(sink),
            stop: Arc::clone(&stop),
            clients: AtomicUsize::new(0),
        });
        let accept = std::thread::Builder::new()
            .name("aimer-prendre-accept".into())
            .spawn(move || accept_loop(&listener, &shared))?;
        aimer_utils::info!("aimer_prendre: listening, endpoint is `{endpoint}`");
        Ok(Self { endpoint, stop, accept: Some(accept) })
    }

    /// The address and token a client connects with.
    #[inline]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // `accept` blocks; a throwaway connection wakes it to see the flag.
        let _ = TcpStream::connect_timeout(&self.endpoint.addr(), POLL);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

struct Shared {
    token: String,
    sink: Box<Sink>,
    stop: Arc<AtomicBool>,
    clients: AtomicUsize,
}

/// Counts a connection for as long as it lives.
struct Slot(Arc<Shared>);

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::AcqRel);
    }
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stop.load(Ordering::Acquire) {
            return;
        }
        let Ok(mut stream) = stream else { continue };
        if shared.clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
            shared.clients.fetch_sub(1, Ordering::AcqRel);
            let _ = stream.write_all(b"err busy\n");
            continue;
        }
        let slot = Slot(Arc::clone(shared));
        let spawned = std::thread::Builder::new()
            .name("aimer-prendre-client".into())
            .spawn(move || serve(stream, &slot));
        if let Err(e) = spawned {
            aimer_utils::error!("aimer_prendre: could not start a client thread: {e}");
        }
    }
}

fn serve(stream: TcpStream, slot: &Slot) {
    let shared = &*slot.0;
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(POLL));
    let _ = stream.set_write_timeout(Some(HANDSHAKE));
    let Ok(mut writer) = stream.try_clone() else { return };
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();

    let deadline = Instant::now() + HANDSHAKE;
    let hello = read_line(&mut reader, &mut line, &shared.stop, Some(deadline));
    let authorized = matches!(hello, Ok(Some(text))
        if text.strip_prefix("hello ").is_some_and(|t| same_token(t, &shared.token)));
    if !authorized {
        let _ = writer.write_all(b"err unauthorized\n");
        return;
    }
    if writer.write_all(b"ok\n").is_err() {
        return;
    }

    while let Ok(Some(text)) = read_line(&mut reader, &mut line, &shared.stop, None) {
        let reply = match text.parse::<Command>() {
            Ok(command) => {
                if (shared.sink)(command) { "ok\n".to_owned() } else { "err app unavailable\n".to_owned() }
            }
            Err(e) => format!("err {e}\n"),
        };
        if writer.write_all(reply.as_bytes()).is_err() {
            return;
        }
    }
}

/// Reads one line without its terminator.
///
/// Returns `Ok(None)` when the peer closed, the server stopped, the `deadline`
/// passed, or the line outgrew [`MAX_LINE`]: in each case the connection is over.
/// Bytes read before a poll timeout stay in `line`, so a slow sender is not
/// corrupted.
fn read_line(
    reader: &mut BufReader<TcpStream>,
    line: &mut Vec<u8>,
    stop: &AtomicBool,
    deadline: Option<Instant>,
) -> io::Result<Option<String>> {
    line.clear();
    loop {
        if stop.load(Ordering::Acquire) || deadline.is_some_and(|d| Instant::now() >= d) {
            return Ok(None);
        }
        let room = (MAX_LINE + 1 - line.len()) as u64;
        match reader.by_ref().take(room).read_until(b'\n', line) {
            Ok(0) => return Ok(None),
            Ok(_) if line.last() == Some(&b'\n') => {
                line.pop();
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                return Ok(std::str::from_utf8(line).ok().map(str::to_owned));
            }
            // No terminator and no room left.
            Ok(_) if line.len() > MAX_LINE => return Ok(None),
            Ok(_) => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e),
        }
    }
}

/// Compares without stopping at the first differing byte, so response time does
/// not teach a local attacker the token one character at a time.
fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    let mut diff = given.len() ^ expected.len();
    for (i, &byte) in expected.iter().enumerate() {
        diff |= usize::from(byte ^ given.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// 128 bits from the standard library's per-process random hash keys.
///
/// `RandomState` is seeded from the operating system, which is enough to keep
/// another local process from guessing a development token without adding a
/// dependency. Pass [`Config::token`] when stronger material is available.
fn random_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut token = String::with_capacity(32);
    for salt in 0..2u64 {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u128(nanos);
        let _ = write!(token, "{:016x}", hasher.finish());
    }
    token
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::sync::Mutex;

    use aimer_events::pointer::PointerButton;

    use super::*;
    use crate::Client;

    /// A server whose accepted commands arrive on the returned channel.
    fn collecting(config: Config) -> (Server, mpsc::Receiver<Command>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let server = Server::start_with(config, move |command| {
            tx.lock().expect("sender lock").send(command).is_ok()
        })
        .expect("server starts");
        (server, rx)
    }

    fn raw(server: &Server) -> (TcpStream, BufReader<TcpStream>) {
        let stream = TcpStream::connect(server.endpoint().addr()).expect("connect");
        stream.set_read_timeout(Some(Duration::from_secs(5))).expect("timeout");
        let reader = BufReader::new(stream.try_clone().expect("clone"));
        (stream, reader)
    }

    fn line(reader: &mut BufReader<TcpStream>) -> String {
        let mut text = String::new();
        reader.read_line(&mut text).expect("a reply");
        text.trim_end().to_owned()
    }

    #[test]
    fn an_authorised_client_delivers_commands_in_order() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let click = [
            Command::PointerMove { x: 5.0, y: 6.0 },
            Command::PointerDown { x: 5.0, y: 6.0, button: PointerButton::Primary },
            Command::PointerUp { x: 5.0, y: 6.0, button: PointerButton::Primary },
            Command::Text("héllo\nworld".into()),
        ];
        for command in &click {
            client.send(command).expect("accepted");
        }
        // `send` returns after the reply, which is after the sink ran.
        let got: Vec<Command> = rx.try_iter().collect();
        assert_eq!(got, click);
    }

    #[test]
    fn a_wrong_token_is_refused_and_nothing_is_delivered() {
        let (server, rx) = collecting(Config::new().token("right"));
        let wrong = Endpoint::new(server.endpoint().addr(), "wrong").expect("endpoint");
        let err = Client::connect(&wrong).err().expect("refused");
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);

        // Skipping the handshake gets no further than a wrong one.
        let (mut stream, mut reader) = raw(&server);
        stream.write_all(b"move 1 2\n").expect("write");
        assert_eq!(line(&mut reader), "err unauthorized");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_bad_command_is_reported_and_the_connection_stays_usable() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let (mut stream, mut reader) = raw(&server);
        stream.write_all(format!("hello {}\nmove NaN 1\nteleport\nleave\n", server.endpoint().token()).as_bytes()).expect("write");
        assert_eq!(line(&mut reader), "ok");
        assert_eq!(line(&mut reader), "err invalid x");
        assert_eq!(line(&mut reader), "err unknown command");
        assert_eq!(line(&mut reader), "ok");
        assert_eq!(rx.try_recv(), Ok(Command::PointerLeave));
        client.send(&Command::PointerLeave).expect("other clients are unaffected");
    }

    #[test]
    fn an_app_that_cannot_take_input_says_so() {
        let server = Server::start_with(Config::new(), |_| false).expect("server starts");
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let err = client.send(&Command::PointerLeave).expect_err("not taken");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "app unavailable");
    }

    #[test]
    fn a_line_that_never_ends_closes_the_connection() {
        let (server, rx) = collecting(Config::new());
        let (mut stream, mut reader) = raw(&server);
        stream.write_all(format!("hello {}\n", server.endpoint().token()).as_bytes()).expect("write");
        assert_eq!(line(&mut reader), "ok");
        let flood = vec![b'a'; MAX_LINE + 10];
        // The server may hang up mid-write; either outcome is the point.
        let _ = stream.write_all(&flood);
        let mut rest = String::new();
        assert_eq!(reader.read_line(&mut rest).unwrap_or(0), 0, "no reply, just EOF");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_split_write_is_not_corrupted_by_the_poll_timeout() {
        let (server, rx) = collecting(Config::new());
        let (mut stream, mut reader) = raw(&server);
        stream.write_all(format!("hello {}\ntext h", server.endpoint().token()).as_bytes()).expect("write");
        assert_eq!(line(&mut reader), "ok");
        std::thread::sleep(POLL * 3);
        stream.write_all("\u{e9}llo\n".as_bytes()).expect("write");
        assert_eq!(line(&mut reader), "ok");
        assert_eq!(rx.try_recv(), Ok(Command::Text("h\u{e9}llo".into())));
    }

    #[test]
    fn clients_beyond_the_cap_are_turned_away() {
        let (server, _rx) = collecting(Config::new());
        let held: Vec<Client> =
            (0..MAX_CLIENTS).map(|_| Client::connect(server.endpoint()).expect("within cap")).collect();
        let err = Client::connect(server.endpoint()).err().expect("over the cap");
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        drop(held);
    }

    #[test]
    fn dropping_the_server_stops_listening() {
        let (server, _rx) = collecting(Config::new());
        let endpoint = server.endpoint().clone();
        let mut client = Client::connect(&endpoint).expect("connect");
        drop(server);
        assert!(TcpStream::connect_timeout(&endpoint.addr(), Duration::from_millis(500)).is_err());
        // The open connection is closed within a poll or two.
        std::thread::sleep(POLL * 3);
        assert!(client.send(&Command::PointerLeave).is_err());
    }

    #[test]
    fn random_tokens_are_long_unique_and_line_safe() {
        let (a, b) = (random_token(), random_token());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert!(same_token("abc", "abc") && !same_token("abd", "abc") && !same_token("ab", "abc") && !same_token("abcd", "abc"));
    }

    #[test]
    fn a_whitespace_token_is_rejected_at_start() {
        let err = Server::start_with(Config::new().token("two words"), |_| true).err().expect("rejected");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
