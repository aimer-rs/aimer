use std::collections::HashSet;

use super::*;

mod animation_walks;
mod backdrop_filter;
mod focus;
mod foreground_decoration;
mod gradient;
mod gradient_resize;
mod gradient_scroll;
mod light_edge;
mod light_edge_color;
mod magnification;
mod morph_transition;
mod multi_decoration_box;
mod navigation_transitions;
mod scrolling;
mod svg_asset_first_frame;
mod selection;
mod set_state_profile;

#[test]
fn lucide_gallery_is_registered_with_a_constructible_page() {
    let example = EXAMPLES.iter().find(|example| example.key() == "lucide-icons")
        .expect("Lucide gallery must be selectable");
    assert_eq!(example.label(), "Lucide icons");
    let _ = build_example(*example, theme::app_theme());
}

#[test]
fn showcase_has_unique_registered_examples() {
    assert!(!EXAMPLES.is_empty());
    let keys = EXAMPLES
        .iter()
        .map(|example| example.key())
        .collect::<HashSet<_>>();

    assert_eq!(keys.len(), EXAMPLES.len());
}

#[test]
fn sidebar_categories_cover_each_registered_example_once() {
    let categorized = EXAMPLE_CATEGORIES
        .iter()
        .flat_map(|category| {
            assert!(
                !category
                    .label
                    .is_empty()
            );
            assert!(
                !category
                    .examples
                    .is_empty()
            );
            category
                .examples
                .iter()
                .map(|example| example.key())
        })
        .collect::<Vec<_>>();
    let registered = EXAMPLES
        .iter()
        .map(|example| example.key())
        .collect::<HashSet<_>>();
    let categorized_set = categorized
        .iter()
        .copied()
        .collect::<HashSet<_>>();

    assert_eq!(categorized.len(), EXAMPLES.len());
    assert_eq!(categorized_set.len(), categorized.len());
    assert_eq!(categorized_set, registered);
}

#[test]
fn every_registered_example_has_metadata_and_a_constructible_page() {
    for example in EXAMPLES {
        assert!(
            !example
                .label()
                .is_empty()
        );
        assert!(
            !example
                .description()
                .is_empty()
        );
        let _page = build_example(*example, theme::app_theme());
    }
}

/// Pages that still depend on the legacy paint path, by label. Empty: every
/// page now paints through the retained v2 path, so
/// `every_page_renders_without_legacy_paint` fails for any page that
/// regresses. A page may be listed here only as a temporary, reviewed
/// exception; the test also fails for a listed page that is clean, and
/// run with `--nocapture` it prints what holds a listed page back.
const LEGACY_PAGE_BACKLOG: &[&str] = &[];

/// Every page must paint through the retained v2 path: no element may fall
/// back to a legacy island, none may draw without a render node (its paint
/// would be silently lost), none may report unusable bounds, and the render
/// tree must synchronize. This is the gate for removing the legacy
/// `Drawable::update` path; pages in [`LEGACY_PAGE_BACKLOG`] are the known work.
#[test]
fn every_page_renders_without_legacy_paint() {
    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::PhysicalSize;

    let mut regressions = Vec::new();
    let mut now_clean = Vec::new();
    let mut seen_backlog = Vec::new();
    for example in EXAMPLES {
        let page = build_example(*example, theme::app_theme());
        let mut app = AimerApp::start_headless_with(
            theme::provide(page),
            HeadlessOptions {
                size: PhysicalSize::new(1280, 800),
                scale_factor: 1.0,
            },
        );
        app.pump_frames(6);
        let census = app
            .paint_source_census()
            .expect("a mounted page has a root");

        let names = |roots: &[aimer::quiver::handler::census::CensusElement]| -> Vec<&'static str> {
            roots
                .iter()
                .map(|root| root.debug_name)
                .collect()
        };
        let declined = names(&census.declined_paint);
        let unmapped = names(&census.drawn_unmapped);
        let invalid = names(&census.invalid_bounds);
        let offends = !declined.is_empty()
            || !unmapped.is_empty()
            || !invalid.is_empty()
            || census
                .sync_error
                .is_some();
        let label = example.label();
        let known = LEGACY_PAGE_BACKLOG.contains(&label);
        if known {
            seen_backlog.push(label);
        }
        let detail = format!(
            "{label:?}: had no retained paint {declined:?}, drawn without a render node {unmapped:?}, \
             invalid bounds {invalid:?}, sync error {:?}",
            census.sync_error
        );
        if offends && known {
            // Visible with `--nocapture`: what still holds a backlog page back.
            eprintln!("backlog {detail}");
        }
        match (offends, known) {
            (true, false) => regressions.push(detail),
            (false, true) => now_clean.push(label),
            _ => {}
        }
    }

    assert!(
        regressions.is_empty(),
        "pages that newly paint through the legacy path:\n{}",
        regressions.join("\n")
    );
    assert!(
        now_clean.is_empty(),
        "these pages are now clean; remove them from LEGACY_PAGE_BACKLOG: {now_clean:?}"
    );
    let unknown: Vec<_> = LEGACY_PAGE_BACKLOG
        .iter()
        .filter(|label| !seen_backlog.contains(label))
        .collect();
    assert!(
        unknown.is_empty(),
        "backlog names no registered page: {unknown:?}"
    );
}

/// Whether a disagreement concerns something on screen. Rows scrolled out of
/// view are not drawn, so their canvas measurement is stale while the render
/// tree's is current; only visible elements can be compared meaningfully.
fn is_on_screen(
    disagreement: &aimer::InteractionDisagreement,
    logical_width: f32,
    logical_height: f32,
) -> bool {
    let [x, y, width, height] = disagreement.tree;
    x < logical_width && y < logical_height && x + width > 0.0 && y + height > 0.0
}

/// Element types that take their hit area from the render tree. The gate
/// below requires each one to be exercised by some page, so migrating a
/// widget cannot pass the differential check by never being adopted.
const RENDER_TREE_HIT_AREAS: &[&str] = &[
    "GestureDetector",
    "MouseRegion",
    "Draggable",
    "DropZone",
    "DragTarget",
    "RawCalendar",
    "RawDatePicker",
    "RawDateTimePicker",
    "RawTimePicker",
    "RawColorPicker",
    "RawSlider",
    "RawRangeSlider",
    "SliderTrack",
    "SliderThumb",
    "SliderTrail",
    "Svg",
    "TextButton",
    "Container",
    "Resizable",
    "TextField",
    "AnimatedCollapseElement",
    "RawPositionedElement",
    "Anchor",
    "RawScrollableContainer",
    "RawRichText",
];

/// The elements that take their hit area from the render tree must land
/// where the legacy canvas transform stack put it. This is the differential
/// gate for retiring that stack: any disagreement means a click would land
/// somewhere new.
#[test]
fn render_tree_hit_areas_agree_with_the_canvas_on_every_page() {
    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::PhysicalSize;

    let mut adopted = 0;
    let mut adopted_types = std::collections::BTreeSet::new();
    let mut disagreements = Vec::new();
    for example in EXAMPLES {
        let mut app = AimerApp::start_headless_with(
            theme::provide(build_example(*example, theme::app_theme())),
            HeadlessOptions {
                size: PhysicalSize::new(1280, 800),
                scale_factor: 2.0,
            },
        );
        // Let asynchronous content settle (fonts and assets finish loading
        // between frames): an audit taken a frame after a page changed
        // would compare a stale canvas measurement. A disagreement that
        // survives repeated frames is a real one.
        app.pump_frames(60);
        let mut audit = app
            .bounds_audit()
            .expect("a mounted page has a root");
        for _ in 0..40 {
            if !audit
                .interaction_disagreements
                .iter()
                .any(|disagreement| is_on_screen(disagreement, 640.0, 400.0))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
            for _ in 0..3 {
                app.render_frame();
            }
            audit = app
                .bounds_audit()
                .expect("a mounted page has a root");
        }
        adopted += audit.interaction_adopted;
        adopted_types.extend(
            audit
                .interaction_adopted_by_type
                .keys()
                .copied(),
        );
        for disagreement in audit.interaction_disagreements {
            if is_on_screen(&disagreement, 640.0, 400.0) {
                disagreements.push(format!("{:?}: {disagreement:?}", example.label()));
            }
        }
    }

    assert!(
        adopted > 0,
        "no page has an element on render-tree hit areas"
    );
    let unexercised: Vec<_> = RENDER_TREE_HIT_AREAS
        .iter()
        .filter(|name| !adopted_types.contains(**name))
        .collect();
    assert!(
        unexercised.is_empty(),
        "no page exercises these render-tree hit areas: {unexercised:?}"
    );
    assert!(
        disagreements.is_empty(),
        "render-tree hit areas moved away from the canvas-derived ones:\n{}",
        disagreements.join("\n")
    );
}

/// The tree reads each slider visual's position when it syncs, which is
/// before the slider's own update has worked the position out. The very
/// first frame must already place the track, trail and thumb where they are
/// drawn: nothing asks for a second one.
#[test]
fn slider_visuals_are_placed_by_the_first_frame() {
    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::PhysicalSize;

    let mut app = AimerApp::start_headless_with(
        theme::provide(build_example(ExampleId::RangeControls, theme::app_theme())),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 800),
            scale_factor: 2.0,
        },
    );
    app.render_frame();
    let audit = app
        .bounds_audit()
        .expect("a mounted page has a root");
    let misplaced: Vec<_> = audit
        .interaction_disagreements
        .iter()
        .filter(|disagreement| is_on_screen(disagreement, 640.0, 400.0))
        .collect();
    assert!(
        misplaced.is_empty(),
        "the first frame left slider visuals off their drawn place: {misplaced:?}"
    );
}

/// Scrolling moves every row of the sidebar, each a gesture detector. The
/// render tree must carry the hit areas along with the canvas.
#[test]
fn render_tree_hit_areas_follow_a_scrolled_sidebar() {
    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};

    let mut app = AimerApp::start_headless_with(
        theme::provide(ExampleShowcase::new().boxed()),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 800),
            scale_factor: 2.0,
        },
    );
    let device = DeviceId::dummy();
    app.pump_frames(8);
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: device,
        position: PhysicalPosition::new(120.0, 400.0),
    });
    app.pump_frames(2);

    let mut adopted = 0;
    for step in 0..12 {
        for _ in 0..4 {
            app.send_window_event(WindowEvent::MouseWheel {
                device_id: device,
                delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -60.0)),
                phase: TouchPhase::Moved,
            });
            app.pump_frames(1);
        }
        app.pump_frames(30);
        let audit = app
            .bounds_audit()
            .expect("a mounted page has a root");
        adopted = adopted.max(audit.interaction_adopted);
        let visible: Vec<_> = audit
            .interaction_disagreements
            .iter()
            .filter(|disagreement| is_on_screen(disagreement, 640.0, 400.0))
            .collect();
        assert!(visible.is_empty(), "after scroll step {step}: {visible:?}");
    }
    assert!(
        adopted > 0,
        "the sidebar rows must be on render-tree hit areas"
    );
}

/// Prints, for every page, how many cached interaction rectangles agree with
/// the render tree and which element types do not. A diagnostic for moving
/// hit-test geometry off the legacy canvas transform stack.
#[test]
#[ignore = "diagnostic: prints a per-page report and asserts nothing"]
fn report_cached_bounds_against_the_render_tree() {
    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::PhysicalSize;

    let mut by_type: std::collections::BTreeMap<&'static str, (usize, usize)> = Default::default();
    for example in EXAMPLES {
        let mut app = AimerApp::start_headless_with(
            theme::provide(build_example(*example, theme::app_theme())),
            HeadlessOptions {
                size: PhysicalSize::new(1280, 800),
                scale_factor: 2.0,
            },
        );
        app.pump_frames(6);
        let audit = app
            .bounds_audit()
            .expect("a mounted page has a root");
        eprintln!(
            "bounds {:?}: compared {}, unmapped {}, mismatched {}",
            example.label(),
            audit.compared,
            audit.unmapped,
            audit
                .mismatches
                .len()
        );
        for mismatch in &audit.mismatches {
            by_type
                .entry(mismatch.debug_name)
                .or_default()
                .1 += 1;
        }
        if !audit
            .interaction_disagreements
            .is_empty()
        {
            eprintln!(
                "  RENDER-TREE HIT AREA DISAGREES: {:?}",
                audit
                    .interaction_disagreements
                    .iter()
                    .take(6)
                    .collect::<Vec<_>>()
            );
        }
        for (name, entry) in by_type.iter_mut() {
            let _ = name;
            let _ = entry;
        }
        eprintln!(
            "  first mismatches: {:?}",
            audit
                .mismatches
                .iter()
                .take(12)
                .map(|m| (m.debug_name, m.cached, m.render))
                .collect::<Vec<_>>()
        );
    }
    eprintln!("bounds mismatches by element type: {by_type:?}");
}

#[test]
fn the_default_selection_is_the_first_registered_example() {
    let state = ExampleShowcase::new().create_state();

    assert_eq!(state.selection.read().selected, EXAMPLES[0]);
}

#[test]
fn showcase_mounts_with_the_first_example_visible() {
    let mut app = AimerApp::start_headless(ExampleShowcase::new());

    app.pump_frames(2);
}

/// Moves the cursor down the sidebar one row at a time, so every frame
/// leaves one button and enters the next (hover state changes, rebuilds and
/// event-index maintenance).
///
/// Run with `cargo test -p jaime --lib -- --ignored --nocapture
/// profile_sidebar_hover_frames`. It prints timings only; it asserts nothing.
#[test]
#[ignore = "profiling harness: prints timings and asserts nothing"]
fn profile_sidebar_hover_frames() {
    use std::time::Instant;

    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, WindowEvent};

    let mut app = AimerApp::start_headless_with(
        theme::provide(ExampleShowcase::new().boxed()),
        HeadlessOptions {
            size: PhysicalSize::new(2560, 1600),
            scale_factor: 2.0,
        },
    );
    let device = DeviceId::dummy();
    app.pump_frames(8);

    let frames: usize = std::env::var("HOVER_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400);
    let mut times = Vec::with_capacity(frames);
    for step in 0..frames {
        // Logical y of the first sidebar rows, spaced by one row height.
        let logical_y = 100.0 + (step % 8) as f64 * 42.0;
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: device,
            // Physical pixels: logical x = 150, at the 2.0 scale factor.
            position: PhysicalPosition::new(300.0, logical_y * 2.0),
        });
        let start = Instant::now();
        app.render_frame();
        times.push(
            start
                .elapsed()
                .as_secs_f64()
                * 1e6,
        );
    }
    times.sort_by(f64::total_cmp);
    eprintln!(
        "=== hover frame: p50={:.0} us  p95={:.0} us  max={:.0} us  ({} frames)",
        times[times.len() / 2],
        times[(times.len() as f64 * 0.95) as usize - 1],
        times[times.len() - 1],
        times.len(),
    );
}

/// Drags the window edge back and forth, one `Resized` event per step, the way
/// an iPad Stage Manager or desktop edge drag delivers them.
///
/// Each event renders its frame immediately, so the time of
/// `send_window_event(Resized)` is the cost the drag pays per step; the frame
/// that follows it (`render_frame`) is the redundant repaint a platform asks
/// for after the resize. Run with `RESIZE_FRAMES=<n>` to lengthen the run so a
/// sampler can attach. It prints timings only; it asserts nothing.
#[test]
#[ignore = "profiling harness: prints timings and asserts nothing"]
fn profile_resize_frames() {
    use std::time::Instant;

    use aimer::HeadlessOptions;
    use aimer::quiver::winit::dpi::PhysicalSize;
    use aimer::quiver::winit::event::WindowEvent;

    let mut app = AimerApp::start_headless_with(
        theme::provide(ExampleShowcase::new().boxed()),
        HeadlessOptions {
            size: PhysicalSize::new(2253, 1628),
            scale_factor: 2.0,
        },
    );
    app.pump_frames(8);

    let frames: usize = std::env::var("RESIZE_FRAMES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(400);
    let mut resize_us = Vec::with_capacity(frames);
    let mut follow_up_us = Vec::with_capacity(frames);
    let requests = |app: &aimer::HeadlessAimerApp<_>| {
        app.frame_request_stats()
            .accepted
    };
    let requests_before = requests(&app);
    for step in 0..frames {
        // A triangle wave between 1630 and 2250 physical px wide, ~60 px a step.
        let phase = (step * 60) % 1240;
        let width = 1630 + if phase < 620 { phase } else { 1240 - phase } as u32;
        let height = 1200 + (width - 1630) / 2;
        let start = Instant::now();
        app.send_window_event(WindowEvent::Resized(PhysicalSize::new(width, height)));
        resize_us.push(
            start
                .elapsed()
                .as_secs_f64()
                * 1e6,
        );
        let start = Instant::now();
        app.render_frame();
        follow_up_us.push(
            start
                .elapsed()
                .as_secs_f64()
                * 1e6,
        );
    }
    eprintln!(
        "=== redraw requests per resize step: {:.2}",
        (requests(&app) - requests_before) as f64 / frames as f64
    );
    for (label, times) in [("resize", &mut resize_us), ("follow-up", &mut follow_up_us)] {
        times.sort_by(f64::total_cmp);
        eprintln!(
            "=== {label} frame: p50={:.0} us  p95={:.0} us  max={:.0} us  ({} frames)",
            times[times.len() / 2],
            times[(times.len() as f64 * 0.95) as usize - 1],
            times[times.len() - 1],
            times.len(),
        );
    }
}

#[test]
#[ignore = "profiling harness: 30,000 frames (minutes in debug), prints timings and asserts nothing"]
fn profile_sidebar_scroll_frames() {
    use std::time::Instant;

    use aimer::HeadlessOptions;
    use aimer::quiver::frame_stats::{
        frame_breakdown, frame_content_stats, reset_frame_breakdown, reset_frame_content_stats,
    };
    use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
    use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};

    let mut app = AimerApp::start_headless_with(
        theme::provide(ExampleShowcase::new().boxed()),
        HeadlessOptions {
            size: PhysicalSize::new(2560, 1600),
            scale_factor: 2.0,
        },
    );
    let device = DeviceId::dummy();
    app.pump_frames(8);

    // Park the cursor inside the sidebar so wheel events target it.
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: device,
        position: PhysicalPosition::new(120.0, 400.0),
    });
    app.pump_frames(2);

    // Idle frames: no new input, just re-drawing an unchanged tree.
    reset_frame_breakdown();
    reset_frame_content_stats();
    let idle_start = Instant::now();
    for _ in 0..30 {
        app.render_frame();
    }
    let idle_us = idle_start
        .elapsed()
        .as_secs_f64()
        * 1e6
        / 30.0;
    let idle_content = frame_content_stats();

    // Trackpad-style scroll frames with the cursor hovering the sidebar.
    reset_frame_breakdown();
    reset_frame_content_stats();
    let mut times = Vec::new();
    for _ in 0..30000 {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: device,
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -24.0)),
            phase: TouchPhase::Moved,
        });
        // Keep the sidebar oscillating so every frame scrolls.
        if times.len() % 100 == 99 {
            for _ in 0..50 {
                app.send_window_event(WindowEvent::MouseWheel {
                    device_id: device,
                    delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, 48.0)),
                    phase: TouchPhase::Moved,
                });
                app.render_frame();
            }
        }
        let start = Instant::now();
        app.render_frame();
        times.push(
            start
                .elapsed()
                .as_secs_f64()
                * 1e6,
        );
        if times.len() >= 60 && std::env::var("PROFILE_ONCE").is_ok() {
            break;
        }
    }
    times.sort_by(f64::total_cmp);
    let scroll_content = frame_content_stats();
    let scroll_breakdown = frame_breakdown();

    // This control moves the cursor back to the right pane before the
    // frame drains each queued wheel event. It measures hover crossings
    // plus a wheel miss, not scrolling the sidebar.
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: device,
        position: PhysicalPosition::new(900.0, 400.0),
    });
    app.pump_frames(2);
    reset_frame_breakdown();
    reset_frame_content_stats();
    let mut miss_times = Vec::new();
    for _ in 0..60 {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: device,
            position: PhysicalPosition::new(120.0, 400.0),
        });
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: device,
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -24.0)),
            phase: TouchPhase::Moved,
        });
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: device,
            position: PhysicalPosition::new(900.0, 400.0),
        });
        let start = Instant::now();
        app.render_frame();
        miss_times.push(
            start
                .elapsed()
                .as_secs_f64()
                * 1e6,
        );
    }
    miss_times.sort_by(f64::total_cmp);
    let miss_content = frame_content_stats();

    eprintln!("=== idle frame: {idle_us:.0} us");
    eprintln!(
        "=== idle per frame: nodes={:.0} cmds={:.0} retained={:.0} text_cmds={:.0} text_miss={:.1} layout_calls={:.0} hit_test_visits={:.0} paint_calls={:.0} rebuild_visits={:.0} rebuilds(S={:.1},L={:.1})",
        idle_content.average_drawn_nodes(),
        idle_content.average_draw_commands(),
        idle_content.average_retained_layers(),
        idle_content.average_text_commands(),
        idle_content.average_text_cache_misses(),
        idle_content.layout_calls as f64
            / idle_content
                .frames
                .max(1) as f64,
        idle_content.hit_test_visits as f64
            / idle_content
                .frames
                .max(1) as f64,
        idle_content.paint_calls as f64
            / idle_content
                .frames
                .max(1) as f64,
        idle_content.rebuild_visits as f64
            / idle_content
                .frames
                .max(1) as f64,
        idle_content.stateful_builds as f64
            / idle_content
                .frames
                .max(1) as f64,
        idle_content.stateless_builds as f64
            / idle_content
                .frames
                .max(1) as f64,
    );
    eprintln!(
        "=== scroll frame: p50={:.0} us  p95={:.0} us  max={:.0} us",
        times[times.len() / 2],
        times[(times.len() as f64 * 0.95) as usize - 1],
        times[times.len() - 1],
    );
    eprintln!(
        "=== scroll build phase avg: {:.0} us (encode {:.0} us, present {:.0} us)",
        scroll_breakdown
            .build
            .average()
            .as_secs_f64()
            * 1e6,
        scroll_breakdown
            .encode
            .average()
            .as_secs_f64()
            * 1e6,
        scroll_breakdown
            .present
            .average()
            .as_secs_f64()
            * 1e6,
    );
    eprintln!(
        "=== scroll per frame: nodes={:.0} cmds={:.0} retained={:.0} text_cmds={:.0} text_miss={:.1} layout_calls={:.0} hit_test_visits={:.0} paint_calls={:.0} rebuild_visits={:.0} rebuilds(S={:.1},L={:.1})",
        scroll_content.average_drawn_nodes(),
        scroll_content.average_draw_commands(),
        scroll_content.average_retained_layers(),
        scroll_content.average_text_commands(),
        scroll_content.average_text_cache_misses(),
        scroll_content.layout_calls as f64
            / scroll_content
                .frames
                .max(1) as f64,
        scroll_content.hit_test_visits as f64
            / scroll_content
                .frames
                .max(1) as f64,
        scroll_content.paint_calls as f64
            / scroll_content
                .frames
                .max(1) as f64,
        scroll_content.rebuild_visits as f64
            / scroll_content
                .frames
                .max(1) as f64,
        scroll_content.stateful_builds as f64
            / scroll_content
                .frames
                .max(1) as f64,
        scroll_content.stateless_builds as f64
            / scroll_content
                .frames
                .max(1) as f64,
    );
    eprintln!(
        "=== wheel miss after cursor crossings: p50={:.0} us  p95={:.0} us",
        miss_times[miss_times.len() / 2],
        miss_times[(miss_times.len() as f64 * 0.95) as usize - 1],
    );
    eprintln!(
        "=== wheel miss per frame: nodes={:.0} cmds={:.0} retained={:.0} text_cmds={:.0} text_miss={:.1} hit_test_visits={:.0} paint_calls={:.0} rebuild_visits={:.0} rebuilds(S={:.1},L={:.1})",
        miss_content.average_drawn_nodes(),
        miss_content.average_draw_commands(),
        miss_content.average_retained_layers(),
        miss_content.average_text_commands(),
        miss_content.average_text_cache_misses(),
        miss_content.hit_test_visits as f64
            / miss_content
                .frames
                .max(1) as f64,
        miss_content.paint_calls as f64
            / miss_content
                .frames
                .max(1) as f64,
        miss_content.rebuild_visits as f64
            / miss_content
                .frames
                .max(1) as f64,
        miss_content.stateful_builds as f64
            / miss_content
                .frames
                .max(1) as f64,
        miss_content.stateless_builds as f64
            / miss_content
                .frames
                .max(1) as f64,
    );
}
