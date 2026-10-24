//! A deliberately small HTTP/1.1 front for [`Server`].
//!
//! One thread per connection, one request per connection, `Connection: close`.
//! That is plenty for a test relying party and keeps the example free of an
//! HTTP stack; it is not a design to serve traffic with.

use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::app::{Request, Response, Server};

/// Largest header block accepted.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest body accepted. An attestation object is a couple of kilobytes.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// How long a client may take to send its request.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Accepts connections forever, serving each on its own thread.
pub fn serve(listener: TcpListener, server: Arc<Server>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let server = Arc::clone(&server);
        std::thread::spawn(move || {
            // A client that hangs up mid-request is not the server's problem.
            let _ = handle_connection(stream, &server);
        });
    }
}

fn handle_connection(mut stream: TcpStream, server: &Server) -> io::Result<()> {
    stream.set_read_timeout(Some(READ_TIMEOUT))?;
    let response = match read_request(&mut stream) {
        Ok(request) => {
            let response = server.handle(&request);
            println!("{} {} -> {}", request.method, request.path, response.status);
            response
        }
        Err(ReadError::Io(error))
            if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
        {
            Response {
                status: 408,
                body: br#"{"error":"request timed out"}"#.to_vec(),
            }
        }
        Err(ReadError::Io(error)) => return Err(error),
        Err(ReadError::Bad(status, message)) => Response {
            status,
            body: format!(r#"{{"error":"{message}"}}"#).into_bytes(),
        },
    };
    write_response(&mut stream, &response)
}

enum ReadError {
    Io(io::Error),
    Bad(u16, &'static str),
}

impl From<io::Error> for ReadError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

fn read_request(stream: &mut impl Read) -> Result<Request, ReadError> {
    let mut buffer = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(end) = find(&buffer, b"\r\n\r\n") {
            break end;
        }
        if buffer.len() > MAX_HEAD_BYTES {
            return Err(ReadError::Bad(431, "headers too large"));
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(ReadError::Bad(400, "incomplete request"));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };

    let head = std::str::from_utf8(&buffer[..head_end])
        .map_err(|_| ReadError::Bad(400, "headers are not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let (Some(method), Some(target), Some(version)) =
        (request_line.next(), request_line.next(), request_line.next())
    else {
        return Err(ReadError::Bad(400, "malformed request line"));
    };
    // Drop any query string; no endpoint takes one.
    let (method, path) = (
        method.to_owned(),
        target.split('?').next().unwrap_or(target).to_owned(),
    );
    if !version.starts_with("HTTP/1.") {
        return Err(ReadError::Bad(505, "only HTTP/1.x is supported"));
    }
    let mut content_length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            return Err(ReadError::Bad(400, "malformed header"));
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse()
                .map_err(|_| ReadError::Bad(400, "bad content-length"))?;
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            return Err(ReadError::Bad(501, "chunked bodies are not supported"));
        }
    }
    if content_length > MAX_BODY_BYTES {
        return Err(ReadError::Bad(413, "body too large"));
    }

    // Bytes already read past the blank line belong to the body.
    let mut body = buffer.split_off(head_end + 4);
    while body.len() < content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(ReadError::Bad(400, "incomplete body"));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    body.truncate(content_length);

    Ok(Request { method, path, body })
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        408 => "Request Timeout",
        405 => "Method Not Allowed",
        409 => "Conflict",
        413 => "Content Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Unknown",
    }
}

fn write_response(stream: &mut impl Write, response: &Response) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        response.status,
        reason(response.status),
        response.body.len()
    )?;
    stream.write_all(&response.body)?;
    stream.flush()
}
