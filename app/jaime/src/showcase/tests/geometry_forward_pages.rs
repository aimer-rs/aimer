//! Skipping redundant geometry forwards leaves every showcase page's published
//! geometry unchanged, and no child that was spared a forward goes unsynced.
//!
//! The showcase animates on wall-clock time, so a page that is still animating
//! when it is read has a state that depends on how fast the frames ran, and the
//! skip makes them faster. Such a page is left out: it is the one that still
//! asks for another frame after its settling frames. As a second guard the test
//! runs every page twice with the skip off and drops any whose geometry already
//! differs between those two runs.
//!
//! ```text
//! cargo nextest run -p jaime -j 1 --features aimer/frame-stats --run-ignored only \
//!     -E 'test(geometry_forward_pages)' --no-capture
//! ```

use super::set_state_profile::{Captured, Labels, click_at};
use super::*;

use std::rc::Rc;

use aimer::HeadlessOptions;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer_widget::geometry_walk_probe::{GeometryWalkReport, take_geometry_walk_report};

/// Logical width of the sidebar; labels to its right belong to the page.
const SIDEBAR_LOGICAL_WIDTH: f32 = 292.0;

type Rects = Vec<(&'static str, Option<[f32; 4]>)>;

/// Largest difference, in logical pixels, that still counts as the same box.
/// A rotation animation sampled on wall-clock time wobbles by about 1e-4 px
/// between identical runs; a missing geometry sync moves a box by whole pixels.
const TOLERANCE: f32 = 0.01;

fn same_geometry(a: &Rects, b: &Rects) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|((name_a, rect_a), (name_b, rect_b))| {
            name_a == name_b
                && match (rect_a, rect_b) {
                    (None, None) => true,
                    (Some(a), Some(b)) => a.iter().zip(b).all(|(a, b)| (a - b).abs() <= TOLERANCE),
                    _ => false,
                }
        })
}

/// The first element that differs, for the failure message.
fn first_difference(a: &Rects, b: &Rects) -> Option<(usize, String)> {
    if a.len() != b.len() {
        return Some((a.len().min(b.len()), format!("{} elements against {}", a.len(), b.len())));
    }
    a.iter()
        .zip(b)
        .position(|(x, y)| !same_geometry(&vec![x.clone()], &vec![y.clone()]))
        .map(|i| (i, format!("{:?} against {:?}", a[i], b[i])))
}

struct Run {
    /// Each page's label, its geometry, and whether it was still asking for
    /// frames, that is animating, when it was read.
    pages: Vec<(&'static str, Rects, bool)>,
    walk: GeometryWalkReport,
}

fn run(skip: bool) -> Run {
    aimer_widget::set_geometry_forward_skip(skip);
    let labels = Rc::new(Labels::default());
    let mut app = AimerApp::start_headless_with(
        theme::provide(Captured {
            labels: labels.clone(),
        }),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 4200),
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

    let mut run = Run {
        pages: Vec::new(),
        walk: GeometryWalkReport::default(),
    };
    let _ = take_geometry_walk_report();
    for example in EXAMPLES {
        let Some((_, x, y)) = sidebar.iter().find(|(text, _, _)| text == example.label()) else {
            continue;
        };
        click_at(&mut app, *x, *y, 1.0);
        // The probe is drained after every frame: element ids persist.
        run.walk.merge(&take_geometry_walk_report());
        for _ in 0..6 {
            app.render_frame();
            run.walk.merge(&take_geometry_walk_report());
        }
        let animating = app.take_redraw_request();
        run.pages.push((example.label(), app.interaction_rects(), animating));
    }
    aimer_widget::set_geometry_forward_skip(true);
    run
}

#[test]
#[ignore = "slow whole-app differential: three passes over every showcase page"]
fn geometry_forward_pages_publish_the_same_geometry_with_the_skip() {
    let baseline = run(false);
    let again = run(false);
    let skipping = run(true);
    assert!(
        baseline.pages.len() > 20,
        "too few pages were reachable: {}",
        baseline.pages.len()
    );

    let mut held = 0;
    let mut unstable = Vec::new();
    let mut differing = Vec::new();
    for (((label, a, a_animating), (_, b, b_animating)), (_, c, c_animating)) in
        baseline.pages.iter().zip(&again.pages).zip(&skipping.pages)
    {
        if *a_animating || *b_animating || *c_animating || !same_geometry(a, b) {
            unstable.push(*label);
            continue;
        }
        held += 1;
        if !same_geometry(a, c) {
            differing.push((*label, first_difference(a, c)));
        }
    }
    println!(
        "pages {}: {held} held to the comparison, {} animating or unstable {unstable:?}",
        baseline.pages.len(),
        unstable.len()
    );
    println!("without the skip: {:?}", baseline.walk);
    println!("with the skip:    {:?}", skipping.walk);

    assert!(held * 2 >= baseline.pages.len(), "most pages were too unstable to compare");
    assert!(differing.is_empty(), "geometry differs with the skip: {differing:#?}");
    assert_eq!(
        skipping.walk.unpaid_suppressions, 0,
        "a child was spared a forward and never synced itself"
    );
    // Without the `frame-stats` feature the probe records nothing.
    if skipping.walk.own_syncs > 0 {
        assert!(skipping.walk.suppressed_forwards > 0, "the skip never applied");
        assert!(skipping.walk.forwarded_visits < baseline.walk.forwarded_visits);
    }
}
