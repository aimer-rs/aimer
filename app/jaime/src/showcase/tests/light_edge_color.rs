use std::cell::Cell;
use std::rc::Rc;

use aimer::canvas::Canvas;
use aimer::quiver::winit::dpi::PhysicalSize;
use aimer::decoration::{BoxDecoration, Decoration, DecorationBox, LightEdge};
use aimer::{HeadlessOptions, SizedBox};

use super::*;

struct RecordedEdge {
    background: Rc<Cell<Color>>,
    recordings: Rc<Cell<usize>>,
}

impl Decoration for RecordedEdge {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.background
            .set(canvas.background_color());
        self.recordings.set(
            self.recordings
                .get()
                + 1,
        );
        LightEdge::new().paint(canvas, origin, size, scale);
    }
}

#[test]
fn light_edge_auto_color_receives_live_surface_changes_during_native_paint() {
    for scale in [1.0, 2.0] {
        let surface = Rc::new(BoxDecoration::new().background_color(Color::WHITE));
        let background = Rc::new(Cell::new(Color::Transparent));
        let recordings = Rc::new(Cell::new(0));
        let widget = Container::new()
            .width(160)
            .height(80)
            .decoration(surface.clone())
            .child(
                DecorationBox::new()
                    .decoration(RecordedEdge {
                        background: background.clone(),
                        recordings: recordings.clone(),
                    })
                    .child(
                        SizedBox::new()
                            .width(160)
                            .height(80),
                    ),
            );
        let mut app = AimerApp::start_headless_with(
            theme::provide(widget),
            HeadlessOptions {
                size: PhysicalSize::new((300.0 * scale) as u32, (200.0 * scale) as u32),
                scale_factor: scale,
            },
        );
        app.pump_frames(3);
        assert_eq!(background.get(), Color::WHITE);
        let first_recordings = recordings.get();
        assert!(first_recordings > 0);
        surface.update_color(Color::BLACK);
        app.render_frame();
        assert_eq!(background.get(), Color::BLACK);
        assert!(
            recordings.get() > first_recordings,
            "the retained rim must re-record after a surface change"
        );
        let settled = recordings.get();
        app.pump_frames(2);
        assert_eq!(
            recordings.get(),
            settled,
            "unchanged frames must reuse the rim"
        );
        let census = app
            .paint_source_census()
            .unwrap();
        assert!(
            census
                .declined_paint
                .is_empty()
        );
        assert!(
            census
                .drawn_unmapped
                .is_empty()
        );
        assert!(
            census
                .invalid_bounds
                .is_empty()
        );
        assert!(
            census
                .sync_error
                .is_none()
        );
    }
}
