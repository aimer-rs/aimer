use aimer::animation::{AnimatedBuilder, AnimationController, curves};
use aimer::style::{BoxDecoration, BoxFit, BoxShadow, LayoutSpacing, Spacing};
use aimer::{
    AimerApp, AssetImage, Color, Colors, Container, Dimension, Svg, SvgDocument, SvgStyle,
    SvgTransform,
};

const CAT_VIEWBOX_SIZE: f32 = 1024.0;
const ENTRANCE_START_SCALE: f32 = 0.82;

#[derive(Clone, Copy, Debug, PartialEq)]
struct DrawingEntrance {
    opacity: f32,
    scale: f32,
}

impl DrawingEntrance {
    fn transform(self) -> SvgTransform {
        let offset = CAT_VIEWBOX_SIZE * (1.0 - self.scale) * 0.5;
        SvgTransform {
            sx: self.scale,
            sy: self.scale,
            tx: offset,
            ty: offset,
            ..SvgTransform::default()
        }
    }
}

fn drawing_entrance(progress: f32) -> DrawingEntrance {
    let progress = progress.clamp(0.0, 1.0);
    let eased_scale = 1.0 - (1.0 - progress).powi(2);
    DrawingEntrance {
        opacity: progress,
        scale: ENTRANCE_START_SCALE + (1.0 - ENTRANCE_START_SCALE) * eased_scale,
    }
}

#[allow(unused)]
fn test_image() {
    AimerApp::start(
        Container::new()
            .padding(LayoutSpacing::all(Spacing::Percent(15)))
            .box_decoration(BoxDecoration::new().background_color(Colors::Black))
            .child(
                Container::new()
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(Color::Rgb(41, 31, 31))
                            .border_radius((55, 0, 55, 0))
                            .box_shadow(vec![
                                BoxShadow::new()
                                    .color(Colors::Gray.alpha(200))
                                    .blur(12.0)
                                    .spread(10.0)
                                    .offset_x(40.0)
                                    .offset_y(40.0),
                            ]),
                    )
                    .padding(LayoutSpacing::all(Spacing::Px(10)))
                    .child(
                        AssetImage::new("assets/my_image.png")
                            .fit(BoxFit::FitWidth)
                            .scale(1.1_f32),
                    ),
            ),
    )
}

/// Builds the animated SVG showcase without starting an application.
pub fn svg_example() -> impl aimer::Widget {
    let document = SvgDocument::from_svg(include_bytes!("../assets/cat-svgrepo-com.svg"))
        .expect("the bundled cat SVG should be valid");
    let controller = AnimationController::with_millis(1_200, curves::LINEAR);
    controller.forward_from_first_tick();

    AnimatedBuilder::new(controller, move |progress| {
        let entrance = drawing_entrance(progress);
        Svg::new(document.clone())
            .style(
                "path",
                SvgStyle::new()
                    .opacity(entrance.opacity)
                    .transform(entrance.transform()),
            )
            .width(Dimension::Px(320.0 * 2.0))
            .height(Dimension::Px(320.0 * 2.0))
    })
}

pub fn start_svg_test() {
    AimerApp::start(svg_example())
}

#[cfg(test)]
mod tests {
    use super::drawing_entrance;

    #[test]
    fn drawing_entrance_clamps_and_finishes_at_the_identity_transform() {
        let hidden = drawing_entrance(-1.0);
        assert_eq!(hidden.opacity, 0.0);
        assert_eq!(hidden.scale, 0.82);

        let midpoint = drawing_entrance(0.5);
        assert_eq!(midpoint.opacity, 0.5);
        assert!((midpoint.scale - 0.955).abs() < f32::EPSILON);

        let visible = drawing_entrance(2.0);
        assert_eq!(visible.opacity, 1.0);
        assert_eq!(visible.scale, 1.0);
    }
}
