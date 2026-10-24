//! Development-only app listener for authenticated Aimer module reloads.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io;
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::time::{Duration, Instant};

use aimer_reload_protocol::{
    ProtocolError, ProtocolLimits, ReloadCommand, ReloadConnectionOutcome, ReloadResult,
    ReloadStage, SessionCredentials, TransferAcknowledgement, receive_module_and_acknowledge,
    receive_reload_connection,
};

pub use aimer_reload_protocol::{ModuleMetadata, query_reload_result, send_reload_command};

const REQUEST_ID_CONFLICT: u32 = 1;

/// Receives complete authenticated modules outside the live widget tree.
pub trait ModuleSink: Send + Sync + 'static {
    /// Accepts one bounded module after protocol authentication and validation.
    fn accept(&self, module: Vec<u8>) -> Result<(), String>;
}

impl<F> ModuleSink for F
where
    F: Fn(Vec<u8>) -> Result<(), String> + Send + Sync + 'static,
{
    fn accept(&self, module: Vec<u8>) -> Result<(), String> {
        self(module)
    }
}

/// Failure while accepting a reload connection or dispatching its module.
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    #[error("reload listener I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Protocol(#[from] ProtocolError),
    #[error("invalid reload listener configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("reload listener session credentials expired")]
    SessionExpired,
    #[error("reload listener temporarily rejected authentication attempts")]
    RateLimited,
}

/// Explicit expiry and failed-authentication policy for an app listener.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListenerSecurity {
    session_lifetime: Duration,
    max_auth_failures: u32,
    failure_window: Duration,
}

impl ListenerSecurity {
    /// Creates a security policy without process-global timers or counters.
    #[inline]
    pub const fn new(
        session_lifetime: Duration,
        max_auth_failures: u32,
        failure_window: Duration,
    ) -> Self {
        Self {
            session_lifetime,
            max_auth_failures,
            failure_window,
        }
    }
}

/// A debug-app TCP listener that accepts one authenticated controlling client.
pub struct ReloadListener<S> {
    listener: TcpListener,
    credentials: SessionCredentials,
    limits: ProtocolLimits,
    sink: S,
}

impl<S> ReloadListener<S>
where
    S: ModuleSink,
{
    /// Binds a development listener to the target adapter's selected address.
    pub fn bind(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        limits: ProtocolLimits,
        sink: S,
    ) -> Result<Self, ListenerError> {
        Ok(Self {
            listener: TcpListener::bind(address)?,
            credentials,
            limits,
            sink,
        })
    }

    /// Returns the concrete bound address, including an OS-selected port.
    #[inline]
    pub fn local_addr(&self) -> Result<SocketAddr, ListenerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Accepts, authenticates, and dispatches one module upload.
    pub fn accept_once(&self) -> Result<TransferAcknowledgement, ListenerError> {
        let (mut stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(self.limits.io_timeout()))?;
        stream.set_write_timeout(Some(self.limits.io_timeout()))?;
        Ok(receive_module_and_acknowledge(
            &mut stream,
            &self.credentials,
            self.limits,
            |module| self.sink.accept(module),
        )?)
    }
}

/// Executes complete authenticated commands outside the widget/event thread.
pub trait ReloadCommandSink: Send + Sync + 'static {
    /// Returns the authoritative safe-point result for one command.
    fn execute(&self, command: ReloadCommand) -> ReloadResult;
}

impl<F> ReloadCommandSink for F
where
    F: Fn(ReloadCommand) -> ReloadResult + Send + Sync + 'static,
{
    fn execute(&self, command: ReloadCommand) -> ReloadResult {
        self(command)
    }
}

/// A development listener that reports terminal replacement outcomes.
pub struct ReloadCommandListener<S> {
    listener: TcpListener,
    credentials: SessionCredentials,
    limits: ProtocolLimits,
    sink: S,
    results: RefCell<ResultLedger>,
    security: Option<SecurityState>,
}

struct SecurityState {
    policy: ListenerSecurity,
    created_at: Instant,
    failures: RefCell<AuthenticationFailures>,
}

struct AuthenticationFailures {
    window_started_at: Option<Instant>,
    count: u32,
}

struct ResultLedger {
    capacity: usize,
    entries: VecDeque<ResultEntry>,
}

struct ResultEntry {
    request_id: u64,
    module_digest: [u8; 32],
    result: ReloadResult,
}

impl<S> ReloadCommandListener<S>
where
    S: ReloadCommandSink,
{
    /// Binds a command listener with explicit credential expiry and throttling.
    pub fn bind(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        limits: ProtocolLimits,
        security: ListenerSecurity,
        sink: S,
    ) -> Result<Self, ListenerError> {
        Self::bind_secure(address, credentials, limits, security, sink)
    }

    /// Binds a command listener with explicit credential expiry and throttling.
    pub fn bind_secure(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        limits: ProtocolLimits,
        security: ListenerSecurity,
        sink: S,
    ) -> Result<Self, ListenerError> {
        if security.session_lifetime.is_zero()
            || security.failure_window.is_zero()
            || security.max_auth_failures == 0
        {
            return Err(ListenerError::InvalidConfiguration(
                "security durations and authentication failure limit must be nonzero",
            ));
        }
        Self::bind_inner(address, credentials, limits, security, sink)
    }

    fn bind_inner(
        address: impl ToSocketAddrs,
        credentials: SessionCredentials,
        limits: ProtocolLimits,
        security: ListenerSecurity,
        sink: S,
    ) -> Result<Self, ListenerError> {
        if limits.terminal_result_limit() == 0 {
            return Err(ListenerError::InvalidConfiguration(
                "terminal result capacity must be nonzero",
            ));
        }
        Ok(Self {
            listener: TcpListener::bind(address)?,
            credentials,
            limits,
            sink,
            results: RefCell::new(ResultLedger {
                capacity: limits.terminal_result_limit(),
                entries: VecDeque::with_capacity(limits.terminal_result_limit()),
            }),
            security: Some(SecurityState {
                policy: security,
                created_at: Instant::now(),
                failures: RefCell::new(AuthenticationFailures {
                    window_started_at: None,
                    count: 0,
                }),
            }),
        })
    }

    /// Returns the concrete bound address, including an OS-selected port.
    #[inline]
    pub fn local_addr(&self) -> Result<SocketAddr, ListenerError> {
        Ok(self.listener.local_addr()?)
    }

    /// Authenticates and executes one complete command connection.
    pub fn accept_once(&self) -> Result<ReloadResult, ListenerError> {
        match self.accept_connection()? {
            ReloadConnectionOutcome::Command(result) => Ok(result),
            ReloadConnectionOutcome::Query(_) => Err(ListenerError::Protocol(
                ProtocolError::InvalidFrame("expected a reload command"),
            )),
        }
    }

    /// Serves one authenticated command or reconnect result query.
    pub fn accept_connection(&self) -> Result<ReloadConnectionOutcome, ListenerError> {
        let (mut stream, _) = self.listener.accept()?;
        stream.set_read_timeout(Some(self.limits.io_timeout()))?;
        stream.set_write_timeout(Some(self.limits.io_timeout()))?;
        self.check_security_policy()?;
        let result = receive_reload_connection(
            &mut stream,
            &self.credentials,
            self.limits,
            |command| self.execute_once(command),
            |request_id| self.lookup_result(request_id),
        );
        match result {
            Ok(outcome) => {
                self.clear_authentication_failures();
                Ok(outcome)
            }
            Err(error) => {
                if matches!(error, ProtocolError::Authentication) {
                    self.record_authentication_failure();
                }
                Err(ListenerError::Protocol(error))
            }
        }
    }

    fn check_security_policy(&self) -> Result<(), ListenerError> {
        let Some(security) = &self.security else {
            return Ok(());
        };
        let now = Instant::now();
        if now.duration_since(security.created_at) >= security.policy.session_lifetime {
            return Err(ListenerError::SessionExpired);
        }
        let mut failures = security.failures.borrow_mut();
        if let Some(started_at) = failures.window_started_at
            && now.duration_since(started_at) >= security.policy.failure_window
        {
            failures.window_started_at = None;
            failures.count = 0;
        }
        if failures.count >= security.policy.max_auth_failures {
            return Err(ListenerError::RateLimited);
        }
        Ok(())
    }

    fn record_authentication_failure(&self) {
        let Some(security) = &self.security else {
            return;
        };
        let now = Instant::now();
        let mut failures = security.failures.borrow_mut();
        match failures.window_started_at {
            Some(started_at) if now.duration_since(started_at) < security.policy.failure_window => {
            }
            _ => {
                failures.window_started_at = Some(now);
                failures.count = 0;
            }
        }
        failures.count = failures.count.saturating_add(1);
    }

    fn clear_authentication_failures(&self) {
        let Some(security) = &self.security else {
            return;
        };
        let mut failures = security.failures.borrow_mut();
        failures.window_started_at = None;
        failures.count = 0;
    }

    fn execute_once(&self, command: ReloadCommand) -> ReloadResult {
        let existing = self
            .results
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.request_id == command.request_id())
            .map(|entry| (entry.module_digest, entry.result.clone()));
        if let Some((module_digest, result)) = existing {
            if module_digest == command.module_digest() {
                return result;
            }
            return ReloadResult::Rejected {
                stage: ReloadStage::Preflight,
                error_code: REQUEST_ID_CONFLICT,
                active_generation: active_generation(&result),
                diagnostic: "request ID was already used for another module digest".to_owned(),
            };
        }
        let request_id = command.request_id();
        let module_digest = command.module_digest();
        let result = self.sink.execute(command);
        let mut ledger = self.results.borrow_mut();
        if ledger.entries.len() == ledger.capacity {
            ledger.entries.pop_front();
        }
        ledger.entries.push_back(ResultEntry {
            request_id,
            module_digest,
            result: result.clone(),
        });
        result
    }

    fn lookup_result(&self, request_id: u64) -> Option<ReloadResult> {
        self.results
            .borrow()
            .entries
            .iter()
            .find(|entry| entry.request_id == request_id)
            .map(|entry| entry.result.clone())
    }
}

fn active_generation(result: &ReloadResult) -> u64 {
    match result {
        ReloadResult::Committed {
            active_generation, ..
        }
        | ReloadResult::Rejected {
            active_generation, ..
        }
        | ReloadResult::Cancelled { active_generation } => *active_generation,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, mpsc};
    use std::thread;
    use std::time::Duration;

    use aimer_reload_protocol::send_module;

    use super::*;

    #[test]
    fn authenticated_complete_command_returns_the_safe_point_terminal_result() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1)).max_chunk_bytes(3);
        let (received_tx, received_rx) = mpsc::channel();
        let expected = ReloadResult::Committed {
            active_generation: 12,
            reset_state_entries: 1,
            cleanup_warnings: 0,
        };
        let sink_result = expected.clone();
        let listener = ReloadCommandListener::bind(
            "127.0.0.1:0",
            credentials.clone(),
            limits,
            ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
            move |command: ReloadCommand| {
                received_tx.send(command).unwrap();
                sink_result.clone()
            },
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once().unwrap());
        let metadata = ModuleMetadata::new([0x21; 16], [0x31; 16], 1, 0, [0x41; 32]);
        let mut stream = TcpStream::connect(address).unwrap();

        let result = send_reload_command(
            &mut stream,
            &credentials,
            limits,
            73,
            metadata,
            b"\0asm\x01\0\0\0",
        )
        .unwrap();
        let server_result = server.join().unwrap();

        assert_eq!(result, expected);
        assert_eq!(server_result, expected);
        let command = received_rx.recv().unwrap();
        assert_eq!(command.request_id(), 73);
        assert_eq!(command.metadata(), metadata);
        assert_eq!(command.module(), b"\0asm\x01\0\0\0");
    }

    #[test]
    fn repeated_command_and_reconnect_query_recover_one_recorded_result_without_reexecution() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1))
            .max_chunk_bytes(3)
            .max_terminal_results(2);
        let executions = Arc::new(AtomicU32::new(0));
        let sink_executions = Arc::clone(&executions);
        let expected = ReloadResult::Committed {
            active_generation: 12,
            reset_state_entries: 0,
            cleanup_warnings: 0,
        };
        let sink_result = expected.clone();
        let listener = ReloadCommandListener::bind(
            "127.0.0.1:0",
            credentials.clone(),
            limits,
            ListenerSecurity::new(Duration::from_secs(60), 4, Duration::from_secs(1)),
            move |_command: ReloadCommand| {
                sink_executions.fetch_add(1, Ordering::Relaxed);
                sink_result.clone()
            },
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            listener.accept_connection().unwrap();
            listener.accept_connection().unwrap();
            listener.accept_connection().unwrap();
        });
        let metadata = ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]);
        let module = b"\0asm\x01\0\0\0";

        for _ in 0..2 {
            let mut stream = TcpStream::connect(address).unwrap();
            assert_eq!(
                send_reload_command(&mut stream, &credentials, limits, 73, metadata, module,)
                    .unwrap(),
                expected
            );
        }
        let mut stream = TcpStream::connect(address).unwrap();
        assert_eq!(
            query_reload_result(&mut stream, &credentials, limits, 73).unwrap(),
            Some(expected)
        );
        server.join().unwrap();

        assert_eq!(executions.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn authentication_failures_are_rate_limited_before_a_later_sink_entry() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let wrong_credentials = SessionCredentials::from_parts([0x11; 16], [0x5A; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let security = ListenerSecurity::new(Duration::from_secs(60), 1, Duration::from_secs(60));
        let executions = Arc::new(AtomicU32::new(0));
        let sink_executions = Arc::clone(&executions);
        let listener = ReloadCommandListener::bind_secure(
            "127.0.0.1:0",
            credentials.clone(),
            limits,
            security,
            move |_command: ReloadCommand| {
                sink_executions.fetch_add(1, Ordering::Relaxed);
                ReloadResult::Committed {
                    active_generation: 2,
                    reset_state_entries: 0,
                    cleanup_warnings: 0,
                }
            },
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let first = listener.accept_connection();
            let second = listener.accept_connection();
            (first, second)
        });
        let metadata = ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]);

        let mut stream = TcpStream::connect(address).unwrap();
        let _ = send_reload_command(&mut stream, &wrong_credentials, limits, 1, metadata, b"bad");
        let mut stream = TcpStream::connect(address).unwrap();
        let _ = send_reload_command(
            &mut stream,
            &credentials,
            limits,
            2,
            metadata,
            b"still blocked",
        );
        let (first, second) = server.join().unwrap();

        assert!(matches!(
            first,
            Err(ListenerError::Protocol(ProtocolError::Authentication))
        ));
        assert!(matches!(second, Err(ListenerError::RateLimited)));
        assert_eq!(executions.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn expired_session_is_rejected_before_authentication_or_sink_entry() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let listener = ReloadCommandListener::bind_secure(
            "127.0.0.1:0",
            credentials,
            limits,
            ListenerSecurity::new(Duration::from_millis(1), 1, Duration::from_secs(1)),
            |_command: ReloadCommand| unreachable!("expired session reached command sink"),
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        thread::sleep(Duration::from_millis(10));
        let client = TcpStream::connect(address).unwrap();

        let result = listener.accept_connection();
        drop(client);

        assert!(matches!(result, Err(ListenerError::SessionExpired)));
    }

    struct RecordingStream {
        stream: TcpStream,
        written: Vec<u8>,
    }

    impl Read for RecordingStream {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.stream.read(buffer)
        }
    }

    impl Write for RecordingStream {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let written = self.stream.write(buffer)?;
            self.written.extend_from_slice(&buffer[..written]);
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.stream.flush()
        }
    }

    #[test]
    fn authenticated_client_transfers_a_wasm_module_and_receives_acknowledgement() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let (received_tx, received_rx) = mpsc::channel();
        let listener =
            ReloadListener::bind("127.0.0.1:0", credentials.clone(), limits, move |module| {
                received_tx.send(module).unwrap();
                Ok(())
            })
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once().unwrap());
        let module = b"\0asm\x01\0\0\0";
        let mut stream = TcpStream::connect(address).unwrap();

        let acknowledgement = send_module(&mut stream, &credentials, limits, 41, module).unwrap();
        let server_acknowledgement = server.join().unwrap();

        assert_eq!(acknowledgement, server_acknowledgement);
        assert_eq!(acknowledgement.request_id, 41);
        assert_eq!(acknowledgement.module_len, module.len());
        assert_eq!(received_rx.recv().unwrap(), module);
    }

    #[test]
    fn client_with_the_wrong_token_cannot_reach_the_module_sink() {
        let server_credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let client_credentials = SessionCredentials::from_parts([0x11; 16], [0x5A; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let (received_tx, received_rx) = mpsc::channel();
        let listener =
            ReloadListener::bind("127.0.0.1:0", server_credentials, limits, move |module| {
                received_tx.send(module).unwrap();
                Ok(())
            })
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once());
        let mut stream = TcpStream::connect(address).unwrap();

        let client_error = send_module(
            &mut stream,
            &client_credentials,
            limits,
            41,
            b"\0asm\x01\0\0\0",
        )
        .unwrap_err();
        let server_error = server.join().unwrap().unwrap_err();

        assert!(matches!(
            server_error,
            ListenerError::Protocol(ProtocolError::Authentication)
        ));
        assert!(matches!(
            client_error,
            ProtocolError::Authentication | ProtocolError::Io(_)
        ));
        assert!(received_rx.try_recv().is_err());
    }

    #[test]
    fn interrupted_connection_never_reaches_the_module_sink() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let (received_tx, received_rx) = mpsc::channel();
        let listener = ReloadListener::bind("127.0.0.1:0", credentials, limits, move |module| {
            received_tx.send(module).unwrap();
            Ok(())
        })
        .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once());

        drop(TcpStream::connect(address).unwrap());
        let error = server.join().unwrap().unwrap_err();

        assert!(matches!(
            error,
            ListenerError::Protocol(ProtocolError::Io(_))
        ));
        assert!(received_rx.try_recv().is_err());
    }

    #[test]
    fn module_larger_than_the_server_limit_never_reaches_the_sink() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let server_limits = ProtocolLimits::new(4, Duration::from_secs(1));
        let client_limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let (received_tx, received_rx) = mpsc::channel();
        let listener = ReloadListener::bind(
            "127.0.0.1:0",
            credentials.clone(),
            server_limits,
            move |module| {
                received_tx.send(module).unwrap();
                Ok(())
            },
        )
        .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once());
        let mut stream = TcpStream::connect(address).unwrap();

        let _ = send_module(
            &mut stream,
            &credentials,
            client_limits,
            42,
            b"\0asm\x01\0\0\0",
        );
        let server_error = server.join().unwrap().unwrap_err();

        assert!(matches!(
            server_error,
            ListenerError::Protocol(ProtocolError::ModuleTooLarge {
                actual: 8,
                maximum: 4
            })
        ));
        assert!(received_rx.try_recv().is_err());
    }

    #[test]
    fn listener_accepts_a_fresh_authenticated_connection_after_rejection() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let wrong_credentials = SessionCredentials::from_parts([0x11; 16], [0x5A; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let (received_tx, received_rx) = mpsc::channel();
        let listener =
            ReloadListener::bind("127.0.0.1:0", credentials.clone(), limits, move |module| {
                received_tx.send(module).unwrap();
                Ok(())
            })
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || {
            let first = listener.accept_once();
            let second = listener.accept_once();
            (first, second)
        });

        let mut rejected_stream = TcpStream::connect(address).unwrap();
        let _ = send_module(
            &mut rejected_stream,
            &wrong_credentials,
            limits,
            1,
            b"rejected",
        );
        let mut accepted_stream = TcpStream::connect(address).unwrap();
        let acknowledgement = send_module(
            &mut accepted_stream,
            &credentials,
            limits,
            2,
            b"\0asm\x01\0\0\0",
        )
        .unwrap();
        let (first, second) = server.join().unwrap();

        assert!(first.is_err());
        assert_eq!(second.unwrap(), acknowledgement);
        assert_eq!(received_rx.recv().unwrap(), b"\0asm\x01\0\0\0");
    }

    #[test]
    fn authenticated_frames_do_not_expose_module_bytes_to_the_transport() {
        let credentials = SessionCredentials::from_parts([0x11; 16], [0xA5; 32]);
        let limits = ProtocolLimits::new(1024, Duration::from_secs(1));
        let listener =
            ReloadListener::bind("127.0.0.1:0", credentials.clone(), limits, |_| Ok(())).unwrap();
        let address = listener.local_addr().unwrap();
        let server = thread::spawn(move || listener.accept_once().unwrap());
        let module = b"AIMER-CONFIDENTIAL-WASM-MODULE";
        let stream = TcpStream::connect(address).unwrap();
        stream.set_read_timeout(Some(limits.io_timeout())).unwrap();
        stream.set_write_timeout(Some(limits.io_timeout())).unwrap();
        let mut recording = RecordingStream {
            stream,
            written: Vec::new(),
        };

        send_module(&mut recording, &credentials, limits, 99, module).unwrap();
        server.join().unwrap();

        assert!(
            !recording
                .written
                .windows(module.len())
                .any(|window| window == module)
        );
    }
}
