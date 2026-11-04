//! Linux: talking to the `ffmpeg` and `ffprobe` programs.
//!
//! FFmpeg decodes (and streams: http(s) and HLS) in a child process that writes
//! raw RGBA pictures to a pipe. A program rather than a linked library keeps
//! this independent of which FFmpeg version the system has: the program's
//! command line is stable where the libraries' structures are not.

use std::collections::VecDeque;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;

/// Pictures decoded ahead of the playhead. When the queue is full the reader
/// stops reading, the pipe fills, and FFmpeg waits: decoding costs nothing
/// while paused.
const QUEUE: usize = 4;
/// The most pictures a second a video is shown at.
const MAX_FPS: f64 = 60.0;

/// What `ffprobe` says about a source.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Info {
    /// The picture's size once rotated upright.
    pub size: (u32, u32),
    /// Pictures a second, capped at 60 and never zero.
    pub fps: f64,
    /// Length in seconds, or `None` for a live stream.
    pub duration: Option<f64>,
    pub has_audio: bool,
}

/// Parses `ffprobe -of default=nw=1` output for the video stream, plus the
/// names of the audio streams.
pub(crate) fn parse_info(video: &str, audio: &str) -> Option<Info> {
    let mut width = None;
    let mut height = None;
    let mut fps = None;
    let mut rotation = 0.0_f64;
    let mut duration: Option<f64> = None;
    for line in video.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        match key.trim() {
            "width" => width = value.parse::<u32>().ok(),
            "height" => height = value.parse::<u32>().ok(),
            "avg_frame_rate" => fps = parse_rate(value),
            "rotation" | "TAG:rotate" => rotation = value.parse().unwrap_or(0.0),
            // The stream and the container may both say; the longer is right.
            "duration" => {
                if let Ok(seconds) = value.parse::<f64>()
                    && seconds.is_finite()
                    && seconds > 0.0
                {
                    duration = Some(duration.map_or(seconds, |known| known.max(seconds)));
                }
            }
            _ => {}
        }
    }
    let (mut width, mut height) = (width?, height?);
    if width == 0 || height == 0 {
        return None;
    }
    // FFmpeg rotates phone video upright; so must the size we expect.
    if (rotation.abs() / 90.0).round() as i64 % 2 == 1 {
        std::mem::swap(&mut width, &mut height);
    }
    Some(Info {
        size: (width, height),
        fps: fps.unwrap_or(30.0).clamp(1.0, MAX_FPS),
        duration,
        has_audio: audio.lines().any(|line| !line.trim().is_empty()),
    })
}

/// `30000/1001`, or `30`, as a rate; `None` for `0/0`.
fn parse_rate(text: &str) -> Option<f64> {
    let rate = match text.split_once('/') {
        Some((numerator, denominator)) => {
            let denominator: f64 = denominator.trim().parse().ok()?;
            if denominator == 0.0 {
                return None;
            }
            numerator.trim().parse::<f64>().ok()? / denominator
        }
        None => text.trim().parse().ok()?,
    };
    (rate.is_finite() && rate > 0.0).then_some(rate)
}

/// Whether `program` runs.
pub(crate) fn available(program: &str) -> bool {
    Command::new(program)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Asks `ffprobe` about `input`.
pub(crate) fn probe(input: &str) -> Result<Info, String> {
    let run = |args: &[&str]| -> Result<String, String> {
        let output = Command::new("ffprobe")
            .args(["-v", "error"])
            .args(args)
            .arg(input)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| format!("could not run ffprobe: {error}"))?;
        if !output.status.success() {
            let why = String::from_utf8_lossy(&output.stderr);
            return Err(why.trim().lines().last().unwrap_or("ffprobe failed").to_string());
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    };
    let video = run(&[
        "-select_streams",
        "v:0",
        "-show_entries",
        "stream=width,height,avg_frame_rate,duration:stream_side_data=rotation:stream_tags=rotate:format=duration",
        "-of",
        "default=nw=1",
    ])?;
    let audio = run(&["-select_streams", "a", "-show_entries", "stream=codec_name", "-of", "csv=p=0"])?;
    parse_info(&video, &audio).ok_or_else(|| "the source has no video stream".to_string())
}

/// One decoded picture.
pub(crate) struct Picture {
    /// Seconds from the start of the source.
    pub pts: f64,
    pub pixels: Vec<u8>,
}

#[derive(Default)]
struct Queue {
    pictures: VecDeque<Arc<Picture>>,
    /// The decoder has no more pictures.
    finished: bool,
    /// Why the decoder ended early, if it did.
    failure: Option<String>,
    /// The session was dropped; the reader must stop.
    closed: bool,
}

struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
}

/// A running `ffmpeg`, decoding from one point.
pub(crate) struct Session {
    shared: Arc<Shared>,
    child: Child,
    reader: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts decoding `input` from `start` seconds, as pictures of `info`'s
    /// size at `info.fps`.
    pub(crate) fn start(input: &str, info: &Info, start: f64) -> Result<Self, String> {
        let mut command = Command::new("ffmpeg");
        command.args(["-nostdin", "-loglevel", "error"]);
        if input.starts_with("http://") || input.starts_with("https://") {
            command.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "4"]);
        }
        if start > 0.0 {
            command.args(["-ss", &format!("{start:.3}")]);
        }
        let child = command
            .args(["-i", input, "-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
            .arg(format!("fps={}", info.fps))
            .args(["-pix_fmt", "rgba", "-f", "rawvideo", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("could not run ffmpeg: {error}"))?;
        let mut child = child;
        let stdout = child.stdout.take().ok_or("ffmpeg has no output pipe")?;
        let stderr = child.stderr.take();
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue::default()),
            changed: Condvar::new(),
        });
        let reader = {
            let shared = shared.clone();
            let size = info.size;
            let fps = info.fps;
            std::thread::Builder::new()
                .name("aimer-video-ffmpeg".into())
                .spawn(move || read_pictures(stdout, stderr, &shared, size, fps, start))
                .map_err(|error| error.to_string())?
        };
        Ok(Self {
            shared,
            child,
            reader: Some(reader),
        })
    }

    /// Pictures ready to show.
    pub(crate) fn queued(&self) -> usize {
        self.lock().pictures.len()
    }

    /// Takes the picture to show at `position`: the newest one due by then,
    /// dropping any it supersedes. `wanted` is how early a picture may be
    /// shown (half a picture's duration, so a picture is never a frame late).
    pub(crate) fn take_due(&self, position: f64, wanted: f64) -> Option<Arc<Picture>> {
        let mut queue = self.lock();
        let mut due = None;
        while queue.pictures.front().is_some_and(|picture| picture.pts <= position + wanted) {
            due = queue.pictures.pop_front();
        }
        if due.is_some() {
            self.shared.changed.notify_all();
        }
        due
    }

    /// Takes the first picture whatever its time, for the one a seek or the
    /// start shows before the playhead moves.
    pub(crate) fn take_first(&self) -> Option<Arc<Picture>> {
        let picture = self.lock().pictures.pop_front();
        if picture.is_some() {
            self.shared.changed.notify_all();
        }
        picture
    }

    /// Whether every picture has been decoded and taken.
    pub(crate) fn drained(&self) -> bool {
        let queue = self.lock();
        queue.finished && queue.pictures.is_empty()
    }

    /// Why decoding stopped early, once nothing is left to show.
    pub(crate) fn failure(&self) -> Option<String> {
        let queue = self.lock();
        queue.failure.clone().filter(|_| queue.pictures.is_empty())
    }

    /// The time of the newest picture decoded.
    pub(crate) fn decoded_to(&self) -> Option<f64> {
        self.lock().pictures.back().map(|picture| picture.pts)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queue> {
        self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.queue.lock().unwrap_or_else(PoisonError::into_inner).closed = true;
        self.shared.changed.notify_all();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Reads `size` pictures from the pipe until it ends, keeping at most
/// [`QUEUE`] waiting.
fn read_pictures(
    mut stdout: impl Read,
    stderr: Option<impl Read + Send + 'static>,
    shared: &Shared,
    size: (u32, u32),
    fps: f64,
    start: f64,
) {
    // FFmpeg's complaints, for the error message.
    let complaints = stderr.map(|mut stderr| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        })
    });
    let length = size.0 as usize * size.1 as usize * 4;
    let mut index = 0_u64;
    loop {
        // Backpressure: leave the next picture in the pipe until there is room.
        {
            let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
            while queue.pictures.len() >= QUEUE && !queue.closed {
                queue = shared.changed.wait(queue).unwrap_or_else(PoisonError::into_inner);
            }
            if queue.closed {
                return;
            }
        }
        let mut pixels = vec![0_u8; length];
        if stdout.read_exact(&mut pixels).is_err() {
            break;
        }
        let picture = Arc::new(Picture {
            pts: start + index as f64 / fps,
            pixels,
        });
        index += 1;
        let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
        if queue.closed {
            return;
        }
        queue.pictures.push_back(picture);
        shared.changed.notify_all();
    }
    // The pipe ended: the end of the video, or a failure.
    let complaints = complaints.and_then(|handle| handle.join().ok()).unwrap_or_default();
    let mut queue = shared.queue.lock().unwrap_or_else(PoisonError::into_inner);
    if index == 0 && !complaints.trim().is_empty() {
        queue.failure = Some(complaints.trim().lines().last().unwrap_or_default().to_string());
    }
    queue.finished = true;
    shared.changed.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_parse_as_fractions_and_whole_numbers() {
        assert_eq!(parse_rate("30/1"), Some(30.0));
        assert!((parse_rate("30000/1001").unwrap() - 29.97).abs() < 0.01);
        assert_eq!(parse_rate("25"), Some(25.0));
        assert_eq!(parse_rate("0/0"), None);
        assert_eq!(parse_rate("n/a"), None);
    }

    #[test]
    fn probe_output_gives_size_rate_length_and_audio() {
        let video = "width=1920\nheight=1080\navg_frame_rate=30000/1001\nduration=N/A\nduration=12.5\n";
        let info = parse_info(video, "aac\n").unwrap();
        assert_eq!(info.size, (1920, 1080));
        assert!((info.fps - 29.97).abs() < 0.01);
        assert_eq!(info.duration, Some(12.5));
        assert!(info.has_audio);
    }

    #[test]
    fn a_rotated_video_is_measured_upright() {
        let video = "width=1920\nheight=1080\navg_frame_rate=30/1\nrotation=-90\n";
        assert_eq!(parse_info(video, "").unwrap().size, (1080, 1920));
        let tagged = "width=1920\nheight=1080\navg_frame_rate=30/1\nTAG:rotate=180\n";
        assert_eq!(parse_info(tagged, "").unwrap().size, (1920, 1080));
    }

    #[test]
    fn a_live_stream_has_no_length_and_an_unknown_rate_defaults() {
        let info = parse_info("width=640\nheight=360\navg_frame_rate=0/0\nduration=N/A\n", "").unwrap();
        assert_eq!(info.duration, None);
        assert_eq!(info.fps, 30.0);
        assert!(!info.has_audio);
    }

    #[test]
    fn rates_are_capped() {
        let info = parse_info("width=64\nheight=64\navg_frame_rate=240/1\n", "").unwrap();
        assert_eq!(info.fps, MAX_FPS);
    }

    #[test]
    fn output_without_a_size_is_not_a_video() {
        assert_eq!(parse_info("avg_frame_rate=30/1\n", "aac\n"), None);
        assert_eq!(parse_info("width=0\nheight=0\n", ""), None);
    }
}
