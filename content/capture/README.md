# aimer_capture

Turns what an Aimer app draws into an image: the whole window, or the content of
one widget.

An agent that drives an app can act on it but cannot see the result. A person
debugging on a phone cannot attach a screenshot tool to the app. Both want the
app to hand over its own picture, and that is what this crate asks the renderer
for.

## The window

```rust,no_run
use aimer_capture::{Format, screenshot, DEFAULT_TIMEOUT};

// From a worker thread of a running app — never the one that draws.
let picture = screenshot(DEFAULT_TIMEOUT)?;
let png = picture.scaled_to_width(1280).encode(Format::Png)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The renderer copies a finished frame out in order, between drawing it and
presenting it, so the picture is what was really presented: nothing else on the
screen, no other window, and no permission asked of the operating system. An app
that is not changing draws nothing, so asking wakes it to draw a frame. The
picture is in physical pixels; `Image::scale` says how many make a logical pixel,
and it keeps saying so after the picture is cropped or shrunk.

Only the renderer that presents on Apple's Metal (macOS and iOS) can do this
today. Every other renderer answers `CaptureError::Unsupported` at once rather
than leaving the asker waiting.

### Which thread does what

| Step | Thread |
|---|---|
| Copying the frame into a buffer | the one drawing it, in order behind the drawing |
| Waiting for the copy and reading it | the same one, after the frame has been handed on |
| Shrinking and encoding | the caller's — a worker, never the UI thread |

Reading the copy waits for the GPU for a moment on the drawing thread, after the
frame is on its way, so taking a picture does not delay showing one.

## A widget's content

`Capture` wraps a child. It takes no space and draws nothing; a `CaptureHandle`
finds where the child is, and asks for a picture of it.

```rust
use aimer_capture::{Capture, CaptureHandle};
use aimer_widget::ErrorWidget;

let handle = CaptureHandle::new();
let card = Capture::new().handle(&handle).child(ErrorWidget::new("card"));

// On the thread that draws the tree, read where it is and file the request:
//     let request = handle.request()?;
// Then wait for the frame on a worker, or poll it each frame:
//     let image = request.wait(std::time::Duration::from_secs(3))?;
# let _ = card;
```

The picture is that part of the window **as it is drawn**: whatever overlaps the
child is in it, and whatever is clipped away is out of it. That is what a person
sees there, and it is what this is for. It is not an isolated render of the
child, which would draw it again off screen at any size with a transparent
background; that is a different tool, and not this one. A child scrolled out of
the window reports `ShotError::OffScreen`, and one that has not been drawn yet
`ShotError::NotShown`.

## Encoding

`Image` encodes to a lossless PNG, or to a much smaller JPEG for a busy window.
`Image::scaled_to_width` never enlarges. Both use the `image` crate the
workspace already has. Resizing and compressing millions of pixels is slow
unoptimized, so the workspace builds `image`, `png`, `fdeflate` and this crate
with optimizations in debug profiles: a screenshot of a retina window takes tens
of milliseconds there, not over a second.
