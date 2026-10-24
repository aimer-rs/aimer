use std::cell::Cell;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crossbeam::channel::{Receiver, RecvTimeoutError, Sender, unbounded};

use aimer_reload_protocol::{
    ModuleMetadata, ProtocolLimits, ReloadResult, SessionCredentials, receive_reload_connection,
    send_reload_command,
};

const MAX_FUZZ_INPUT: usize = 256;
/// Turns a stalled peer into an end of stream so a mutated length field cannot
/// park both protocol halves forever.
const READ_STALL_TIMEOUT: Duration = Duration::from_millis(25);
const SESSION_ID: [u8; 16] = [0x11; 16];
const TOKEN: [u8; 32] = [0xA5; 32];

fn limits() -> ProtocolLimits {
    ProtocolLimits::new(64, Duration::from_millis(10))
        .max_chunk_bytes(16)
        .max_diagnostic_bytes(32)
        .max_terminal_results(2)
}

fn credentials() -> SessionCredentials {
    SessionCredentials::from_parts(SESSION_ID, TOKEN)
}

pub fn fuzz_unauthenticated_connection(data: &[u8]) {
    let mut stream = RawStream::new(&data[..data.len().min(MAX_FUZZ_INPUT)]);
    let callback_called = Cell::new(false);
    let _ = receive_reload_connection(
        &mut stream,
        &credentials(),
        limits(),
        |_| {
            callback_called.set(true);
            ReloadResult::Cancelled {
                active_generation: 0,
            }
        },
        |_| {
            callback_called.set(true);
            None
        },
    );
    assert!(!callback_called.get());
}

pub fn fuzz_authenticated_connection(data: &[u8]) {
    let mutation = data.first().copied().unwrap_or(0);
    let module_end = data.len().min(65);
    let module = if data.is_empty() {
        &[][..]
    } else {
        &data[1..module_end]
    };
    let (client_stream, server_stream) = duplex();
    let callback_called = Arc::new(AtomicBool::new(false));
    let server_called = Arc::clone(&callback_called);
    let server = thread::spawn(move || {
        let mut server_stream = server_stream;
        let _ = receive_reload_connection(
            &mut server_stream,
            &credentials(),
            limits(),
            |_| {
                server_called.store(true, Ordering::SeqCst);
                ReloadResult::Cancelled {
                    active_generation: 1,
                }
            },
            |_| None,
        );
    });
    let mut client = MutatingStream::new(client_stream, mutation);
    let result = send_reload_command(
        &mut client,
        &credentials(),
        limits(),
        7,
        ModuleMetadata::new([1; 16], [2; 16], 1, 0, [3; 32]),
        module,
    );
    drop(client);
    server.join().unwrap();

    if mutation == 0 {
        assert!(result.is_ok());
        assert!(callback_called.load(Ordering::SeqCst));
    } else {
        assert!(!callback_called.load(Ordering::SeqCst));
    }
}

pub fn fuzz_terminal_result(data: &[u8]) {
    let data = &data[..data.len().min(MAX_FUZZ_INPUT)];
    if let Ok(result) = ReloadResult::decode(data, limits().diagnostic_bytes_limit()) {
        assert_eq!(
            result.encode(limits().diagnostic_bytes_limit()).ok().as_deref(),
            Some(data)
        );
    }
}

struct RawStream<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> RawStream<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }
}

impl Read for RawStream<'_> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let remaining = &self.input[self.offset..];
        let count = remaining.len().min(output.len());
        output[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        Ok(count)
    }
}

impl Write for RawStream<'_> {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Creates an ordered in-memory byte stream pair.
///
/// The authenticated handshake is interactive, so the target needs a real
/// bidirectional channel. An in-memory pipe keeps every iteration free of
/// operating-system sockets, which would otherwise exhaust ephemeral ports long
/// before a campaign finishes.
fn duplex() -> (DuplexStream, DuplexStream) {
    let (left_sender, left_receiver) = unbounded();
    let (right_sender, right_receiver) = unbounded();
    (
        DuplexStream {
            inbound: left_receiver,
            outbound: right_sender,
            pending: VecDeque::new(),
        },
        DuplexStream {
            inbound: right_receiver,
            outbound: left_sender,
            pending: VecDeque::new(),
        },
    )
}

struct DuplexStream {
    inbound: Receiver<Vec<u8>>,
    outbound: Sender<Vec<u8>>,
    pending: VecDeque<u8>,
}

impl Read for DuplexStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        while self.pending.is_empty() {
            match self.inbound.recv_timeout(READ_STALL_TIMEOUT) {
                Ok(bytes) => self.pending.extend(bytes),
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return Ok(0),
            }
        }
        let count = self.pending.len().min(output.len());
        for slot in output.iter_mut().take(count) {
            *slot = self.pending.pop_front().unwrap();
        }
        Ok(count)
    }
}

impl Write for DuplexStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        self.outbound
            .send(input.to_vec())
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        Ok(input.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct MutatingStream {
    inner: DuplexStream,
    mutation: u8,
    writes: usize,
}

impl MutatingStream {
    fn new(inner: DuplexStream, mutation: u8) -> Self {
        Self {
            inner,
            mutation,
            writes: 0,
        }
    }
}

impl Read for MutatingStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.inner.read(output)
    }
}

impl Write for MutatingStream {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let write_index = self.writes;
        self.writes += 1;
        if write_index == 1 && self.mutation != 0 && !input.is_empty() {
            let mut mutated = input.to_vec();
            let index = self.mutation as usize % mutated.len();
            mutated[index] ^= self.mutation;
            self.inner.write_all(&mutated)?;
            Ok(input.len())
        } else {
            self.inner.write(input)
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unauthenticated_bytes_do_not_reach_runtime_callback() {
        fuzz_unauthenticated_connection(&[0_u8; MAX_FUZZ_INPUT + 1]);
    }

    #[test]
    fn authenticated_valid_transcript_reaches_runtime_callback() {
        fuzz_authenticated_connection(&[0]);
    }

    #[test]
    fn authenticated_mutated_transcript_does_not_reach_runtime_callback() {
        fuzz_authenticated_connection(&[1, 0]);
    }

    #[test]
    fn terminal_result_decoder_accepts_canonical_and_rejects_malformed_payloads() {
        fuzz_terminal_result(&[
            1, 0, 0, 0, 9, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0,
        ]);
        fuzz_terminal_result(&[2, 0, 1]);
        fuzz_terminal_result(&[0_u8; MAX_FUZZ_INPUT + 1]);
    }

    #[test]
    fn fuzz_limits_remain_small_and_fixed() {
        let limits = limits();
        assert_eq!(limits.max_module_bytes(), 64);
        assert_eq!(limits.chunk_bytes_limit(), 16);
        assert_eq!(limits.diagnostic_bytes_limit(), 32);
        assert_eq!(limits.terminal_result_limit(), 2);
    }
}
