use std::time::Duration;

use aimer::effects::{self, EffectCommand, ShaderEffect};
use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, MouseScrollDelta, TouchPhase, WindowEvent};
use aimer::{AimerApp, HeadlessOptions};
use aimer_cupid::draw_cmd_v2::{DrawCommand, RenderOp};
use aimer_time::{Duration as TimeDuration, Instant};

use super::*;

fn payload_words(effect: &Melt) -> Vec<f32> {
    let command = EffectCommand::new(effect, [0.0, 0.0, 10.0, 10.0]).unwrap();
    let (_, params) = effects::decode_payload(&command.payload).unwrap();
    params
        .chunks_exact(4)
        .map(|word| f32::from_le_bytes(word.try_into().unwrap()))
        .collect()
}

/// The shader's `Frame` struct and `Melt::write_params` are written apart. If
/// either changes without the other, the picture is garbage and no compiler
/// notices; the registry checks the shader against the contract and this checks
/// the Rust side against the shader.
#[test]
fn the_rust_parameters_fill_the_shaders_frame_exactly() {
    effects::register::<Melt>().expect("the shader follows the effect contract");
    let spec = effects::registered()
        .into_iter()
        .find(|spec| spec.name() == Melt::NAME)
        .expect("registered");
    let words = payload_words(&Props::DEFAULT.effect(0.0));
    assert_eq!(words.len() * 4, spec.program().params_size());
}

#[test]
fn the_controls_land_in_the_shaders_fields_in_order() {
    let props = Props {
        speed: 1.0,
        scale: 2.0,
        melt: 0.75,
        contrast: 4.0,
        radius: 0.25,
        grain: 0.5,
        palette: 2,
    };
    let words = payload_words(&props.effect(3.5));
    // time, scale, melt, contrast, radius, grain, two pad words.
    assert_eq!(&words[..6], [3.5, 2.0, 0.75, 4.0, 0.25, 0.5]);
    assert_eq!(&words[6..8], [0.0, 0.0]);
    assert_eq!(&words[8..12], PALETTES[2].colors[0]);
    assert_eq!(&words[20..24], PALETTES[2].colors[3]);
}

#[test]
fn out_of_range_values_are_clamped_and_nan_falls_back() {
    let wild = Props {
        speed: 99.0,
        scale: -1.0,
        melt: f32::INFINITY,
        contrast: 0.0,
        radius: 7.0,
        grain: f32::NAN,
        palette: 99,
    }
    .clamped();
    assert_eq!(wild.speed, SPEED.1);
    assert_eq!(wild.scale, SCALE.0);
    assert_eq!(wild.melt, Props::DEFAULT.melt, "infinity is not a number to trust");
    assert_eq!(wild.contrast, CONTRAST.0);
    assert_eq!(wild.radius, RADIUS.1);
    assert_eq!(wild.grain, Props::DEFAULT.grain);
    assert_eq!(wild.palette, PALETTES.len() - 1);

    assert_eq!(Props::DEFAULT.clamped(), Props::DEFAULT, "defaults are in range");
}

#[test]
fn time_is_the_integral_of_speed() {
    let clock = MeltClock::new();
    assert_eq!(clock.advance(1.0, 2.0), 2.0);
    // Halving the speed continues from 2.0, it does not rescale the past.
    assert_eq!(clock.advance(2.0, 0.5), 3.0);
    assert_eq!(clock.advance(5.0, 0.0), 3.0, "speed 0 freezes the picture");
}

#[test]
fn a_stalled_tick_cannot_leap_forward() {
    let clock = MeltClock::new();
    let start = Instant::now();
    assert_eq!(clock.tick(start, 1.0), 0.0, "the first tick has no previous one");
    let time = clock.tick(start + TimeDuration::from_secs(30), 1.0);
    assert_eq!(time, MAX_STEP as f32);
}

#[test]
fn negative_or_non_finite_steps_add_nothing() {
    let clock = MeltClock::new();
    assert_eq!(clock.advance(-1.0, 1.0), 0.0);
    assert_eq!(clock.advance(f64::NAN, 1.0), 0.0);
    assert_eq!(clock.advance(0.5, f32::NAN), 0.0);
}

#[test]
fn every_palette_is_four_opaque_srgb_colors() {
    let mut names = std::collections::HashSet::new();
    for palette in &PALETTES {
        assert!(names.insert(palette.name), "palette names are unique");
        for color in palette.colors {
            assert!(color[..3].iter().all(|channel| (0.0..=1.0).contains(channel)));
            assert_eq!(color[3], 1.0);
        }
    }
}

fn melt_payloads(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> Vec<Vec<u8>> {
    app.render_ops()
        .iter()
        .filter_map(|op| match op {
            RenderOp::Draw(item) => Some(item.snapshot().commands.to_vec()),
            _ => None,
        })
        .flatten()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { pipeline_name, data } if &*pipeline_name == Melt::NAME => {
                Some(data.to_vec())
            }
            _ => None,
        })
        .collect()
}

/// The whole page, mounted: the effect is drawn over the box the page lays out,
/// and its time moves from one frame to the next.
#[test]
fn the_page_draws_the_effect_and_its_time_advances() {
    let mut app = AimerApp::start_headless_with(
        theme::provide(color_melt_example()),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 900),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(4);
    let first = melt_payloads(&app);
    assert_eq!(first.len(), 1, "one effect on the page");
    let (rect, _) = effects::decode_payload(&first[0]).unwrap();
    assert_eq!((rect[0], rect[1]), (0.0, 0.0), "the rectangle is the painted box");
    assert_eq!(rect[3], 360.0, "the box is as tall as the page asks");
    assert!(rect[2] > 400.0, "and fills the width it is given: {rect:?}");

    let time_of = |payload: &Vec<u8>| {
        let (_, params) = effects::decode_payload(payload).unwrap();
        f32::from_le_bytes(params[..4].try_into().unwrap())
    };
    let before = time_of(&first[0]);
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(20));
        aimer::events::window::request_scoped_frame();
        app.pump_frames(2);
    }
    let after = time_of(melt_payloads(&app).last().expect("still drawn"));
    assert!(after > before, "time moved from {before} to {after}");
}

/// The top edge of the box the effect is drawn in, `None` when it is off screen.
fn picture_top(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> Option<f32> {
    app.render_ops().into_iter().find_map(|op| match op {
        RenderOp::Draw(item)
            if item.snapshot().commands.iter().any(|command| {
                matches!(command, DrawCommand::DrawCustom { pipeline_name, .. }
                    if &**pipeline_name == Melt::NAME)
            }) =>
        {
            Some(item.bounds.y)
        }
        _ => None,
    })
}

/// The controls sit below the picture, so in a short window the page is taller
/// than the window and must scroll to reach them.
#[test]
fn the_page_scrolls_with_the_wheel_in_a_short_window() {
    let mut app = AimerApp::start_headless_with(
        theme::provide(color_melt_example()),
        HeadlessOptions {
            size: PhysicalSize::new(900, 500),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    let before = picture_top(&app).expect("the picture is on screen");

    // Over the picture, which takes no pointer events of its own.
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(450.0, 300.0),
    });
    app.render_frame();
    for _ in 0..4 {
        app.send_window_event(WindowEvent::MouseWheel {
            device_id: DeviceId::dummy(),
            delta: MouseScrollDelta::PixelDelta(PhysicalPosition::new(0.0, -60.0)),
            phase: TouchPhase::Moved,
        });
        app.render_frame();
    }
    app.pump_frames(20);
    let after = picture_top(&app).map_or(f32::NEG_INFINITY, |top| top);
    assert!(after < before, "the picture moves up as the page scrolls: {before} -> {after}");
}
