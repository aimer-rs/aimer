use super::*;

const DT: f32 = 1.0 / 60.0;

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// Runs the motion at 60 Hz until it stops, returning every position it moved
/// to and how many seconds that took.
fn run(motion: &mut Motion) -> (Vec<(f32, f32)>, f32) {
    let mut path = Vec::new();
    while let Some(at) = motion.step(DT) {
        path.push(at);
        assert!(path.len() < 1200, "it never arrived");
    }
    let seconds = path.len() as f32 * DT;
    (path, seconds)
}

fn placed(at: (f32, f32)) -> Motion {
    let mut motion = Motion::default();
    motion.place(Some(at));
    motion
}

fn gliding(from: (f32, f32), to: (f32, f32), millis: Option<u32>) -> Motion {
    let mut motion = placed(from);
    assert_eq!(motion.glide(to, millis), None, "a glide from a known place is a motion, not a move");
    motion
}

#[test]
fn with_nowhere_to_glide_from_the_pointer_just_arrives() {
    let mut motion = Motion::default();
    assert_eq!(motion.glide((100.0, 50.0), None), Some((100.0, 50.0)), "the caller emits this one move");
    assert!(!motion.is_moving());
    assert_eq!(motion.position(), Some((100.0, 50.0)));
}

#[test]
fn a_pointer_that_is_already_there_does_not_move() {
    let mut motion = placed((5.0, 5.0));
    assert_eq!(motion.glide((5.0, 5.0), Some(500)), None);
    assert!(!motion.is_moving());
    assert_eq!(motion.step(DT), None);
}

#[test]
fn a_glide_arrives_exactly_on_the_target_and_then_stops() {
    let mut motion = gliding((0.0, 0.0), (400.0, 0.0), None);
    assert!(motion.is_moving());
    let (path, _) = run(&mut motion);
    assert_eq!(path.last(), Some(&(400.0, 0.0)), "the last step is the target itself");
    assert!(!motion.is_moving());
    assert_eq!(motion.position(), Some((400.0, 0.0)));
    assert_eq!(motion.step(DT), None, "and nothing more is emitted");
}

#[test]
fn undisturbed_it_follows_the_straight_line_and_never_goes_backwards() {
    let mut motion = gliding((0.0, 0.0), (600.0, 300.0), None);
    let (path, _) = run(&mut motion);
    for (x, y) in &path {
        assert!((y - x / 2.0).abs() < 0.6, "({x}, {y}) is off the line");
    }
    assert!(path.windows(2).all(|w| w[0].0 <= w[1].0), "never backwards");
}

#[test]
fn it_eases_out_of_the_start_and_into_the_end() {
    let mut motion = gliding((0.0, 0.0), (800.0, 0.0), None);
    let (path, _) = run(&mut motion);
    let steps: Vec<f32> = std::iter::once(0.0).chain(path.iter().map(|p| p.0)).collect::<Vec<_>>().windows(2).map(|w| w[1] - w[0]).collect();
    let fastest = steps.iter().cloned().fold(0.0, f32::max);
    assert!(steps[0] < fastest / 2.0, "slow out of the start: {} vs {fastest}", steps[0]);
    assert!(*steps.last().unwrap() < fastest / 2.0, "slow into the end");
}

#[test]
fn the_time_follows_the_distance_unless_it_is_asked_for() {
    let time = |d: f32, millis| run(&mut gliding((0.0, 0.0), (d, 0.0), millis)).1;
    let (near, far) = (time(30.0, None), time(2000.0, None));
    assert!(near < far);
    assert!(near >= 0.08, "never a blink: {near}");
    assert!(far <= 1.3, "never a crawl: {far}");
    let asked = time(1000.0, Some(1000));
    assert!((0.7..=1.5).contains(&asked), "about the time asked for: {asked}");
    let quick = time(1000.0, Some(300));
    assert!(quick < asked, "{quick} {asked}");
}

#[test]
fn a_requested_time_is_capped_so_the_client_does_not_give_up_first() {
    let capped = run(&mut gliding((0.0, 0.0), (1000.0, 0.0), Some(u32::MAX))).1;
    assert!(capped <= (MAX_MILLIS as f32 / 1000.0) * 1.6, "{capped}");
}

#[test]
fn a_second_target_mid_flight_is_turned_toward_at_a_limited_rate() {
    let mut motion = gliding((0.0, 0.0), (1200.0, 0.0), Some(900));
    for _ in 0..25 {
        motion.step(DT);
    }
    // A right-angle turn, at speed.
    motion.glide((motion.position().unwrap().0 + 20.0, 600.0), Some(900));
    let mut path = vec![motion.position().unwrap()];
    while let Some(at) = motion.step(DT) {
        path.push(at);
        assert!(path.len() < 600);
    }
    let headings: Vec<f32> = path.windows(2).filter(|w| dist(w[0], w[1]) > 1.0).map(|w| (w[1].1 - w[0].1).atan2(w[1].0 - w[0].0)).collect();
    let mut turned = 0.0f32;
    for pair in headings.windows(2) {
        let change = (pair[1] - pair[0]).abs();
        assert!(change <= MAX_TURN * DT + 1e-2, "turned {change} rad in one step");
        turned += change;
    }
    assert!(turned > 1.0, "it did turn, through an arc: {turned}");
    assert_eq!(path.last().map(|p| (p.0.round(), p.1.round())), Some((motion.position().unwrap().0.round(), 600.0)));
}

#[test]
fn a_target_behind_it_is_not_a_reversal_on_the_spot() {
    let mut motion = gliding((0.0, 0.0), (1200.0, 0.0), Some(900));
    for _ in 0..25 {
        motion.step(DT);
    }
    let at = motion.position().unwrap();
    motion.glide((at.0 - 300.0, 0.0), Some(900));
    let mut furthest = at.0;
    while let Some(p) = motion.step(DT) {
        furthest = furthest.max(p.0);
    }
    assert!(furthest > at.0 + 5.0, "it carries on a little before coming round: {furthest} vs {}", at.0);
    assert_eq!(motion.position(), Some((at.0 - 300.0, 0.0)));
}

#[test]
fn the_velocity_never_jumps_not_even_when_the_target_changes() {
    let mut motion = gliding((0.0, 0.0), (1000.0, 0.0), Some(800));
    let mut last = (0.0, 0.0);
    let mut moved = 0;
    while let Some(_) = motion.step(DT) {
        moved += 1;
        if moved == 20 {
            let at = motion.position().unwrap();
            motion.glide((at.0, 700.0), Some(800));
        }
        let v = motion.velocity();
        let dv = dist(v, last);
        let bound = DECEL.max(ACCEL) * DT + dist(v, (0.0, 0.0)).max(dist(last, (0.0, 0.0))) * MAX_TURN * DT + 1.0;
        assert!(dv <= bound, "the velocity jumped by {dv} (bound {bound}) at step {moved}");
        last = v;
        assert!(moved < 600);
    }
}

#[test]
fn placing_the_pointer_cancels_any_glide() {
    let mut motion = gliding((0.0, 0.0), (500.0, 0.0), None);
    motion.step(DT);
    motion.place(Some((10.0, 10.0)));
    assert!(!motion.is_moving());
    assert_eq!(motion.position(), Some((10.0, 10.0)));
    motion.place(None);
    assert_eq!(motion.position(), None);
}


#[test]
fn the_braking_curve_never_asks_for_more_than_the_pointer_can_do() {
    // Between two close distances the speed may fall by no more than DECEL allows.
    let mut d = 0.5;
    while d < 3000.0 {
        let next = d * 1.01;
        let (fast, slow) = (brake_speed(next), brake_speed(d));
        // Falling from `fast` to `slow` over the gap takes (next-d)/mean-speed seconds.
        let seconds = (next - d) / ((fast + slow) / 2.0);
        assert!((fast - slow) / seconds <= DECEL * 1.02, "at {d}: {} px/s²", (fast - slow) / seconds);
        d = next;
    }
    assert!(brake_speed(0.0) == 0.0 && brake_speed(0.4) < 10.0, "it settles into the target");
}
