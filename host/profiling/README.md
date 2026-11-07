# aimer_profiling

The on-screen performance overlay for Aimer applications.

[`PerformanceOverlay`] paints a retained diagnostics panel above the
application and is toggled with `F3`. It shows:

| Field         | Meaning                                                                   |
|---------------|---------------------------------------------------------------------------|
| Draw FPS      | Frames the application drew per second.                                   |
| Present FPS   | Frames that reached the screen per second.                                |
| UI draw       | Mean UI-thread time of one application frame.                             |
| GPU frame     | Mean GPU time of one frame (Metal only, see below).                       |
| CPU           | Process CPU use in percent of one core; above 100 means several cores.    |
| RSS           | Process resident memory.                                                  |
| UI heap       | Bytes mapped by the Rubick UI heap, and its limit.                        |

With the `frame-stats` feature it also shows build / encode / present phase
timings.

The overlay is owned by the UI thread. While hidden it allocates nothing, reads
no clocks or counters, asks for no timer and does not switch GPU timing on.

## How the numbers are measured

Measurement runs in windows of a quarter of a second. A window's numbers are
shown when it closes, so every rate is a count divided by the real elapsed time
of the window, not by an assumed frame period.

**An idle application falls to zero.** The overlay only repaints when a frame
runs, so a host asks [`PerformanceOverlay::refresh_in`] how long to wait and
draws one more frame when that elapses. That frame measures the idle window.
The frames that exist only to refresh the overlay are not counted: the host
tells the overlay which frames the application itself changed
([`PerformanceOverlay::note_app_frame`]), and the presents those refresh frames
cause are subtracted from Present FPS. Without a host timer (a headless host)
an idle overlay keeps its last values.

**Draw FPS is not Present FPS.** A frame can be drawn and never shown, for
example when the surface cannot be acquired, and a frame handed to the raster
thread is shown a little after it was drawn.

**UI draw** covers build, layout and recording on the UI thread. Encoding and
presentation run elsewhere and are not in it.

**RSS** is the operating system's resident size: the Mach task's `resident_size`
on macOS and iOS, `/proc/self/statm` on Linux and Android, the working set on
Windows. The browser exposes none, so the field says `n/a` there. It is not the
number Xcode's memory gauge shows, which also counts compressed pages.

**CPU** is the change in user plus system time of every thread (`getrusage`, or
`GetProcessTimes` on Windows) over the window. The overlay's own work is
included; it is a few system calls four times a second.

**GPU frame** is the span from the earliest GPU start to the latest GPU end of
the frame's command buffers, from Metal's completion timestamps. It is collected
only while the overlay is visible. Other backends report no GPU time and the
field says `n/a`; it also reads `n/a` for a window in which nothing was drawn.

**UI heap** is address space mapped by the UI allocator, which the operating
system backs lazily. It is not resident memory; read RSS for that.

## Fields that say why they have no number

- `sampling`: the first window has not closed yet.
- `idle`: the application drew no frame in the window.
- `n/a`: the platform or backend has no source for it.

## Features

- `frame-stats`: also show the build / encode / present timings collected by
  `aimer_frame_stats`.
