//! Leaving unchanged subtrees alone leaves every showcase page drawing what it
//! draws without it, through a script of scrolling, hovering and resizing.
//!
//! The showcase animates on wall-clock time and the scroll smoother runs on the
//! clock, so a page's frames can differ between two runs that do the same thing.
//! A page that is still asking for frames after its settling frames is left out,
//! and so is any page whose frames already differ between two runs with the skip
//! off: those are the ones a comparison could not prove anything about.
//!
//! ```text
//! cargo nextest run -p jaime -j 1 --run-ignored only \
//!     -E 'test(clean_skip_pages)' --no-capture
//! ```

use super::set_state_profile::{Captured, Labels, click_at};
use super::*;

use std::rc::Rc;

use aimer::HeadlessOptions;
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};

/// Logical width of the sidebar; labels to its right belong to the page.
const SIDEBAR_LOGICAL_WIDTH: f32 = 292.0;
const TALL: u32 = 4200;
const SHORT: u32 = 700;

type Frames = Vec<Vec<String>>;

/// Rounds every number with a fraction to a whole pixel: the scroll smoother
/// settles within a hundredth of a pixel of where it settled the last time.
fn quantize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut number = String::new();
    let flush = |number: &mut String, out: &mut String| {
        if number.contains('.') && let Ok(value) = number.parse::<f64>() {
            out.push_str(&format!("{value:.0}"));
        } else {
            out.push_str(number);
        }
        number.clear();
    };
    for ch in text.chars() {
        if ch.is_ascii_digit() || ch == '.' || (ch == '-' && number.is_empty()) {
            number.push(ch);
        } else {
            flush(&mut number, &mut out);
            out.push(ch);
        }
    }
    flush(&mut number, &mut out);
    out
}

fn plan<W: Widget + 'static>(app: &aimer::quiver::HeadlessAimerApp<W>) -> Vec<String> {
    app.render_plan().iter().map(|line| quantize(line)).collect()
}

fn move_cursor<W: Widget + 'static>(app: &mut aimer::quiver::HeadlessAimerApp<W>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.render_frame();
    app.render_frame();
}

fn wheel<W: Widget + 'static>(app: &mut aimer::quiver::HeadlessAimerApp<W>, delta: f64) {
    app.send_window_event(WindowEvent::MouseWheel {
        device_id: DeviceId::dummy(),
        delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, delta)),
        phase: TouchPhase::Moved,
    });
    let mut frames = 0;
    while (app.scroll_motion_active() || frames == 0) && frames < 90 {
        app.render_frame();
        frames += 1;
    }
}

fn resize<W: Widget + 'static>(app: &mut aimer::quiver::HeadlessAimerApp<W>, height: u32) {
    app.send_window_event(WindowEvent::Resized(PhysicalSize::new(1280, height)));
    app.render_frame();
    app.render_frame();
}

/// One pass over every reachable page: its label, what it drew at each step of
/// the script, and whether it was still asking for frames when it was read.
fn run(skip: bool) -> (Vec<(&'static str, Frames, bool)>, aimer_widget::CleanSkipStats) {
    let (pages, stats, _) = run_with(skip, false);
    (pages, stats)
}

/// [`run`], optionally in verify mode, and the violations it found.
fn run_with(
    skip: bool,
    verify: bool,
) -> (
    Vec<(&'static str, Frames, bool)>,
    aimer_widget::CleanSkipStats,
    Vec<aimer_widget::CleanSkipViolation>,
) {
    aimer_widget::set_clean_subtree_skip(skip);
    aimer_widget::reset_clean_subtree_skip_stats();
    aimer_widget::set_clean_skip_verify(verify);
    let labels = Rc::new(Labels::default());
    let mut app = AimerApp::start_headless_with(
        theme::provide(Captured {
            labels: labels.clone(),
        }),
        HeadlessOptions {
            size: PhysicalSize::new(1280, TALL),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(8);
    labels.capture.set(true);
    app.render_frame();
    labels.capture.set(false);
    let sidebar: Vec<(String, f32, f32)> = labels
        .found
        .borrow()
        .iter()
        .filter(|(_, x, _)| *x < SIDEBAR_LOGICAL_WIDTH)
        .cloned()
        .collect();

    let mut pages = Vec::new();
    for example in EXAMPLES {
        let Some((_, x, y)) = sidebar.iter().find(|(text, _, _)| text == example.label()) else {
            continue;
        };
        click_at(&mut app, *x, *y, 1.0);
        for _ in 0..6 {
            app.render_frame();
        }
        // A window short enough that the page's content scrolls.
        resize(&mut app, SHORT);
        let mut frames = Vec::new();
        move_cursor(&mut app, 800.0, 350.0);
        frames.push(plan(&app));
        for _ in 0..3 {
            wheel(&mut app, -48.0);
            frames.push(plan(&app));
        }
        wheel(&mut app, 48.0);
        frames.push(plan(&app));
        // The pointer visits the sidebar and comes back to the content.
        move_cursor(&mut app, 100.0, 300.0);
        frames.push(plan(&app));
        move_cursor(&mut app, 800.0, 350.0);
        wheel(&mut app, -48.0);
        frames.push(plan(&app));
        let animating = app.take_redraw_request();
        pages.push((example.label(), frames, animating));
        resize(&mut app, TALL);
    }
    let stats = aimer_widget::clean_subtree_skip_stats();
    let violations = aimer_widget::clean_skip_violations();
    aimer_widget::set_clean_skip_verify(false);
    aimer_widget::set_clean_subtree_skip(true);
    (pages, stats, violations)
}

/// The first step whose frame differs, and the first line of it that does.
fn first_difference(a: &Frames, b: &Frames) -> Option<String> {
    for (step, (x, y)) in a.iter().zip(b).enumerate() {
        if x == y {
            continue;
        }
        let line = x.iter().zip(y).position(|(p, q)| p != q).unwrap_or(x.len().min(y.len()));
        return Some(format!(
            "step {step}: {} lines against {}; first difference at line {line}:\n    without: {:?}\n    with:    {:?}",
            x.len(),
            y.len(),
            x.get(line),
            y.get(line)
        ));
    }
    None
}

#[test]
#[ignore = "slow whole-app differential: three passes over every showcase page"]
fn clean_skip_pages_draw_the_same_frames_with_the_skip() {
    let (baseline, _) = run(false);
    let (again, _) = run(false);
    let (skipping, stats) = run(true);
    assert!(
        baseline.len() > 20,
        "too few pages were reachable: {}",
        baseline.len()
    );

    let mut held = 0;
    let mut unstable = Vec::new();
    let mut differing = Vec::new();
    for (((label, a, a_animating), (_, b, b_animating)), (_, c, c_animating)) in
        baseline.iter().zip(&again).zip(&skipping)
    {
        if *a_animating || *b_animating || *c_animating || a != b {
            unstable.push(*label);
            continue;
        }
        held += 1;
        if a != c {
            differing.push((*label, first_difference(a, c)));
        }
    }
    println!(
        "pages {}: {held} held to the comparison, {} animating or unstable {unstable:?}",
        baseline.len(),
        unstable.len()
    );
    println!("with the skip: {stats:?}");

    assert!(held * 2 >= baseline.len(), "most pages were too unstable to compare");
    assert!(differing.is_empty(), "frames differ with the skip: {differing:#?}");
    assert!(stats.skips > 0, "the skip never applied, so the comparison proves nothing");
}

/// Verify mode over every page, animating ones included: each subtree the skip
/// would have left alone is walked, and what the walk leaves in the retained tree
/// is compared with what was there before. There is no second run, so a page that
/// differs between two runs of itself is as good a subject as any other.
#[test]
#[ignore = "slow whole-app check: one pass over every showcase page in verify mode"]
fn clean_skip_pages_lose_nothing_when_verified() {
    let (pages, stats, violations) = run_with(true, true);
    assert!(pages.len() > 20, "too few pages were reachable: {}", pages.len());

    let mut by_element = std::collections::BTreeMap::<String, usize>::new();
    for violation in &violations {
        let name = violation.detail.split(' ').next().unwrap_or("?").to_owned();
        *by_element.entry(name).or_default() += 1;
    }
    println!(
        "pages {}: verified {} subtrees, {} violations {by_element:?}",
        pages.len(),
        stats.verified,
        violations.len()
    );
    for violation in violations.iter().take(15) {
        println!("  {}", violation.detail);
    }
    assert!(stats.verified > 0, "nothing was verified, so the run proves nothing");
    assert!(violations.is_empty(), "walks changed what a skip would have left alone: {violations:#?}");
}
