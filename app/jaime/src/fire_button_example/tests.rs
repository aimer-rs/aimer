use aimer::effects::{self, EffectCommand};
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{AimerApp, HeadlessOptions};
use aimer_cupid::draw_cmd::{DrawCommand, RenderOp};
use aimer_time::Duration as TimeDuration;

use super::*;

fn payload_words(effect: &Fire) -> Vec<f32> {
    let command = EffectCommand::new(effect, [0.0, 0.0, 10.0, 10.0]).unwrap();
    let (_, params) = effects::decode_payload(&command.payload).unwrap();
    params
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

/// The shader's `Frame` struct and `Fire::write_params` are written apart. If
/// either changes without the other the picture is garbage and no compiler
/// notices; the registry checks the shader against the contract and this checks
/// the Rust side against the shader.
#[test]
fn the_rust_parameters_fill_the_shaders_frame_exactly() {
    effects::register::<Fire>().expect("the shader follows the effect contract");
    let spec = effects::registered()
        .into_iter()
        .find(|spec| spec.name() == Fire::NAME)
        .expect("registered");
    let words = payload_words(&Fire { time: 0.0, level: 1.0 });
    assert_eq!(words.len() * 4, spec.program().params_size());
}

#[test]
fn the_parameters_land_in_the_shaders_fields_in_order() {
    let words = payload_words(&Fire { time: 2.5, level: 0.75 });
    // time, level, corner, _pad0, then the button.
    assert_eq!(&words[..4], [2.5, 0.75, 1.0, 0.0]);
    assert_eq!(&words[4..8], BUTTON_FRACTIONS);
}

#[test]
fn the_button_fractions_describe_a_button_inside_the_box() {
    let [cx, cy, hx, hy] = BUTTON_FRACTIONS;
    assert_eq!(cx * BOX_WIDTH, BUTTON_LEFT + BUTTON_WIDTH / 2.0, "the stack places it where the shader looks");
    assert_eq!(cy * BOX_HEIGHT, BUTTON_TOP + BUTTON_HEIGHT / 2.0);
    assert_eq!((cx, cy), (0.5, 0.5), "the button is centered in the box");
    assert!(cx - hx > 0.0 && cx + hx < 1.0 && cy - hy > 0.0 && cy + hy < 1.0);
}

#[test]
fn a_fire_burns_out_over_the_exhaust_time_and_stops_at_empty() {
    let fuel = Fuel::new();
    assert_eq!(fuel.stage(), Stage::Burning);
    fuel.toggle();
    assert_eq!(fuel.stage(), Stage::Exhausting);

    let half = fuel.advance(EXHAUST_SECONDS / 2.0);
    assert!((half.level - 0.5).abs() < 1e-5, "{half:?}");
    assert!(!fuel.is_out());

    let out = fuel.advance(EXHAUST_SECONDS * 10.0);
    assert_eq!(out.level, 0.0, "fuel never goes below empty");
    assert_eq!(fuel.stage(), Stage::Out);
    assert!(fuel.is_out());
}

#[test]
fn a_press_while_dying_relights_from_where_it_is() {
    let fuel = Fuel::new();
    fuel.toggle();
    fuel.advance(EXHAUST_SECONDS / 2.0);
    fuel.toggle();
    assert_eq!(fuel.stage(), Stage::Burning);
    let fire = fuel.advance(IGNITE_SECONDS / 4.0);
    assert!((fire.level - 0.75).abs() < 1e-5, "continues up from 0.5: {fire:?}");
    assert_eq!(fuel.advance(IGNITE_SECONDS * 10.0).level, 1.0, "and stops at full");
}

#[test]
fn a_cold_fire_relights_from_empty() {
    let fuel = Fuel::new();
    fuel.toggle();
    fuel.advance(EXHAUST_SECONDS * 2.0);
    assert!(fuel.is_out());
    fuel.toggle();
    assert!(!fuel.is_out(), "wanted again, so it is no longer out");
    let fire = fuel.advance(IGNITE_SECONDS / 2.0);
    assert!((fire.level - 0.5).abs() < 1e-5, "{fire:?}");
}

#[test]
fn time_keeps_running_while_the_fire_changes_and_ignores_bad_steps() {
    let fuel = Fuel::new();
    assert_eq!(fuel.advance(1.0).time, 1.0);
    assert_eq!(fuel.advance(-1.0).time, 1.0);
    assert_eq!(fuel.advance(f64::NAN).time, 1.0);
    assert_eq!(fuel.advance(f64::INFINITY).time, 1.0);
    assert_eq!(fuel.advance(0.0).level, 1.0, "a zero step leaves the fuel");
    fuel.toggle();
    assert_eq!(fuel.advance(0.5).time, 1.5);
}

#[test]
fn a_stalled_tick_cannot_leap_forward_and_a_toggle_forgets_the_last_tick() {
    let fuel = Fuel::new();
    let start = Instant::now();
    assert_eq!(fuel.tick(start).time, 0.0, "the first tick has no previous one");
    let leaped = fuel.tick(start + TimeDuration::from_secs(30));
    assert_eq!(leaped.time, MAX_STEP as f32);

    // The clock was stopped for an hour; the first tick after a press adds nothing.
    fuel.toggle();
    let resumed = fuel.tick(start + TimeDuration::from_secs(3600));
    assert_eq!(resumed.time, MAX_STEP as f32);
}

#[test]
fn the_animation_stops_when_the_fire_goes_out_and_a_press_restarts_it() {
    let fuel = Fuel::new();
    let ticker = AnimationController::new(Duration::from_secs(1), curves::LINEAR);
    ticker.set_repeat(true);
    ticker.forward_from_first_tick();

    // Burning: every frame keeps the animation going.
    let start = Instant::now();
    frame(&fuel, &ticker, start);
    assert!(ticker.is_animating());

    press(&fuel, &ticker);
    assert!(ticker.is_animating(), "still burning out");
    let mut now = start;
    for _ in 0..100 {
        now = now + TimeDuration::from_millis(100);
        let fire = frame(&fuel, &ticker, now);
        if fire.level == 0.0 {
            break;
        }
        assert!(ticker.is_animating());
    }
    assert!(fuel.is_out());
    assert!(!ticker.is_animating(), "a cold button asks for no frames");

    press(&fuel, &ticker);
    assert!(ticker.is_animating(), "pressing again lights it and restarts the frames");
    assert_eq!(fuel.stage(), Stage::Burning);
}

fn levels(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> Vec<(f32, [f32; 4], f32)> {
    app.render_ops()
        .iter()
        .filter_map(|op| match op {
            RenderOp::Draw(item) => Some(item.snapshot().commands.to_vec()),
            _ => None,
        })
        .flatten()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { pipeline_name, data } if &*pipeline_name == Fire::NAME => {
                let (rect, params) = effects::decode_payload(&data)?;
                let word = |i: usize| f32::from_le_bytes(params[i * 4..i * 4 + 4].try_into().unwrap());
                Some((word(1), rect, word(0)))
            }
            _ => None,
        })
        .collect()
}

/// The top-left corner of the box the fire is drawn in, on screen.
fn box_origin(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> (f32, f32) {
    app.render_ops()
        .into_iter()
        .find_map(|op| match op {
            RenderOp::Draw(item)
                if item.snapshot().commands.iter().any(|command| {
                    matches!(command, DrawCommand::DrawCustom { pipeline_name, .. }
                        if &**pipeline_name == Fire::NAME)
                }) =>
            {
                Some((item.bounds.x, item.bounds.y))
            }
            _ => None,
        })
        .expect("the fire is on screen")
}

fn mouse(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>, x: f64, y: f64) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(x, y),
    });
    app.render_frame();
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(2);
    }
}

fn run(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>, frames: usize) {
    for _ in 0..frames {
        std::thread::sleep(Duration::from_millis(20));
        aimer::events::window::request_scoped_frame();
        app.pump_frames(2);
    }
}

/// The whole page, mounted: the fire is drawn over the box the page lays out;
/// pressing the button lowers the fuel, and pressing it again raises it.
#[test]
fn pressing_the_button_burns_the_fire_out_and_pressing_again_relights_it() {
    let mut app = AimerApp::start_headless_with(
        theme::provide(fire_button_example()),
        HeadlessOptions {
            size: PhysicalSize::new(900, 900),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let first = levels(&app);
    assert_eq!(first.len(), 1, "one fire on the page");
    let (level, rect, _) = first[0];
    assert_eq!(level, 1.0, "the fire starts at full blaze");
    assert_eq!((rect[2], rect[3]), (BOX_WIDTH, BOX_HEIGHT), "the box is the one the page lays out");

    // The button's center, from the box the fire was drawn in.
    let (left, top) = box_origin(&app);
    assert_eq!(left + BOX_WIDTH / 2.0, 450.0, "the fire, and so the button, is centered in the 900 px window");
    let x = f64::from(left + BUTTON_LEFT + BUTTON_WIDTH / 2.0);
    let y = f64::from(top + BUTTON_TOP + BUTTON_HEIGHT / 2.0);
    mouse(&mut app, x, y);
    run(&mut app, 10);
    let burning_out = levels(&app).last().expect("still drawn").0;
    assert!(burning_out < 1.0, "the press lowered the fuel: {burning_out}");
    run(&mut app, 5);
    let lower = levels(&app).last().expect("still drawn").0;
    assert!(lower < burning_out, "and it keeps falling: {burning_out} -> {lower}");

    mouse(&mut app, x, y);
    run(&mut app, 10);
    let relit = levels(&app).last().expect("still drawn").0;
    assert!(relit > lower, "the second press raises it again: {lower} -> {relit}");
}
