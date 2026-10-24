use std::sync::Arc;

use aimer::shape::{
    FillStyle, ShapeClip, ShapeColor, ShapeFit, ShapeHitTest, ShapePath, StrokeStyle,
};
use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::style::{LayoutSpacing, Spacing, TextStyle};
use aimer::{
    AimerApp, Colors, Column, Container, CustomShape, Text, Widget,
};

/// Builds a finite path with lines, a quadratic curve, a cubic curve, and a
/// closed contour. The pure geometry builder performs all finite and bounded
/// validation before this example can hand the path to a widget.
pub fn demo_shape_path() -> ShapePath {
    ShapePath::builder()
        .move_to(28.0, 188.0)
        .line_to(84.0, 36.0)
        .quadratic_to(142.0, 116.0, 194.0, 42.0)
        .cubic_to(232.0, 0.0, 270.0, 94.0, 332.0, 34.0)
        .line_to(304.0, 204.0)
        .line_to(72.0, 220.0)
        .close()
        .build()
        .expect("the bounded custom-shape example path is finite")
}

/// Returns the immutable path handle used by the animated page.
///
/// `CustomShape::shared_path` retains this allocation across animation
/// rebuilds, so the renderer can reuse the path identity instead of cloning
/// every command on each frame.
pub fn shared_demo_shape_path() -> Arc<ShapePath> {
    Arc::new(demo_shape_path())
}

/// Demonstrates the safe invalid-geometry path used by the example tests and
/// by callers that load shape data at runtime.
pub fn invalid_shape_is_rejected() -> bool {
    ShapePath::builder()
        .move_to(f32::NAN, 0.0)
        .line_to(1.0, 1.0)
        .build()
        .is_err()
}

/// Builds the animated custom-shape page without starting the application.
pub fn custom_shape_example() -> impl Widget {
    let path = shared_demo_shape_path();
    let path_id = path.id().0;
    let invalid_geometry_rejected = invalid_shape_is_rejected();
    let controller = AnimationController::with_millis(1_600, curves::EASE_IN_OUT);
    controller.forward_from_first_tick();

    AnimatedBuilder::new(controller, move |progress| {
        let opacity = 0.35 + progress.clamp(0.0, 1.0) * 0.65;
        let fill = FillStyle::solid(ShapeColor::rgba8(55, 115, 220, 220));
        let stroke = StrokeStyle::new(4.0, ShapeColor::rgba8(16, 36, 84, 255))
            .expect("the example stroke is finite")
            .with_line_cap(aimer::shape::LineCap::Round)
            .with_line_join(aimer::shape::LineJoin::Round);

        CustomShape::new()
            .shared_path(path.clone())
            .fill(fill)
            .stroke(stroke)
            .clip(ShapeClip::Bounds)
            .fit(ShapeFit::Contain)
            .hit_test(ShapeHitTest::FillOrStroke)
            .opacity(opacity)
            .child(
                Container::new()
                    .width(360.0)
                    .height(250.0)
                    .padding(LayoutSpacing::all(Spacing::Px(22)))
                    .child(Column::new().children(vec![
                            Text::new("CustomShape")
                                .text_style(
                                    TextStyle::new()
                                        .font_size(26)
                                        .color(Colors::Black),
                                )
                                .boxed(),
                            Text::new(
                                "Finite curves, fill, stroke, clip, and hit-test metadata",
                            )
                                .text_style(TextStyle::new().color(Colors::Black))
                                .wrapped()
                                .boxed(),
                            Text::new(format!(
                                "Contain · Bounds clip · FillOrStroke · opacity {:.0}%",
                                opacity * 100.0,
                            ))
                            .text_style(TextStyle::new().color(Colors::Black))
                            .wrapped()
                            .boxed(),
                            Text::new(format!(
                                "path cache id: {path_id:016x} · invalid geometry: {}",
                                if invalid_geometry_rejected {
                                    "rejected safely"
                                } else {
                                    "unexpectedly accepted"
                                },
                            ))
                            .text_style(TextStyle::new().color(Colors::Black))
                            .wrapped()
                            .boxed(),
                        ])),
            )
    })
}

/// Starts the standalone custom-shape example.
pub fn start_custom_shape_example() {
    AimerApp::start(crate::theme::provide(custom_shape_example()));
}
