use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::motion::Pointer;
use crate::trackpad;
use crate::typing;
use crate::{Command, Endpoint, Role, env, hello};

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
/// Takes a picture of the window.
type CaptureFn = dyn Fn() -> Result<aimer_capture::Image, aimer_capture::CaptureError> + Send + Sync;
/// Told, with every name, whenever someone attaches or detaches.
type Presence = dyn Fn(&[Attached]) + Send + Sync;

/// One connected client, as the app is told about it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attached {
    /// What the client called itself, or `driver` when it did not.
    pub name: String,
    /// What the connection is for.
    pub role: Role,
}

/// How long a command that waits for the mouse waits for it. Shorter than the
/// time a client waits for any reply.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(4);

/// What a client that gave no name is called.
const UNNAMED: &str = "driver";

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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    bind: IpAddr,
    port: u16,
    token: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self { bind: Ipv4Addr::LOCALHOST.into(), port: 0, token: None }
    }
}

impl Config {
    /// The loopback interface, an ephemeral port and a random token.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// The address to listen on; the default is `127.0.0.1`.
    ///
    /// **Anything but a loopback address exposes the app to every device that can
    /// reach this one**, and the token becomes the only thing between them and
    /// full control of it. It exists for a phone, whose loopback is not
    /// reachable from the machine driving it; bind the narrowest address that
    /// works and keep the server out of release builds. `0.0.0.0` and `::` listen
    /// on every interface, and `::` also accepts IPv4 on most systems.
    ///
    /// The [`Endpoint`] then reports that address. If it is unspecified, say how
    /// this machine is reached with [`Endpoint::with_ip`].
    #[inline]
    pub fn bind(mut self, ip: IpAddr) -> Self {
        self.bind = ip;
        self
    }

    /// The port to listen on; `0` (the default) lets the OS choose.
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

    /// Reads the configuration a launcher put in the environment, or `None` when
    /// it did not ask for a server. See [`env`](crate::env) for the variables.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidInput`] naming the variable, when the server was
    /// asked for and a setting cannot be read. Falling back to a default would
    /// start a server somewhere the launcher did not expect.
    pub fn from_env() -> io::Result<Option<Self>> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// [`Config::from_env`] over any source of variables.
    fn from_vars(get: impl Fn(&str) -> Option<String>) -> io::Result<Option<Self>> {
        if get(env::ENABLE).is_none_or(|on| on.is_empty() || on == "0") {
            return Ok(None);
        }
        let bad = |name: &str, value: &str| {
            io::Error::new(io::ErrorKind::InvalidInput, format!("{name}={value:?} cannot be read"))
        };
        let mut config = Self::new();
        if let Some(value) = get(env::BIND) {
            config.bind = value.parse().map_err(|_| bad(env::BIND, &value))?;
        }
        if let Some(value) = get(env::PORT) {
            config.port = value.parse().map_err(|_| bad(env::PORT, &value))?;
        }
        config.token = get(env::TOKEN);
        Ok(Some(config))
    }
}

/// Receives synthetic input from another process and hands it to the app.
///
/// The server listens on the loopback interface unless [`Config::bind`] says
/// otherwise, and a client must present the [`Endpoint`]'s token before anything
/// it sends is acted on. Both matter: on a shared machine, and on Android, other
/// local processes can reach a loopback port, and input injection is full
/// control of the app. Start it
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
    shared: Arc<Shared>,
    accept: Option<JoinHandle<()>>,
    /// Walks the injected mouse while it glides.
    pointer: Option<JoinHandle<()>>,
}

impl Server {
    /// Starts a server if the launcher asked for one in the environment, and
    /// returns `None` if it did not.
    ///
    /// This is the call an app makes unconditionally, in debug builds, so that
    /// `aimer run` can drive it without the app changing: nothing listens unless
    /// [`env::ENABLE`] is set. Keep the returned server alive for as long as
    /// input should be accepted.
    ///
    /// # Errors
    ///
    /// As [`Config::from_env`], and as [`Server::start`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// // Held until the app exits.
    /// let _prendre = aimer_prendre::Server::start_from_env()?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub fn start_from_env() -> io::Result<Option<Self>> {
        Config::from_env()?.map(Self::start).transpose()
    }

    /// Starts a server that delivers every accepted command to the running
    /// application through the platform event loop.
    ///
    /// A command that arrives before the loop exists is answered with an error
    /// and dropped, so a driver can retry.
    pub fn start(config: Config) -> io::Result<Self> {
        Self::start_with_presence(config, Command::deliver, |attached| {
            // The debug overlay shows who is attached; it reads this on the loop.
            let everyone = attached.iter().map(|a| a.name.clone()).collect();
            let agents = attached.iter().filter(|a| a.role == Role::Agent).map(|a| a.name.clone()).collect();
            if aimer_platform::attachment::set_connections(everyone, agents)
                && let Some(proxy) = aimer_platform::EVENT_PROXY.get()
            {
                let _ = proxy.send_event(aimer_platform::AimerNativePlatformEvent::AttachmentChanged);
            }
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
        Self::start_with_presence(config, sink, |_| {})
    }

    /// [`Server::start_with`], and `presence` is told who is attached, with their
    /// roles, in the order they attached, whenever that list changes.
    ///
    /// A client is attached from the moment its hello is accepted until its
    /// connection ends, and one that gave no name is called `driver`. `presence`
    /// runs on a server thread while the list is held, so it must be quick and
    /// must not call back into the server.
    pub fn start_with_presence(
        config: Config,
        sink: impl Fn(Command) -> bool + Send + Sync + 'static,
        presence: impl Fn(&[Attached]) + Send + Sync + 'static,
    ) -> io::Result<Self> {
        Self::start_with_capture(config, sink, presence, || {
            aimer_capture::screenshot(aimer_capture::DEFAULT_TIMEOUT)
        })
    }

    /// [`Server::start_with_presence`], and `capture` takes the picture a
    /// `screenshot` command asks for instead of the running window's renderer.
    ///
    /// It runs on the connection's own thread, and is what lets a test stand in a
    /// fixed picture for a renderer.
    #[doc(hidden)]
    pub fn start_with_capture(
        config: Config,
        sink: impl Fn(Command) -> bool + Send + Sync + 'static,
        presence: impl Fn(&[Attached]) + Send + Sync + 'static,
        capture: impl Fn() -> Result<aimer_capture::Image, aimer_capture::CaptureError> + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind((config.bind, config.port))?;
        let token = config.token.unwrap_or_else(random_token);
        let endpoint = Endpoint::new(listener.local_addr()?, token)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            token: endpoint.token().to_owned(),
            sink: Box::new(sink),
            presence: Box::new(presence),
            capture: Box::new(capture),
            pointer: Pointer::new(),
            attached: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(0),
            stop: Arc::clone(&stop),
            clients: AtomicUsize::new(0),
        });
        let driver = Arc::clone(&shared);
        let pointer = std::thread::Builder::new().name("aimer-prendre-pointer".into()).spawn(move || {
            driver.pointer.drive(&driver.stop, |(x, y), modifiers| {
                (driver.sink)(Command::PointerMove { x, y, modifiers })
            });
        })?;
        let accepting = Arc::clone(&shared);
        let accept = std::thread::Builder::new()
            .name("aimer-prendre-accept".into())
            .spawn(move || accept_loop(&listener, &accepting))?;
        if !config.bind.is_loopback() {
            aimer_utils::warn!(
                "aimer_prendre: listening on {}, beyond loopback: only the token protects this app",
                config.bind
            );
        }
        aimer_utils::info!("{}", endpoint.announcement());
        Ok(Self { endpoint, stop, shared, accept: Some(accept), pointer: Some(pointer) })
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
        // A glide in progress, and anything waiting for it, is asked to stop.
        self.shared.pointer.wake();
        // `accept` blocks; a throwaway connection wakes it to see the flag.
        let _ = TcpStream::connect_timeout(&wake_addr(self.endpoint.addr()), POLL);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
        if let Some(pointer) = self.pointer.take() {
            let _ = pointer.join();
        }
    }
}

/// Where to connect to reach a listener bound to `bound`: an unspecified address
/// is not one that can be connected to everywhere, so use this machine's own.
fn wake_addr(bound: SocketAddr) -> SocketAddr {
    let ip = match bound.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => Ipv4Addr::LOCALHOST.into(),
        IpAddr::V6(ip) if ip.is_unspecified() => std::net::Ipv6Addr::LOCALHOST.into(),
        ip => ip,
    };
    SocketAddr::new(ip, bound.port())
}

struct Shared {
    token: String,
    sink: Box<Sink>,
    presence: Box<Presence>,
    /// Takes a picture of the window for a `screenshot`.
    capture: Box<CaptureFn>,
    /// The injected mouse, which glides on a thread of its own.
    pointer: Pointer,
    /// Who is attached, with an id so a client leaves its own entry and no other.
    attached: Mutex<Vec<(u64, Attached)>>,
    next_id: AtomicU64,
    stop: Arc<AtomicBool>,
    clients: AtomicUsize,
}

impl Shared {
    /// Hands `command` to the app, or does what it asks of the mouse, and says why
    /// it could not.
    ///
    /// A glide only sets the mouse going and returns. A command that puts the
    /// mouse somewhere — `move`, `down`, `up` — first waits for a glide in
    /// progress to arrive, so `glide A`, `down A`, `up A` is a click on `A` and
    /// not a press that cuts the glide short. Everything else is not held up.
    fn run(&self, command: Command) -> Result<(), &'static str> {
        match command {
            Command::PointerGlide { x, y, millis, modifiers } => {
                if let Some((x, y)) = self.pointer.glide((x, y), millis, modifiers) {
                    // Nowhere to glide from: the mouse is simply there.
                    self.take(Command::PointerMove { x, y, modifiers })?;
                }
                Ok(())
            }
            Command::Settle => {
                if self.pointer.wait_settled(&self.stop, SETTLE_TIMEOUT) { Ok(()) } else { Err("timed out") }
            }
            Command::TrackpadScroll { dx, dy, millis, modifiers } => self.swipe((dx, dy), millis, modifiers),
            Command::Typing { text, millis } => self.type_out(&text, millis),
            Command::PointerMove { x, y, .. } | Command::PointerDown { x, y, .. } | Command::PointerUp { x, y, .. } => {
                if !self.pointer.wait_settled(&self.stop, SETTLE_TIMEOUT) {
                    self.pointer.cancel();
                }
                self.pointer.place(Some((x, y)));
                self.take(command)
            }
            Command::PointerDownHere { button, modifiers } | Command::PointerUpHere { button, modifiers } => {
                // Where the mouse is once any glide has arrived: that is where a person
                // who did not name a point means.
                if !self.pointer.wait_settled(&self.stop, SETTLE_TIMEOUT) {
                    self.pointer.cancel();
                }
                let (x, y) = self.pointer.current().ok_or("the mouse has not been placed: give a position")?;
                self.take(if matches!(command, Command::PointerDownHere { .. }) {
                    Command::PointerDown { x, y, button, modifiers }
                } else {
                    Command::PointerUp { x, y, button, modifiers }
                })
            }
            Command::PointerLeave => {
                self.pointer.place(None);
                self.take(command)
            }
            other => self.take(other),
        }
    }

    /// Scrolls the way a trackpad does: a paced stream of small pixel deltas, with
    /// the finger landing before the first and lifting after the last. Holds this
    /// connection until the swipe is over, and stops at once if the app stops
    /// taking input or the server is shut down.
    fn swipe(&self, by: (f32, f32), millis: Option<u32>, modifiers: crate::model::Modifiers) -> Result<(), &'static str> {
        use crate::model::{ScrollDeltaKind, TouchPhase};
        let plan = trackpad::plan(by.0, by.1, millis);
        if plan.steps.is_empty() {
            return Ok(());
        }
        let scroll = |dx, dy, phase| Command::Scroll { dx, dy, kind: ScrollDeltaKind::Pixel, phase, modifiers };
        let last = plan.steps.len() - 1;
        let started = Instant::now();
        for (index, (dx, dy)) in plan.steps.into_iter().enumerate() {
            if self.stop.load(Ordering::Acquire) {
                return Err("app unavailable");
            }
            self.take(scroll(dx, dy, if index == 0 { TouchPhase::Started } else { TouchPhase::Moved }))?;
            if index < last {
                // Due times are counted from the start, so a late wake-up is made up
                // by the next step and the cadence the app reads as a trackpad holds.
                let due = started + plan.interval * (index as u32 + 1);
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
            }
        }
        // The finger lifts: the end of the gesture carries no distance.
        self.take(scroll(0.0, 0.0, TouchPhase::Ended))
    }

    /// Types the way a person does: a keystroke at a time on an uneven rhythm.
    /// Holds this connection until the last one is in, and stops at once if the
    /// app stops taking input or the server is shut down.
    fn type_out(&self, text: &str, millis: Option<u32>) -> Result<(), &'static str> {
        let started = Instant::now();
        let mut elapsed = Duration::ZERO;
        for (keystroke, wait) in typing::plan(text, millis).steps {
            if self.stop.load(Ordering::Acquire) {
                return Err("app unavailable");
            }
            self.take(Command::Text(keystroke))?;
            elapsed += wait;
            // Counted from the start, so a late wake-up is made up by the next key.
            if !wait.is_zero()
                && let Some(left) = (started + elapsed).checked_duration_since(Instant::now())
            {
                std::thread::sleep(left);
            }
        }
        Ok(())
    }

    /// The picture a `screenshot` asks for, encoded: captured, shrunk and encoded
    /// here, on the connection's own thread, never the one drawing the window.
    fn screenshot(&self, width: Option<u32>, jpeg: Option<u8>) -> Result<Shot, String> {
        use aimer_capture::Format;
        let image = (self.capture)().map_err(|e| e.to_string())?;
        let target = match width {
            // The window's logical width: a point in the picture is a point to click.
            None => (image.width() as f32 / image.scale()).round().max(1.0) as u32,
            Some(width) => width,
        };
        let image = image.scaled_to_width(target);
        let format = jpeg.map_or(Format::Png, |quality| Format::Jpeg { quality });
        let bytes = image.encode(format).map_err(|e| e.to_string())?;
        if bytes.len() > crate::command::MAX_IMAGE_BYTES {
            return Err(format!(
                "the picture is {} bytes, over the {} byte limit: ask for a smaller one with width= or jpeg=",
                bytes.len(),
                crate::command::MAX_IMAGE_BYTES
            ));
        }
        Ok(Shot { mime: format.mime(), bytes, width: image.width(), height: image.height(), scale: image.scale() })
    }

    fn take(&self, command: Command) -> Result<(), &'static str> {
        if (self.sink)(command) { Ok(()) } else { Err("app unavailable") }
    }

    /// Lists `name` as attached until the returned guard is dropped.
    fn attach(&self, name: &str, role: Role) -> Attachment<'_> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut attached = self.attached.lock().expect("attached mutex poisoned");
        attached.push((id, Attached { name: name.to_owned(), role }));
        self.report(&attached);
        Attachment { shared: self, id }
    }

    fn report(&self, attached: &[(u64, Attached)]) {
        let list: Vec<Attached> = attached.iter().map(|(_, who)| who.clone()).collect();
        (self.presence)(&list);
    }
}

/// A client that is attached for as long as this lives.
struct Attachment<'a> {
    shared: &'a Shared,
    id: u64,
}

impl Drop for Attachment<'_> {
    fn drop(&mut self) {
        let mut attached = self.shared.attached.lock().expect("attached mutex poisoned");
        attached.retain(|(id, _)| *id != self.id);
        self.shared.report(&attached);
    }
}

/// A picture ready to send.
struct Shot {
    mime: &'static str,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    scale: f32,
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
    let hello = match &hello {
        Ok(Some(text)) => hello::parse(text).filter(|h| same_token(h.token, &shared.token)),
        _ => None,
    };
    let Some(hello) = hello else {
        let _ = writer.write_all(b"err unauthorized\n");
        return;
    };
    // The name is shown by the app, so it must be plain before it goes anywhere.
    if hello.name.is_some_and(|name| !hello::is_valid_name(name)) {
        let _ = writer.write_all(b"err invalid name\n");
        return;
    }
    // Listed before the reply, so a client that has its `ok` is already attached.
    let _attached = shared.attach(hello.name.unwrap_or(UNNAMED), hello.role);
    if writer.write_all(b"ok\n").is_err() {
        return;
    }

    while let Ok(Some(text)) = read_line(&mut reader, &mut line, &shared.stop, None) {
        let reply = match text.parse::<Command>() {
            // The one reply that is not a line: a header, and then the picture.
            Ok(Command::Screenshot { width, jpeg }) => match shared.screenshot(width, jpeg) {
                Ok(shot) => {
                    let header = format!("ok image {} {} {} {} {}\n", shot.mime, shot.bytes.len(), shot.width, shot.height, shot.scale);
                    if writer.write_all(header.as_bytes()).is_err() || writer.write_all(&shot.bytes).is_err() {
                        return;
                    }
                    continue;
                }
                Err(reason) => format!("err {reason}\n"),
            },
            Ok(command) => match shared.run(command) {
                Ok(()) => "ok\n".to_owned(),
                Err(reason) => format!("err {reason}\n"),
            },
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

// The tests drive the server with a real `Client`.
#[cfg(all(test, feature = "client"))]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;
    use std::sync::mpsc;
    use std::sync::Mutex;

    use crate::model::PointerButton;

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

    /// A server that reports who is attached, with their roles, to the returned channel.
    fn presence_with_roles(config: Config) -> (Server, mpsc::Receiver<Vec<Attached>>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let server = Server::start_with_presence(config, |_| true, move |attached| {
            let _ = tx.lock().expect("tx").send(attached.to_vec());
        })
        .expect("start");
        (server, rx)
    }

    /// The same, with the names only.
    fn presence(config: Config) -> (Server, mpsc::Receiver<Vec<String>>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let server = Server::start_with_presence(config, |_| true, move |attached| {
            let _ = tx.lock().expect("tx").send(attached.iter().map(|a| a.name.clone()).collect());
        })
        .expect("start");
        (server, rx)
    }

    fn next(rx: &mpsc::Receiver<Vec<String>>) -> Vec<String> {
        rx.recv_timeout(Duration::from_secs(3)).expect("a presence report")
    }

    #[test]
    fn what_each_client_is_for_is_reported_with_its_name() {
        let (server, rx) = presence_with_roles(Config::new());
        let launcher = Client::connect_with(server.endpoint(), "aimer-cli", Role::Launcher).expect("connect");
        let agent = Client::connect_with(server.endpoint(), "mcp:codex", Role::Agent).expect("connect");
        let plain = Client::connect_as(server.endpoint(), "script").expect("connect");
        let roles = |attached: Vec<Attached>| attached.into_iter().map(|a| (a.name, a.role)).collect::<Vec<_>>();
        let reports: Vec<_> = (0..3).map(|_| roles(rx.recv_timeout(Duration::from_secs(3)).expect("a report"))).collect();
        assert_eq!(reports[0], [("aimer-cli".to_owned(), Role::Launcher)]);
        assert_eq!(reports[1], [("aimer-cli".to_owned(), Role::Launcher), ("mcp:codex".to_owned(), Role::Agent)]);
        assert_eq!(reports[2].last(), Some(&("script".to_owned(), Role::Agent)), "a client that names itself with no role is an agent");

        drop(agent);
        let after = roles(rx.recv_timeout(Duration::from_secs(3)).expect("a report"));
        assert_eq!(after, [("aimer-cli".to_owned(), Role::Launcher), ("script".to_owned(), Role::Agent)], "the launcher stays when the agent goes");
        drop((launcher, plain));
    }

    #[test]
    fn who_is_attached_is_reported_as_they_come_and_go() {
        let (server, rx) = presence(Config::new());
        let first = Client::connect_as(server.endpoint(), "aimer-cli").expect("connect");
        assert_eq!(next(&rx), ["aimer-cli"]);
        let second = Client::connect(server.endpoint()).expect("connect");
        assert_eq!(next(&rx), ["aimer-cli", "driver"], "a client that gave no name is a driver");
        drop(first);
        assert_eq!(next(&rx), ["driver"]);
        drop(second);
        assert_eq!(next(&rx), Vec::<String>::new());
    }

    #[test]
    fn a_refused_client_is_never_reported_as_attached() {
        let (server, rx) = presence(Config::new());
        let wrong = Endpoint::new(server.endpoint().addr(), "wrong").expect("endpoint");
        assert!(Client::connect_as(&wrong, "intruder").is_err());
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "nobody attached");
    }

    #[test]
    fn a_name_that_is_not_plain_is_refused_before_it_can_be_shown() {
        let (server, rx) = presence(Config::new());
        let (mut stream, mut reader) = raw(&server);
        let token = server.endpoint().token().to_owned();
        stream.write_all(format!("hello {token} bad`name\n").as_bytes()).expect("write");
        assert_eq!(line(&mut reader), "err invalid name");
        assert!(rx.recv_timeout(Duration::from_millis(300)).is_err());
        assert!(Client::connect_as(server.endpoint(), "two words").is_err(), "refused by the client too");
    }

    fn moves(commands: &[Command]) -> Vec<(f32, f32)> {
        commands
            .iter()
            .filter_map(|c| if let Command::PointerMove { x, y, .. } = c { Some((*x, *y)) } else { None })
            .collect()
    }

    fn send(client: &mut Client, line: &str) {
        client.send(&line.parse().expect("a command")).unwrap_or_else(|e| panic!("`{line}`: {e}"));
    }

    /// A server that stamps the time each command reaches the app.
    fn timed(config: Config) -> (Server, mpsc::Receiver<(Instant, Command)>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let server = Server::start_with(config, move |command| {
            tx.lock().expect("tx").send((Instant::now(), command)).is_ok()
        })
        .expect("start");
        (server, rx)
    }

    fn scrolls(got: &[(Instant, Command)]) -> Vec<(Instant, f32, f32, crate::model::TouchPhase)> {
        got.iter()
            .filter_map(|(at, c)| match c {
                Command::Scroll { dx, dy, kind: crate::model::ScrollDeltaKind::Pixel, phase, .. } => Some((*at, *dx, *dy, *phase)),
                _ => None,
            })
            .collect()
    }

    /// A server whose screenshots are a fixed picture, standing in for a renderer.
    fn with_picture(width: u32, height: u32, scale: f32) -> Server {
        Server::start_with_capture(
            Config::new(),
            |_| true,
            |_| {},
            move || {
                let rgba: Vec<u8> = (0..width * height).flat_map(|i| [(i % 251) as u8, 40, 200, 255]).collect();
                Ok(aimer_capture::Image::new(width, height, scale, rgba).expect("a picture"))
            },
        )
        .expect("start")
    }

    #[test]
    fn a_screenshot_comes_back_as_a_header_and_exactly_that_many_png_bytes_at_logical_size() {
        let server = with_picture(400, 300, 2.0);
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let shot = client.screenshot(None, None).expect("a picture");
        assert_eq!(shot.mime, "image/png");
        // 400 physical pixels at 2x is 200 logical: the picture a person can point at.
        assert_eq!((shot.width, shot.height), (200, 150));
        assert!((shot.scale - 1.0).abs() < 0.01, "pixels in it are logical pixels: {}", shot.scale);
        let decoded = ::image::load_from_memory(&shot.bytes).expect("a PNG");
        assert_eq!((decoded.width(), decoded.height()), (200, 150));
    }

    #[test]
    fn a_width_caps_the_picture_and_zero_means_the_real_pixels() {
        let server = with_picture(400, 300, 2.0);
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let small = client.screenshot(Some(100), None).expect("a picture");
        assert_eq!((small.width, small.height), (100, 75));
        assert!((small.scale - 0.5).abs() < 0.01, "400 to 100 at 2x: {}", small.scale);
        let full = client.screenshot(Some(0), None).expect("a picture");
        assert_eq!((full.width, full.height, full.scale), (400, 300, 2.0), "nothing thrown away");
    }

    #[test]
    fn jpeg_is_a_smaller_picture_with_its_own_mime_type() {
        let server = with_picture(400, 300, 1.0);
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let png = client.screenshot(Some(0), None).expect("png");
        let jpeg = client.screenshot(Some(0), Some(60)).expect("jpeg");
        assert_eq!(jpeg.mime, "image/jpeg");
        assert_eq!(&jpeg.bytes[..2], &[0xFF, 0xD8]);
        assert!(jpeg.bytes.len() < png.bytes.len(), "{} vs {}", jpeg.bytes.len(), png.bytes.len());
    }

    #[test]
    fn the_connection_is_still_good_after_a_picture() {
        let server = with_picture(64, 48, 1.0);
        let mut client = Client::connect(server.endpoint()).expect("connect");
        client.screenshot(None, None).expect("a picture");
        send(&mut client, "settle");
        client.screenshot(None, None).expect("and another");
    }

    #[test]
    fn a_renderer_that_cannot_capture_is_reported_as_an_error_not_a_hang() {
        let server = Server::start_with_capture(Config::new(), |_| true, |_| {}, || Err(aimer_capture::CaptureError::Unsupported))
            .expect("start");
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let err = client.screenshot(None, None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("cannot capture"), "{err}");
        send(&mut client, "settle");
    }

    #[test]
    fn a_picture_too_big_to_send_says_how_to_make_it_smaller() {
        // Wide noise that PNG cannot squeeze.
        let server = Server::start_with_capture(Config::new(), |_| true, |_| {}, || {
            let rgba: Vec<u8> = (0..4000u32 * 3000).flat_map(|i| {
                let h = i.wrapping_mul(2_654_435_761);
                [(h >> 24) as u8, (h >> 16) as u8, (h >> 8) as u8, 255]
            }).collect();
            Ok(aimer_capture::Image::new(4000, 3000, 1.0, rgba).expect("a picture"))
        })
        .expect("start");
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let err = client.screenshot(Some(0), None).unwrap_err();
        assert!(err.to_string().contains("width="), "{err}");
    }

    #[test]
    fn a_trackpad_scroll_is_a_paced_stream_of_pixel_deltas_that_starts_and_ends_like_a_gesture() {
        use crate::model::TouchPhase;
        let (server, rx) = timed(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let started = Instant::now();
        send(&mut client, "trackpad 0 -400 500");
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(430) && took <= Duration::from_millis(650), "about as long as asked: {took:?}");

        let got: Vec<_> = rx.try_iter().collect();
        let steps = scrolls(&got);
        assert!(got.iter().all(|(_, c)| matches!(c, Command::Scroll { .. })), "only scrolling reaches the app");
        assert!(steps.len() > 25, "a stream, not a pulse: {} events", steps.len());
        let (first, last) = (steps.first().unwrap(), steps.last().unwrap());
        assert_eq!(first.3, TouchPhase::Started, "the finger lands");
        assert_eq!((last.3, last.1, last.2), (TouchPhase::Ended, 0.0, 0.0), "and lifts, carrying no distance");
        assert!(steps[1..steps.len() - 1].iter().all(|s| s.3 == TouchPhase::Moved));
        let travelled: f32 = steps.iter().map(|s| s.2).sum();
        assert!((travelled + 400.0).abs() < 0.01, "exactly the distance: {travelled}");

        // Faster than the app's 20 ms line between a trackpad and a wheel, and never a long pause.
        let gaps: Vec<f32> = steps.windows(2).map(|w| w[1].0.duration_since(w[0].0).as_secs_f32() * 1000.0).collect();
        let mean = gaps.iter().sum::<f32>() / gaps.len() as f32;
        assert!(mean < 16.0, "mean gap {mean} ms");
        assert!(gaps.iter().cloned().fold(0.0, f32::max) < 60.0, "no stall: {gaps:?}");
    }

    #[test]
    fn typing_is_one_keystroke_at_a_time_at_a_human_pace_and_holds_the_connection_until_done() {
        let (server, rx) = timed(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let started = Instant::now();
        send(&mut client, "typing 600 Hello, Aimer!");
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(540) && took <= Duration::from_millis(800), "about as long as asked: {took:?}");

        let got: Vec<_> = rx.try_iter().collect();
        let typed: Vec<&str> = got.iter().map(|(_, c)| match c {
            Command::Text(text) => text.as_str(),
            other => panic!("only text reaches the app: {other:?}"),
        }).collect();
        assert_eq!(typed.concat(), "Hello, Aimer!");
        assert_eq!(typed.len(), 13, "a keystroke per character: {typed:?}");
        let gaps: Vec<f32> = got.windows(2).map(|w| w[1].0.duration_since(w[0].0).as_secs_f32() * 1000.0).collect();
        assert!(gaps.iter().all(|g| *g > 5.0), "paced, not a burst: {gaps:?}");
        assert!(gaps.iter().any(|g| (g - gaps[0]).abs() > 3.0), "and uneven: {gaps:?}");
    }

    #[test]
    fn typing_stops_when_the_app_stops_taking_input() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let server = Server::start_with(Config::new(), move |_| seen.fetch_add(1, Ordering::SeqCst) < 2).expect("server starts");
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let err = client.send(&"typing 300 hello".parse().expect("parses")).expect_err("the app went away");
        assert_eq!(err.to_string(), "app unavailable");
        assert_eq!(calls.load(Ordering::SeqCst), 3, "no keystrokes after the first refusal");
    }

    #[test]
    fn a_trackpad_scroll_does_not_move_the_mouse_and_ignores_a_glide_in_progress() {
        let (server, rx) = timed(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 50 50");
        rx.try_iter().for_each(drop);
        send(&mut client, "trackpad 0 -120 200");
        let got: Vec<_> = rx.try_iter().collect();
        assert!(got.iter().all(|(_, c)| !matches!(c, Command::PointerMove { .. })), "it scrolls where the mouse is");
    }

    #[test]
    fn a_scroll_too_small_to_see_is_replied_to_and_sends_nothing() {
        let (server, rx) = timed(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "trackpad 0 0.1");
        assert_eq!(rx.try_iter().count(), 0);
    }

    #[test]
    fn dropping_the_server_mid_scroll_ends_it_promptly() {
        let (server, _rx) = timed(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let handle = std::thread::spawn(move || {
            let _ = client.send(&"trackpad 0 -3000 3000".parse().expect("command"));
        });
        std::thread::sleep(Duration::from_millis(150));
        let started = Instant::now();
        drop(server);
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
        let _ = handle.join();
    }

    #[test]
    fn a_glide_replies_at_once_and_the_mouse_arrives_afterwards() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        let started = Instant::now();
        send(&mut client, "glide 400 0 600");
        assert!(started.elapsed() < Duration::from_millis(100), "it did not wait: {:?}", started.elapsed());
        assert!(moves(&rx.try_iter().collect::<Vec<_>>()).len() <= 3, "the mouse has hardly set off");

        send(&mut client, "settle");
        let path = moves(&rx.try_iter().collect::<Vec<_>>());
        assert_eq!(path.last(), Some(&(400.0, 0.0)), "arrived, and said so");
        assert!(path.len() > 10, "a path, not a jump: {} moves", path.len());
        assert!(path.windows(2).all(|w| w[0].0 <= w[1].0), "never backwards");
        assert!(started.elapsed() >= Duration::from_millis(300), "it took time: {:?}", started.elapsed());
    }

    #[test]
    fn a_press_waits_for_the_glide_to_arrive_so_it_lands_where_it_says() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 300 0 400");
        send(&mut client, "down 300 0");
        send(&mut client, "up 300 0");
        let got: Vec<Command> = rx.try_iter().collect();
        let press = got.iter().position(|c| matches!(c, Command::PointerDown { .. })).expect("pressed");
        let before = moves(&got[..press]);
        assert_eq!(before.last(), Some(&(300.0, 0.0)), "the glide had arrived before the press: {before:?}");
        assert!(before.len() > 5, "and it got there by gliding");
        assert!(got[press + 1..].iter().all(|c| matches!(c, Command::PointerUp { .. })), "nothing glided after");
    }

    #[test]
    fn a_press_without_a_point_lands_where_the_mouse_was_left_and_waits_for_a_glide() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 120 40 300");
        send(&mut client, "down");
        send(&mut client, "up secondary");
        let got: Vec<Command> = rx.try_iter().collect();
        let press = got.iter().position(|c| matches!(c, Command::PointerDown { .. })).expect("pressed");
        assert_eq!(
            got[press],
            Command::PointerDown { x: 120.0, y: 40.0, button: PointerButton::Primary, modifiers: Default::default() },
            "at the glide's end, not where it was when the press was sent"
        );
        assert_eq!(
            got[press + 1],
            Command::PointerUp { x: 120.0, y: 40.0, button: PointerButton::Secondary, modifiers: Default::default() }
        );
    }

    #[test]
    fn a_press_without_a_point_before_the_mouse_was_ever_placed_says_so() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let err = client.send(&"down".parse().expect("parses")).expect_err("nowhere to press");
        assert!(err.to_string().contains("give a position"), "{err}");
        assert!(rx.try_recv().is_err(), "nothing reached the app");
    }

    #[test]
    fn a_new_target_mid_flight_bends_the_path_instead_of_jumping() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 900 0 900");
        std::thread::sleep(Duration::from_millis(220));
        let before = moves(&rx.try_iter().collect::<Vec<_>>());
        let at = *before.last().expect("it was under way");
        assert!(at.0 > 50.0 && at.0 < 800.0, "caught mid-flight: {at:?}");

        // Back the way it came, and down: a turn of well over a right angle.
        send(&mut client, "glide 100 500 900");
        send(&mut client, "settle");
        let after = moves(&rx.try_iter().collect::<Vec<_>>());
        assert_eq!(after.last(), Some(&(100.0, 500.0)));

        let path: Vec<(f32, f32)> = std::iter::once(at).chain(after).collect();
        let steps: Vec<(f32, f32)> = path.windows(2).map(|w| (w[1].0 - w[0].0, w[1].1 - w[0].1)).collect();
        let longest = steps.iter().map(|s| s.0.hypot(s.1)).fold(0.0, f32::max);
        assert!(longest < 110.0, "no jump: the longest step was {longest}");
        let furthest = path.iter().map(|p| p.0).fold(0.0, f32::max);
        assert!(furthest > at.0 + 10.0, "it carried on before coming round: {furthest} vs {}", at.0);

        let turned: f32 = steps
            .windows(2)
            .filter(|w| w[0].0.hypot(w[0].1) > 2.0 && w[1].0.hypot(w[1].1) > 2.0)
            .map(|w| {
                let (a, b) = (w[0].1.atan2(w[0].0), w[1].1.atan2(w[1].0));
                let d = (b - a + std::f32::consts::PI).rem_euclid(2.0 * std::f32::consts::PI) - std::f32::consts::PI;
                assert!(d.abs() < 0.7, "turned {d} rad in one step");
                d.abs()
            })
            .sum();
        assert!(turned > 1.5, "it curved through an arc: {turned} rad");
    }

    #[test]
    fn other_commands_are_not_held_up_by_a_glide() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 1200 0 900");
        let started = Instant::now();
        send(&mut client, "text hi");
        send(&mut client, "key press - enter");
        assert!(started.elapsed() < Duration::from_millis(150), "{:?}", started.elapsed());
        let got: Vec<Command> = rx.try_iter().collect();
        let text = got.iter().position(|c| matches!(c, Command::Text(_))).expect("typed");
        let arrived = moves(&got).last().copied();
        assert_ne!(arrived, Some((1200.0, 0.0)), "typed while the mouse was still on its way");
        assert!(text < got.len() - 1 || got.len() > 1);
    }

    #[test]
    fn settle_with_nothing_to_wait_for_replies_at_once() {
        let (server, _rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let started = Instant::now();
        send(&mut client, "settle");
        send(&mut client, "move 5 5");
        send(&mut client, "settle");
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn a_glide_from_nowhere_is_a_single_move() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "glide 40 50");
        send(&mut client, "settle");
        assert_eq!(moves(&rx.try_iter().collect::<Vec<_>>()), [(40.0, 50.0)], "nothing to glide from");
    }

    #[test]
    fn the_next_glide_starts_where_the_last_press_was() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "down 200 0");
        rx.try_iter().for_each(drop);
        send(&mut client, "glide 300 0 80");
        send(&mut client, "settle");
        let path = moves(&rx.try_iter().collect::<Vec<_>>());
        assert!(path.first().is_some_and(|p| p.0 > 200.0 && p.0 < 300.0), "began at 200: {path:?}");
    }

    #[test]
    fn leaving_the_window_forgets_where_the_pointer_was() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "leave");
        rx.try_iter().for_each(drop);
        send(&mut client, "glide 500 0 100");
        send(&mut client, "settle");
        assert_eq!(moves(&rx.try_iter().collect::<Vec<_>>()), [(500.0, 0.0)], "nowhere to glide from");
    }

    #[test]
    fn a_glide_stops_when_the_app_stops_taking_input() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let server = Server::start_with(Config::new(), move |_| counted.fetch_add(1, Ordering::SeqCst) < 4).expect("start");
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 500 0 800");
        send(&mut client, "settle");
        std::thread::sleep(Duration::from_millis(100));
        assert!(calls.load(Ordering::SeqCst) < 12, "it gave up instead of walking the whole path");
        // The app is gone: the next thing sent says so.
        let err = client.send(&"text hi".parse().expect("command")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn dropping_the_server_mid_glide_ends_it_promptly() {
        let (server, _rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        send(&mut client, "move 0 0");
        send(&mut client, "glide 2000 0 3000");
        let started = Instant::now();
        drop(client);
        drop(server);
        assert!(started.elapsed() < Duration::from_secs(1), "{:?}", started.elapsed());
    }

    #[test]
    fn a_server_listens_on_loopback_unless_told_otherwise() {
        let (server, _rx) = collecting(Config::new());
        assert!(server.endpoint().addr().ip().is_loopback());
    }

    #[test]
    fn a_server_can_listen_beyond_loopback_when_asked_and_still_stops() {
        let (server, rx) = collecting(Config::new().bind(Ipv4Addr::UNSPECIFIED.into()));
        assert!(server.endpoint().addr().ip().is_unspecified());

        // 0.0.0.0 is where it listens, not where a driver connects.
        let reachable = server.endpoint().with_ip(Ipv4Addr::LOCALHOST.into());
        let mut client = Client::connect(&reachable).expect("connect");
        client.send(&Command::Quit).expect("accepted");
        assert!(matches!(rx.try_iter().next(), Some(Command::Quit)));

        drop(client);
        drop(server); // joins the accept thread: it must have been woken
        assert!(TcpStream::connect_timeout(&reachable.addr(), Duration::from_millis(500)).is_err());
    }

    fn vars<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v).to_owned())
    }

    #[test]
    fn the_environment_turns_the_server_on_only_when_asked() {
        assert_eq!(Config::from_vars(vars(&[])).expect("valid"), None);
        assert_eq!(Config::from_vars(vars(&[("AIMER_PRENDRE", "0")])).expect("valid"), None);
        assert_eq!(Config::from_vars(vars(&[("AIMER_PRENDRE", "")])).expect("valid"), None);
        // Settings without the switch do nothing: a stray token must not start a server.
        assert_eq!(Config::from_vars(vars(&[("AIMER_PRENDRE_PORT", "9")])).expect("valid"), None);
        assert_eq!(
            Config::from_vars(vars(&[("AIMER_PRENDRE", "1")])).expect("valid"),
            Some(Config::new())
        );
    }

    #[test]
    fn the_environment_sets_bind_port_and_token() {
        let config = Config::from_vars(vars(&[
            ("AIMER_PRENDRE", "1"),
            ("AIMER_PRENDRE_BIND", "::"),
            ("AIMER_PRENDRE_PORT", "9000"),
            ("AIMER_PRENDRE_TOKEN", "cli-made"),
        ]))
        .expect("valid")
        .expect("enabled");
        assert_eq!(config, Config::new().bind("::".parse().expect("ip")).port(9000).token("cli-made"));
    }

    #[test]
    fn a_setting_that_cannot_be_read_is_an_error_not_a_silent_default() {
        for (name, value) in [
            ("AIMER_PRENDRE_BIND", "everywhere"),
            ("AIMER_PRENDRE_PORT", "70000"),
            ("AIMER_PRENDRE_PORT", "http"),
        ] {
            let err = Config::from_vars(vars(&[("AIMER_PRENDRE", "1"), (name, value)])).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{name}={value}");
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    #[test]
    fn an_announced_endpoint_is_the_one_the_server_has() {
        let (server, _rx) = collecting(Config::new());
        let line = server.endpoint().announcement();
        assert_eq!(Endpoint::from_announcement(&line).as_ref(), Some(server.endpoint()));
    }

    #[test]
    fn an_authorised_client_delivers_commands_in_order() {
        let (server, rx) = collecting(Config::new());
        let mut client = Client::connect(server.endpoint()).expect("connect");
        let click = [
            Command::PointerMove { x: 5.0, y: 6.0, modifiers: Default::default() },
            Command::PointerDown { x: 5.0, y: 6.0, button: PointerButton::Primary, modifiers: Default::default() },
            Command::PointerUp { x: 5.0, y: 6.0, button: PointerButton::Primary, modifiers: Default::default() },
            Command::Touch { phase: crate::model::TouchPhase::Started, id: 2, x: 1.0, y: 2.0 },
            Command::FileDrop("/tmp/a b.png".into()),
            Command::Quit,
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
