use std::time::Duration;

use super::*;

fn total(plan: &Plan) -> (f32, f32) {
    plan.steps.iter().fold((0.0, 0.0), |a, s| (a.0 + s.0, a.1 + s.1))
}

fn duration(plan: &Plan) -> Duration {
    plan.interval * plan.steps.len() as u32
}

#[test]
fn the_steps_add_up_to_exactly_the_distance_asked_for() {
    for (dx, dy) in [(0.0, -300.0), (0.0, 1234.5), (80.0, -40.0), (-7.0, 0.0), (0.0, 0.7)] {
        let plan = plan(dx, dy, None);
        let (sx, sy) = total(&plan);
        assert!((sx - dx).abs() < 1e-3 && (sy - dy).abs() < 1e-3, "({dx}, {dy}) came to ({sx}, {sy})");
    }
}

#[test]
fn every_step_goes_the_same_way_and_the_axes_keep_their_ratio() {
    let plan = plan(100.0, -50.0, None);
    assert!(plan.steps.iter().all(|s| s.0 >= 0.0 && s.1 <= 0.0), "never backwards");
    for (x, y) in &plan.steps {
        assert!((y + x / 2.0).abs() < 1e-3, "({x}, {y}) is off the 2:-1 line");
    }
}

#[test]
fn it_is_many_small_steps_at_a_trackpads_cadence_not_a_few_big_ones() {
    let plan = plan(0.0, -600.0, None);
    assert!(plan.steps.len() >= 20, "{} steps", plan.steps.len());
    assert!(plan.interval <= Duration::from_millis(16), "faster than the classifier's 20 ms line: {:?}", plan.interval);
    let biggest = plan.steps.iter().map(|s| s.1.abs()).fold(0.0, f32::max);
    assert!(biggest < 600.0 / 8.0, "no step is a large share of the whole: {biggest}");
}

#[test]
fn it_speeds_up_and_slows_down_like_a_swipe_that_starts_and_ends_with_the_finger_at_rest() {
    let plan = plan(0.0, 1000.0, None);
    let speeds: Vec<f32> = plan.steps.iter().map(|s| s.1).collect();
    let fastest = speeds.iter().cloned().fold(0.0, f32::max);
    assert!(speeds[0] < fastest / 4.0, "gentle start: {} vs {fastest}", speeds[0]);
    assert!(speeds[speeds.len() - 1] < fastest / 4.0, "gentle end");
    let peak = speeds.iter().position(|s| *s == fastest).unwrap();
    assert!(peak > speeds.len() / 4 && peak < speeds.len() * 3 / 4, "fastest in the middle, not at an end");
}

#[test]
fn the_deltas_vary_so_the_app_does_not_mistake_them_for_wheel_pulses() {
    // The receiving side calls a stream of repeated equal deltas a mouse wheel.
    let plan = plan(0.0, -600.0, None);
    let first = plan.steps[0].1;
    let same = plan.steps.iter().filter(|s| (s.1 - first).abs() <= 0.01).count();
    assert!((same as f32 / plan.steps.len() as f32) < 0.5, "{same} of {} equal", plan.steps.len());
}

#[test]
fn the_time_follows_the_distance_unless_it_is_asked_for_and_is_capped() {
    let short = duration(&plan(0.0, -100.0, None));
    let long = duration(&plan(0.0, -3000.0, None));
    assert!(short < long);
    assert!(short >= Duration::from_millis(300), "never a blink: {short:?}");
    let steady = duration(&plan(0.0, -700.0, None));
    assert!(steady >= Duration::from_millis(800) && steady <= Duration::from_millis(1000), "a deliberate scroll: {steady:?}");
    assert!(long <= Duration::from_millis(2600), "never a crawl: {long:?}");
    let asked = duration(&plan(0.0, -500.0, Some(1000)));
    assert!(asked >= Duration::from_millis(950) && asked <= Duration::from_millis(1050), "{asked:?}");
    let capped = duration(&plan(0.0, -500.0, Some(u32::MAX)));
    assert!(capped <= Duration::from_millis(3050), "{capped:?}");
}

#[test]
fn nothing_to_scroll_is_nothing() {
    assert!(plan(0.0, 0.0, None).steps.is_empty());
    assert!(plan(0.1, -0.2, None).steps.is_empty(), "less than a pixel");
}
