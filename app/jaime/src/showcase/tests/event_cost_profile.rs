//! What handling an input event costs, apart from the frame it asks for.
//!
//! The other profiling harnesses start their timer after the event has been
//! delivered, so they measure the frame that follows an event and say nothing
//! about the event. This one times the two apart, for the interactions an
//! application spends most of its time on:
//!
//! * `event`: [`send_window_event`](aimer::quiver::aimer_app::HeadlessAimerApp::send_window_event),
//!   the path the windowed loop takes (hit test, hover reconciliation, gesture
//!   recognisers, handlers). A frame is drawn inside it only for the events
//!   that a window answers with one of its own.
//! * `frame`: the `render_frame` that follows, drawn only when the event asked
//!   for one, as a window would.
//!
//! Run unoptimised, which is the mode this repository optimises for, one test
//! at a time:
//!
//! ```text
//! cargo nextest run -p jaime profile_event_costs --run-ignored only --no-capture
//! ```
//!
//! `EVENTS=<n>` sizes the run (default 300); `ONLY=<name>` runs one scenario so
//! a sampler can attach to it; `ROUNDS=<n>` repeats each scenario and reports the
//! median of the per-round medians. `CLEAN_SKIP_PROBE=1` also prints what the
//! element walk left alone per frame and why it did not leave more alone, and
//! `CLEAN_SKIP_VERIFY=1` walks what it would have left alone and prints what
//! that changed in the retained tree. It prints timings only and asserts nothing.

use super::*;

use aimer_time::Instant;

use aimer::HeadlessOptions;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{
    DeviceId, ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent,
};

const SCALE: f64 = 2.0;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name).ok().and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn cursor(device: DeviceId, x: f64, y: f64) -> WindowEvent {
    WindowEvent::CursorMoved { device_id: device, position: PhysicalPosition::new(x, y) }
}

fn wheel(device: DeviceId, dy: f64) -> WindowEvent {
    WindowEvent::MouseWheel {
        device_id: device,
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, dy)),
        phase: TouchPhase::Moved,
    }
}

fn button(device: DeviceId, state: ElementState) -> WindowEvent {
    WindowEvent::MouseInput { device_id: device, state, button: MouseButton::Left }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)]
}

/// One scenario: a name, where the cursor rests first, and the events to send.
struct Scenario {
    name: &'static str,
    rest: (f64, f64),
    events: Vec<WindowEvent>,
}

fn scenarios(device: DeviceId, count: usize) -> Vec<Scenario> {
    let logical = |x: f64, y: f64| (x * SCALE, y * SCALE);
    // The sidebar rows, one row height apart, as the hover harness walks them.
    let row_y = |step: usize| 100.0 + (step % 8) as f64 * 42.0;
    let (cx, cy) = logical(800.0, 400.0);
    let (sx, sy) = logical(150.0, 200.0);

    vec![
        Scenario {
            name: "hover: sidebar rows (each move changes the hovered row)",
            rest: (sx, sy),
            events: (0..count)
                .map(|step| {
                    let (x, y) = logical(150.0, row_y(step));
                    cursor(device, x, y)
                })
                .collect(),
        },
        Scenario {
            name: "hover: small moves inside the content (no target changes)",
            rest: (cx, cy),
            events: (0..count).map(|step| cursor(device, cx + (step % 16) as f64, cy)).collect(),
        },
        Scenario {
            name: "scroll: wheel over the content",
            rest: (cx, cy),
            events: (0..count)
                .map(|step| wheel(device, if (step / 20) % 2 == 0 { -48.0 } else { 48.0 }))
                .collect(),
        },
        Scenario {
            name: "scroll: wheel over the sidebar",
            rest: (sx, sy),
            events: (0..count)
                .map(|step| wheel(device, if (step / 20) % 2 == 0 { -48.0 } else { 48.0 }))
                .collect(),
        },
        Scenario {
            name: "click: press + release on the selected sidebar row",
            rest: logical(150.0, row_y(0)),
            events: (0..count)
                .map(|step| {
                    button(
                        device,
                        if step % 2 == 0 { ElementState::Pressed } else { ElementState::Released },
                    )
                })
                .collect(),
        },
        Scenario {
            name: "drag: press, move 40 steps across the content, release",
            rest: (cx, cy),
            events: (0..count)
                .map(|step| match step % 42 {
                    0 => button(device, ElementState::Pressed),
                    41 => button(device, ElementState::Released),
                    n => cursor(device, cx + n as f64 * 8.0, cy + (n % 5) as f64),
                })
                .collect(),
        },
    ]
}

fn summarize(label: &str, times: &mut [f64]) -> String {
    times.sort_by(f64::total_cmp);
    format!(
        "{label} p50={:>7.0} us  p95={:>7.0} us  max={:>7.0} us  (n={})",
        percentile(times, 0.5),
        percentile(times, 0.95),
        times.last().copied().unwrap_or(0.0),
        times.len(),
    )
}

#[test]
#[ignore = "profiling harness: prints timings and asserts nothing"]
fn profile_event_costs() {
    let count = env_usize("EVENTS", 300);
    let rounds = env_usize("ROUNDS", 1);
    let only = std::env::var("ONLY").ok();
    let device = DeviceId::dummy();
    if std::env::var_os("CLEAN_SKIP_PROBE").is_some() {
        aimer_widget::set_clean_skip_blocker_probe(true);
    }
    let verify = std::env::var_os("CLEAN_SKIP_VERIFY").is_some();
    aimer_widget::set_clean_skip_verify(verify);

    for scenario in scenarios(device, count) {
        if only.as_deref().is_some_and(|only| !scenario.name.starts_with(only)) {
            continue;
        }
        let mut event_medians = Vec::new();
        let mut frame_medians = Vec::new();
        let mut last = (String::new(), String::new(), 0usize, 0.0f64);
        for _ in 0..rounds {
            let mut app = AimerApp::start_headless_with(
                theme::provide(ExampleShowcase::new().boxed()),
                HeadlessOptions { size: PhysicalSize::new(2560, 1600), scale_factor: SCALE },
            );
            app.pump_frames(8);
            app.send_window_event(cursor(device, scenario.rest.0, scenario.rest.1));
            app.pump_frames(3);

            let mut event_times = Vec::with_capacity(scenario.events.len());
            let mut frame_times = Vec::new();
            aimer_widget::reset_clean_subtree_skip_stats();
            let wall = Instant::now();
            for event in &scenario.events {
                let start = Instant::now();
                app.send_window_event(event.clone());
                event_times.push(start.elapsed().as_secs_f64() * 1e6);
                if app.take_redraw_request() {
                    let start = Instant::now();
                    app.render_frame();
                    frame_times.push(start.elapsed().as_secs_f64() * 1e6);
                }
            }
            let total_us = wall.elapsed().as_secs_f64() * 1e6;
            let frames = frame_times.len();
            if verify {
                let violations = aimer_widget::clean_skip_violations();
                eprintln!(
                    "===   verified {} subtrees, {} violations",
                    aimer_widget::clean_subtree_skip_stats().verified,
                    violations.len(),
                );
                for violation in violations.iter().take(10) {
                    eprintln!("===     {:?}: {}", violation.root, violation.detail);
                }
            }
            if std::env::var_os("CLEAN_SKIP_PROBE").is_some() {
                let stats = aimer_widget::clean_subtree_skip_stats();
                let per = frames.max(1) as f64;
                eprintln!(
                    "===   per frame: walked {:.0} elements, skipped {:.1} subtrees, recorded {:.1}, tree queries {:.1}",
                    stats.visits as f64 / per,
                    stats.skips as f64 / per,
                    stats.records as f64 / per,
                    stats.tree_queries as f64 / per,
                );
                for (at, count) in aimer_widget::clean_skip_frame_requesters().into_iter().take(8) {
                    eprintln!("===     {:>7.1}/frame  frames requested from {at}", count as f64 / per);
                }
                for (name, count) in aimer_widget::clean_skip_blockers().into_iter().take(10) {
                    eprintln!("===     {:>7.1}/frame  {name}", count as f64 / per);
                }
            }
            event_medians.push(percentile(&{ let mut t = event_times.clone(); t.sort_by(f64::total_cmp); t }, 0.5));
            frame_medians.push(percentile(&{ let mut t = frame_times.clone(); t.sort_by(f64::total_cmp); t }, 0.5));
            last = (
                summarize("event", &mut event_times),
                summarize("frame", &mut frame_times),
                frames,
                total_us,
            );
        }
        event_medians.sort_by(f64::total_cmp);
        frame_medians.sort_by(f64::total_cmp);
        eprintln!("=== {}", scenario.name);
        eprintln!("===   {}", last.0);
        eprintln!("===   {}   ({} of {} events asked for one)", last.1, last.2, scenario.events.len());
        eprintln!(
            "===   per event, all in: {:.0} us   (median of {} rounds: event p50 {:.0} us, frame p50 {:.0} us)",
            last.3 / scenario.events.len() as f64,
            rounds,
            event_medians[event_medians.len() / 2],
            frame_medians[frame_medians.len() / 2],
        );
    }

    // What a frame costs when nothing happened: the floor every event above pays
    // whenever it asks for one.
    if only.is_none() {
        let mut app = AimerApp::start_headless_with(
            theme::provide(ExampleShowcase::new().boxed()),
            HeadlessOptions { size: PhysicalSize::new(2560, 1600), scale_factor: SCALE },
        );
        app.pump_frames(8);
        let mut idle = Vec::with_capacity(count);
        for _ in 0..count {
            let start = Instant::now();
            app.render_frame();
            idle.push(start.elapsed().as_secs_f64() * 1e6);
        }
        eprintln!("=== idle frame (no event)");
        eprintln!("===   {}", summarize("frame", &mut idle));
    }
}
