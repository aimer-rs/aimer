//! Linux: HTTP for remote audio.
//!
//! [`fetch`] downloads a whole file. [`Download`] keeps downloading on a
//! thread of its own while a decoder reads what has arrived through a
//! [`DownloadReader`], which blocks only when it runs ahead of the network.
//! Everything downloaded is kept, so seeking back never downloads again.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use symphonia::core::io::MediaSource;

use crate::AudioError;

/// Bytes read from the network per wake-up of a waiting reader.
const CHUNK: usize = 64 * 1024;

/// Downloads `url` completely.
pub(crate) fn fetch(url: &str) -> Result<Vec<u8>, AudioError> {
    let response = ureq::get(url)
        .call()
        .map_err(|error| AudioError::Network(error.to_string()))?;
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut bytes)
        .map_err(|error| AudioError::Network(error.to_string()))?;
    Ok(bytes)
}

#[derive(Default)]
struct State {
    bytes: Vec<u8>,
    /// From `Content-Length`, when the server sent it.
    total: Option<u64>,
    done: bool,
    error: Option<String>,
    /// Every reader is gone; the download thread stops.
    cancelled: bool,
}

/// A download in progress, shared by its thread and its readers.
pub(crate) struct Download {
    state: Mutex<State>,
    grew: Condvar,
}

impl Download {
    /// Starts downloading `url` on a helper thread.
    pub fn start(url: String) -> Arc<Self> {
        let download = Arc::new(Self {
            state: Mutex::new(State::default()),
            grew: Condvar::new(),
        });
        let shared = download.clone();
        let spawned = thread::Builder::new()
            .name("aimer-audio-download".into())
            .spawn(move || shared.run(&url));
        if let Err(error) = spawned {
            download.finish(Err(error.to_string()));
        }
        download
    }

    /// A download that already has every byte; for tests and data in memory.
    #[cfg(test)]
    pub fn complete(bytes: Vec<u8>) -> Arc<Self> {
        let total = Some(bytes.len() as u64);
        Arc::new(Self {
            state: Mutex::new(State {
                bytes,
                total,
                done: true,
                ..State::default()
            }),
            grew: Condvar::new(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run(&self, url: &str) {
        let response = match ureq::get(url).call() {
            Ok(response) => response,
            Err(error) => return self.finish(Err(error.to_string())),
        };
        let total = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse().ok());
        {
            let mut state = self.lock();
            state.total = total;
            if let Some(total) = total {
                state.bytes.reserve(total as usize);
            }
        }
        let mut reader = response.into_body().into_reader();
        let mut chunk = vec![0; CHUNK];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => return self.finish(Ok(())),
                Ok(read) => {
                    let mut state = self.lock();
                    if state.cancelled {
                        return;
                    }
                    state.bytes.extend_from_slice(&chunk[..read]);
                    drop(state);
                    self.grew.notify_all();
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return self.finish(Err(error.to_string())),
            }
        }
    }

    fn finish(&self, result: Result<(), String>) {
        let mut state = self.lock();
        state.done = true;
        state.error = result.err();
        drop(state);
        self.grew.notify_all();
    }

    /// Stops the download; waiting readers return an error.
    pub fn cancel(&self) {
        self.lock().cancelled = true;
        self.grew.notify_all();
    }

    /// The fraction downloaded, when the length is known.
    pub fn progress(&self) -> Option<f64> {
        let state = self.lock();
        if state.done && state.error.is_none() {
            return Some(1.0);
        }
        let total = state.total.filter(|total| *total > 0)?;
        Some((state.bytes.len() as f64 / total as f64).min(1.0))
    }

    /// Why the download failed, once it has.
    pub fn error(&self) -> Option<String> {
        self.lock().error.clone()
    }

    pub fn reader(self: &Arc<Self>) -> DownloadReader {
        DownloadReader {
            download: self.clone(),
            position: 0,
        }
    }
}

/// Reads a [`Download`] as it arrives.
pub(crate) struct DownloadReader {
    download: Arc<Download>,
    position: u64,
}

impl Read for DownloadReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.download.lock();
        loop {
            if state.cancelled {
                return Err(io::Error::other("download cancelled"));
            }
            let have = state.bytes.len() as u64;
            if self.position < have {
                let start = self.position as usize;
                let read = buf.len().min(state.bytes.len() - start);
                buf[..read].copy_from_slice(&state.bytes[start..start + read]);
                self.position += read as u64;
                return Ok(read);
            }
            if state.done {
                return match &state.error {
                    Some(error) => Err(io::Error::other(error.clone())),
                    None => Ok(0),
                };
            }
            // Ahead of the network: wait for the next chunk.
            state = self.download.grew.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

impl Seek for DownloadReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let state = self.download.lock();
        let end = state.total.unwrap_or(state.bytes.len() as u64);
        let target = match to {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => end.checked_add_signed(delta),
        };
        // A target past what has arrived is fine: reading it waits.
        self.position = target
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        Ok(self.position)
    }
}

impl MediaSource for DownloadReader {
    fn is_seekable(&self) -> bool {
        // Only when the end is known; probing seeks there to look for tags.
        self.download.lock().total.is_some()
    }

    fn byte_len(&self) -> Option<u64> {
        self.download.lock().total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reader_reads_seeks_and_reports_the_end() {
        let download = Download::complete((0..10).collect());
        let mut reader = download.reader();
        let mut buf = [0; 4];
        assert_eq!(reader.read(&mut buf).unwrap(), 4);
        assert_eq!(buf, [0, 1, 2, 3]);
        assert_eq!(reader.seek(SeekFrom::End(-2)).unwrap(), 8);
        assert_eq!(reader.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], &[8, 9]);
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
        assert!(reader.seek(SeekFrom::Current(-20)).is_err());
        assert_eq!(download.progress(), Some(1.0));
        assert!(reader.is_seekable());
        assert_eq!(reader.byte_len(), Some(10));
    }

    #[test]
    fn a_reader_waits_for_bytes_that_have_not_arrived() {
        let download = Arc::new(Download {
            state: Mutex::new(State {
                total: Some(4),
                ..State::default()
            }),
            grew: Condvar::new(),
        });
        let feeder = download.clone();
        let reader_thread = thread::spawn({
            let mut reader = download.reader();
            move || {
                let mut all = Vec::new();
                reader.read_to_end(&mut all).map(|_| all)
            }
        });
        assert_eq!(download.progress(), Some(0.0));
        for byte in [7, 8, 9, 10] {
            feeder.lock().bytes.push(byte);
            feeder.grew.notify_all();
        }
        feeder.finish(Ok(()));
        assert_eq!(reader_thread.join().unwrap().unwrap(), vec![7, 8, 9, 10]);
    }

    #[test]
    fn a_failed_or_cancelled_download_unblocks_its_reader_with_an_error() {
        let download = Download::complete(Vec::new());
        download.lock().done = false;
        download.cancel();
        assert!(download.reader().read(&mut [0; 1]).is_err());

        let download = Download::complete(Vec::new());
        download.finish(Err("connection reset".into()));
        assert!(download.reader().read(&mut [0; 1]).is_err());
        assert_eq!(download.error().as_deref(), Some("connection reset"));
        assert_eq!(download.progress(), None);
    }
}
