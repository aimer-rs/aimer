//! Captions and chapters from sidecar text files: WebVTT and SubRip (SRT).
//!
//! Both formats are parsed here, in platform-neutral Rust, so captions look
//! and behave the same on every platform and are drawn as framework text.
//! Parsing is lenient, like a browser's: a malformed cue is skipped, not the
//! whole file.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// One caption: text shown from `start` until `end`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    start: Duration,
    end: Duration,
    text: String,
}

impl Cue {
    /// A cue showing `text` over `start..end`.
    pub fn new(start: Duration, end: Duration, text: impl Into<String>) -> Self {
        Self {
            start,
            end,
            text: text.into(),
        }
    }

    /// When the cue appears.
    #[inline]
    pub fn start(&self) -> Duration {
        self.start
    }

    /// When the cue disappears.
    #[inline]
    pub fn end(&self) -> Duration {
        self.end
    }

    /// The plain text, with styling tags removed and lines joined by `\n`.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Why a caption file could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptionError {
    /// The bundled asset or file does not exist or cannot be read.
    NotFound(String),
    /// The text is neither WebVTT nor SRT with at least one valid cue.
    Invalid,
}

impl fmt::Display for CaptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(what) => write!(f, "caption file not found: {what}"),
            Self::Invalid => f.write_str("not a WebVTT or SRT caption file"),
        }
    }
}

impl std::error::Error for CaptionError {}

/// Where a caption or chapter file comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptionSource {
    /// A file registered in `Aimer.toml`, by its project-relative path.
    Bundled(String),
    /// A file on the host file system.
    File(PathBuf),
    /// The file's contents, already in memory (fetched by the app, say).
    Text(String),
}

impl CaptionSource {
    /// Reads the source's text. Caption files are small, so this reads in
    /// place; call it off the UI thread for a slow disk if that matters.
    fn read(&self) -> Result<String, CaptionError> {
        let read = |path: PathBuf| {
            std::fs::read_to_string(&path).map_err(|_| CaptionError::NotFound(path.display().to_string()))
        };
        match self {
            Self::Text(text) => Ok(text.clone()),
            Self::File(path) => read(path.clone()),
            Self::Bundled(key) => {
                let exe = std::env::current_exe().ok();
                let path = crate::assets::bundled_candidates(key, exe.as_deref())
                    .into_iter()
                    .find(|candidate| candidate.is_file())
                    .ok_or_else(|| CaptionError::NotFound(key.clone()))?;
                read(path)
            }
        }
    }
}

/// A caption track: cues in time order.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use aimer_video::CaptionTrack;
///
/// let track = CaptionTrack::parse(
///     "WEBVTT\n\n00:01.000 --> 00:03.500\nHello, <b>world</b>!\n",
/// )
/// .unwrap();
/// assert_eq!(track.text_at(Duration::from_secs(2)).as_deref(), Some("Hello, world!"));
/// assert_eq!(track.text_at(Duration::from_secs(4)), None);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptionTrack {
    cues: Vec<Cue>,
    /// The longest cue, which bounds how far back an active cue can start.
    longest: Duration,
}

impl CaptionTrack {
    /// A track from cues in any order.
    pub fn from_cues(mut cues: Vec<Cue>) -> Self {
        cues.retain(|cue| cue.end > cue.start);
        cues.sort_by_key(|cue| cue.start);
        let longest = cues.iter().map(|cue| cue.end - cue.start).max().unwrap_or_default();
        Self { cues, longest }
    }

    /// Parses WebVTT (a file starting with `WEBVTT`) or SRT.
    pub fn parse(text: &str) -> Result<Self, CaptionError> {
        let cues = parse_cues(text)?;
        Ok(Self::from_cues(cues))
    }

    /// Reads and parses `source`.
    pub fn load(source: &CaptionSource) -> Result<Self, CaptionError> {
        Self::parse(&source.read()?)
    }

    /// Every cue, in time order.
    #[inline]
    pub fn cues(&self) -> &[Cue] {
        &self.cues
    }

    /// The cues showing at `at`, in the order they started.
    pub fn cues_at(&self, at: Duration) -> impl Iterator<Item = &Cue> {
        // Cues starting after `at` are not showing; ones starting more than
        // the longest cue ago have ended. Only the span between is checked.
        let upper = self.cues.partition_point(|cue| cue.start <= at);
        let earliest = at.saturating_sub(self.longest);
        let lower = self.cues[..upper].partition_point(|cue| cue.start < earliest);
        self.cues[lower..upper].iter().filter(move |cue| at < cue.end)
    }

    /// The text showing at `at`, overlapping cues joined line by line, or
    /// `None` between cues.
    pub fn text_at(&self, at: Duration) -> Option<String> {
        let mut text: Option<String> = None;
        for cue in self.cues_at(at) {
            match &mut text {
                Some(text) => {
                    text.push('\n');
                    text.push_str(cue.text());
                }
                None => text = Some(cue.text.clone()),
            }
        }
        text
    }

    /// An identity for the cues showing at `at`: it changes exactly when the
    /// showing text does, so a view can skip work while it stays the same.
    pub(crate) fn showing_at(&self, at: Duration) -> (usize, usize) {
        let upper = self.cues.partition_point(|cue| cue.start <= at);
        let earliest = at.saturating_sub(self.longest);
        let lower = self.cues[..upper].partition_point(|cue| cue.start < earliest);
        let mut first = usize::MAX;
        let mut count = 0;
        for (index, cue) in self.cues[lower..upper].iter().enumerate() {
            if at < cue.end {
                first = first.min(lower + index);
                count += 1;
            }
        }
        (first, count)
    }
}

/// One chapter of a video.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    start: Duration,
    title: String,
}

impl Chapter {
    /// A chapter starting at `start`.
    pub fn new(start: Duration, title: impl Into<String>) -> Self {
        Self {
            start,
            title: title.into(),
        }
    }

    /// Where the chapter starts.
    #[inline]
    pub fn start(&self) -> Duration {
        self.start
    }

    /// The chapter's title.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }
}

/// A video's chapters, in time order. Each runs until the next one starts;
/// the last runs to the end of the video.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use aimer_video::{Chapter, Chapters};
///
/// let chapters = Chapters::new(vec![
///     Chapter::new(Duration::ZERO, "Intro"),
///     Chapter::new(Duration::from_secs(90), "The good part"),
/// ]);
/// assert_eq!(chapters.at(Duration::from_secs(100)).map(|c| c.title()), Some("The good part"));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Chapters {
    chapters: Vec<Chapter>,
}

impl Chapters {
    /// Chapters from a list in any order. Several at the same time keep only
    /// the first.
    pub fn new(mut chapters: Vec<Chapter>) -> Self {
        chapters.sort_by_key(|chapter| chapter.start);
        chapters.dedup_by_key(|chapter| chapter.start);
        Self { chapters }
    }

    /// Parses a WebVTT chapters file (or SRT): each cue's start and text
    /// become a chapter's start and title.
    pub fn parse(text: &str) -> Result<Self, CaptionError> {
        let cues = parse_cues(text)?;
        Ok(Self::new(
            cues.into_iter()
                .map(|cue| Chapter::new(cue.start, cue.text.replace('\n', " ")))
                .collect(),
        ))
    }

    /// Reads and parses `source`.
    pub fn load(source: &CaptionSource) -> Result<Self, CaptionError> {
        Self::parse(&source.read()?)
    }

    /// Every chapter, in time order.
    #[inline]
    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }

    /// Whether there are none.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    /// The chapter playing at `at`: the last one started by then.
    pub fn at(&self, at: Duration) -> Option<&Chapter> {
        let index = self.chapters.partition_point(|chapter| chapter.start <= at);
        index.checked_sub(1).map(|index| &self.chapters[index])
    }

    /// The chapter boundaries as fractions of `duration`, for drawing a
    /// segmented track: each chapter's `(start, end)` in `0.0..=1.0`.
    pub fn spans(&self, duration: Duration) -> Vec<(f64, f64)> {
        let total = duration.as_secs_f64();
        if total <= 0.0 || self.chapters.is_empty() {
            return Vec::new();
        }
        let fraction = |time: Duration| (time.as_secs_f64() / total).clamp(0.0, 1.0);
        let mut spans: Vec<(f64, f64)> = Vec::with_capacity(self.chapters.len());
        for (index, chapter) in self.chapters.iter().enumerate() {
            let start = if index == 0 { 0.0 } else { fraction(chapter.start) };
            let end = self.chapters.get(index + 1).map_or(1.0, |next| fraction(next.start));
            if end > start {
                spans.push((start, end));
            }
        }
        spans
    }
}

/// Parses WebVTT when the text says so, SRT otherwise.
fn parse_cues(text: &str) -> Result<Vec<Cue>, CaptionError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let is_vtt = text
        .lines()
        .next()
        .is_some_and(|first| first == "WEBVTT" || first.starts_with("WEBVTT ") || first.starts_with("WEBVTT\t"));
    let mut cues = Vec::new();
    let mut blocks = text.split("\n\n").map(|block| block.trim_matches('\n'));
    if is_vtt {
        // The header block (`WEBVTT` and any metadata) carries no cue.
        blocks.next();
    }
    for block in blocks {
        if block.is_empty() {
            continue;
        }
        if is_vtt && (block.starts_with("NOTE") || block.starts_with("STYLE") || block.starts_with("REGION")) {
            continue;
        }
        let mut lines = block.lines();
        // An optional identifier (VTT) or index (SRT) precedes the timing.
        let Some(mut timing) = lines.next() else {
            continue;
        };
        if !timing.contains("-->") {
            let Some(next) = lines.next() else {
                continue;
            };
            timing = next;
        }
        let Some((start, end)) = parse_timing(timing) else {
            continue;
        };
        let text = lines.map(clean_line).collect::<Vec<_>>().join("\n");
        let text = text.trim();
        if !text.is_empty() {
            cues.push(Cue::new(start, end, text));
        }
    }
    if cues.is_empty() && !text.trim().is_empty() && !is_vtt {
        return Err(CaptionError::Invalid);
    }
    Ok(cues)
}

/// `start --> end [settings]`.
fn parse_timing(line: &str) -> Option<(Duration, Duration)> {
    let (start, rest) = line.split_once("-->")?;
    let end = rest.split_whitespace().next()?;
    Some((parse_timestamp(start.trim())?, parse_timestamp(end)?))
}

/// `[hh:]mm:ss.ttt` (VTT) or `hh:mm:ss,ttt` (SRT).
fn parse_timestamp(text: &str) -> Option<Duration> {
    let text = text.replace(',', ".");
    let (clock, fraction) = text.split_once('.').unwrap_or((&text, "0"));
    if fraction.is_empty() || fraction.len() > 3 || !fraction.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let millis: u64 = format!("{fraction:0<3}").parse().ok()?;
    let parts = clock.split(':').map(|part| {
        (!part.is_empty() && part.bytes().all(|b| b.is_ascii_digit())).then(|| part.parse::<u64>().ok()).flatten()
    });
    let parts: Option<Vec<u64>> = parts.collect();
    let parts = parts?;
    let (hours, minutes, seconds) = match parts.as_slice() {
        [minutes, seconds] => (0, *minutes, *seconds),
        [hours, minutes, seconds] => (*hours, *minutes, *seconds),
        _ => return None,
    };
    if minutes > 59 || seconds > 59 {
        return None;
    }
    Some(Duration::from_millis(((hours * 60 + minutes) * 60 + seconds) * 1000 + millis))
}

/// Removes styling markup (`<i>`, `<c.yellow>`, `<v Speaker>`, `{\an8}`) and
/// decodes the entities WebVTT allows.
fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '<' => {
                for c in chars.by_ref() {
                    if c == '>' {
                        break;
                    }
                }
            }
            // SRT positioning such as `{\an8}`.
            '{' if chars.peek() == Some(&'\\') => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
            }
            '&' => {
                let mut entity = String::new();
                while let Some(&next) = chars.peek() {
                    if next == ';' || entity.len() > 6 || !next.is_ascii_alphanumeric() {
                        break;
                    }
                    entity.push(next);
                    chars.next();
                }
                let decoded = match entity.as_str() {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "nbsp" => Some('\u{a0}'),
                    "lrm" => Some('\u{200e}'),
                    "rlm" => Some('\u{200f}'),
                    _ => None,
                };
                match decoded {
                    Some(decoded) if chars.peek() == Some(&';') => {
                        chars.next();
                        out.push(decoded);
                    }
                    _ => {
                        out.push('&');
                        out.push_str(&entity);
                    }
                }
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
#[path = "captions_tests.rs"]
mod tests;
