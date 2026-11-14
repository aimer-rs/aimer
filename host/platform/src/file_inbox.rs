//! Files the platform asks the app to open, from whichever thread it does so.
//!
//! The registry that opens files ([`aimer_events::file_open`]) belongs to the UI
//! thread. The platform does not: AppKit, a JVM activity and a launcher's `main`
//! each report from their own place, and the launch file can arrive before the
//! event loop exists to be woken. So paths are posted here, which any thread may
//! do at any time, and the event loop drains them on its own thread: at start, and
//! whenever a post wakes it. It is the same arrangement as [`crate::link_inbox`].

use std::collections::VecDeque;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A bounded, thread-safe queue of paths.
#[derive(Default)]
pub struct Inbox {
    paths: Mutex<VecDeque<PathBuf>>,
}

impl Inbox {
    /// More than a person can open before the loop wakes; the oldest are dropped.
    pub const CAPACITY: usize = aimer_events::file_open::MAX_WAITING;

    pub const fn new() -> Self {
        Self { paths: Mutex::new(VecDeque::new()) }
    }

    /// Queues `paths` in order, returning whether the event loop needs waking: only
    /// the first post after a drain does, because a wake-up already on its way will
    /// drain the rest. Empty paths are not files and are ignored.
    pub fn post(&self, paths: impl IntoIterator<Item = PathBuf>) -> bool {
        // A poisoned lock only means another thread panicked holding it; the
        // queue itself is still a valid queue.
        let mut queue = self.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_empty = queue.is_empty();
        let mut posted = false;
        for path in paths.into_iter().filter(|path| !path.as_os_str().is_empty()) {
            if queue.len() == Self::CAPACITY {
                queue.pop_front();
            }
            queue.push_back(path);
            posted = true;
        }
        posted && was_empty
    }

    /// Takes every queued path, oldest first.
    pub fn drain(&self) -> Vec<PathBuf> {
        let mut queue = self.paths.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.drain(..).collect()
    }
}

static INBOX: Inbox = Inbox::new();

/// Hands files to the app from any thread, waking the event loop if it exists.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn post_files(paths: impl IntoIterator<Item = PathBuf>) {
    if INBOX.post(paths)
        && let Some(proxy) = crate::event::EVENT_PROXY.get()
    {
        let _ = proxy.send_event(crate::event::AimerNativePlatformEvent::OpenFiles);
    }
}

/// Hands the app a URL it was opened with: a `file:` URL is a file to open, and
/// anything else is a link, as [`crate::link_inbox::post_link`] takes it.
///
/// A file URL that names no local path — another host, or an encoding that is not a
/// path — is dropped rather than guessed at.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios", target_os = "android")), allow(dead_code))]
pub fn post_url(url: String) {
    if is_file_url(&url) {
        if let Some(path) = file_url_to_path(&url) {
            post_files([path]);
        }
    } else {
        crate::link_inbox::post_link(url);
    }
}

/// Delivers every posted file to the registered handlers. Called on the UI
/// thread, where the handlers live.
pub fn deliver_posted_files() {
    let paths = INBOX.drain();
    if !paths.is_empty() {
        aimer_events::file_open::deliver_files(&paths);
    }
}

fn is_file_url(url: &str) -> bool {
    url.get(..5).is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
}

/// The local path a `file:` URL names, or `None` if it names none.
///
/// Accepts `file:///path`, `file://localhost/path` and the Windows form
/// `file:///C:/path`, and decodes `%XX` escapes. A host other than `localhost`
/// is a network location, not a path on this machine.
pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    if !is_file_url(url) {
        return None;
    }
    let rest = url[5..].strip_prefix("//")?;
    let slash = rest.find('/')?;
    let (host, path) = rest.split_at(slash);
    if !host.is_empty() && !host.eq_ignore_ascii_case("localhost") {
        return None;
    }
    // A query or fragment is not part of a path.
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let decoded = percent_decode(path)?;
    // `/C:/work` is how a URL spells a drive.
    let bytes = decoded.as_slice();
    let drive = bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':';
    let decoded = if drive { &decoded[1..] } else { bytes };
    // A NUL ends a path in every API that takes one.
    if decoded.is_empty() || decoded.contains(&0) {
        return None;
    }
    Some(PathBuf::from(os_string(decoded)?))
}

fn percent_decode(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let digit = |b: u8| (b as char).to_digit(16);
            out.push((digit(hex[0])? * 16 + digit(hex[1])?) as u8);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    Some(out)
}

#[cfg(unix)]
fn os_string(bytes: &[u8]) -> Option<OsString> {
    use std::os::unix::ffi::OsStringExt;
    Some(OsString::from_vec(bytes.to_vec()))
}

#[cfg(not(unix))]
fn os_string(bytes: &[u8]) -> Option<OsString> {
    // Windows paths are text; bytes that are not UTF-8 name nothing there.
    String::from_utf8(bytes.to_vec()).ok().map(OsString::from)
}

/// The files named on the command line a program was started with, in order.
///
/// `args` is the arguments *after* the program's own name. A word that starts with
/// `-` is an option, not a file; every other word is a file if `exists` says it
/// names something. This is what a file manager does on Windows and Linux when a
/// file is dropped on the program or opened with it: it starts the program with
/// the path as an argument.
///
/// A word that names nothing is skipped instead of being passed on, so a program
/// with arguments of its own — a subcommand, a number — does not see them as files.
pub fn launch_files(args: impl IntoIterator<Item = OsString>, exists: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    args.into_iter()
        .filter(|arg| !arg.is_empty() && !arg.to_string_lossy().starts_with('-'))
        .map(PathBuf::from)
        .filter(|path| exists(path))
        .collect()
}

/// Posts the files the process was started with, for a platform whose launcher
/// passes them as arguments. Call it once, before the event loop starts.
#[cfg(any(target_os = "windows", target_os = "linux"))]
pub fn post_launch_files() {
    let files = launch_files(std::env::args_os().skip(1), |path| path.exists());
    post_files(files);
}

#[cfg(test)]
#[path = "file_inbox_tests.rs"]
mod tests;
