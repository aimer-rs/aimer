use aimer::animation::Animatable;
use aimer::style::{AnimatedTheme, TextStyle, Theme, ThemeData};
use aimer::{
    AimerApp, BoxAlignment, BuildContext, Color, Column, Container, Dimension, SizedBox,
    StatelessWidget, Text, Widget,
};

#[derive(Clone, Debug, PartialEq, Animatable)]
struct PanelStyle {
    width: f32,
    accent: Color,
}

#[derive(Clone, Debug, PartialEq, Animatable)]
struct Point(f32, f32);

#[derive(Debug, PartialEq, Animatable)]
#[animatable(discrete)]
enum LoadingState {
    Idle,
    Message(&'static str),
}

#[derive(Debug, PartialEq, Animatable)]
#[animatable(fieldwise)]
enum Geometry {
    Point(Point),
    Hidden,
}

/// Starts the derived-value example under the application's normal theme
/// provider. W17 owns registering this entry point in the shared showcase.
pub fn start_animatable_example() {
    AimerApp::start(
        AnimatedTheme::new()
            .data(ThemeData::light())
            .child(AnimatableExample),
    )
}

/// Builds the derived-value example without starting an application.
pub fn animatable_example() -> impl Widget {
    AnimatableExample
}

#[derive(StatelessWidget)]
struct AnimatableExample;

impl StatelessWidget for AnimatableExample {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let theme = ThemeData::of(ctx);
        let style = PanelStyle {
            width: 180.0,
            accent: Color::Rgba(20, 20, 20, 255),
        }
        .lerp(
            &PanelStyle {
                width: 340.0,
                accent: Color::Rgba(220, 220, 220, 255),
            },
            0.5,
        );
        let point = Point(0.0, 20.0).lerp(&Point(120.0, 80.0), 0.5);
        let state = LoadingState::Idle.lerp(&LoadingState::Message("Ready"), 0.75);
        let geometry =
            Geometry::Point(Point(0.0, 10.0)).lerp(&Geometry::Point(Point(100.0, 30.0)), 0.5);

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children([
                        Text::new("Derived Animatable values")
                            .text_style(
                                TextStyle::new()
                                    .font_size(26)
                                    .color(theme.on_background_color),
                            )
                            .boxed(),
                        SizedBox::new()
                            .height(20)
                            .boxed(),
                        Container::new()
                            .width(Dimension::Px(style.width))
                            .height(Dimension::Px(56.0))
                            .color(style.accent)
                            .child(Text::new(format!(
                                "point ({:.0}, {:.0}) · {state:?} · {geometry:?}",
                                point.0, point.1,
                            )))
                            .boxed(),
                        SizedBox::new()
                            .height(20)
                            .boxed(),
                        Text::new(
                            "String, bool, and Option<T> fields in structs need a manual \
                             Animatable implementation; custom enum mappings are also manual.",
                        )
                        .text_style(
                            TextStyle::new()
                                .font_size(14)
                                .color(theme.on_background_color),
                        )
                        .boxed(),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_covers_struct_tuple_discrete_and_fieldwise_values() {
        assert_eq!(
            Point(0.0, 10.0).lerp(&Point(10.0, 30.0), 0.5),
            Point(5.0, 20.0)
        );
        assert_eq!(
            LoadingState::Idle.lerp(&LoadingState::Message("ready"), 0.5),
            LoadingState::Message("ready")
        );
        assert_eq!(
            Geometry::Point(Point(0.0, 4.0)).lerp(&Geometry::Point(Point(8.0, 12.0)), 0.25),
            Geometry::Point(Point(2.0, 6.0))
        );
        assert_eq!(
            Geometry::Hidden.lerp(&Geometry::Hidden, 0.5),
            Geometry::Hidden
        );
    }
}
