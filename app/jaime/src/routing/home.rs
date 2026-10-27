use aimer::console::debug;
use aimer::input::button::Button;
use aimer::macros::StatelessWidget;
use aimer::router::NavigatorController;
use aimer::style::*;
use aimer::*;

use crate::routing::AppRouting;

#[derive(Clone)]
#[derive(StatelessWidget)]
pub struct HomeWidget {}

impl StatelessWidget for HomeWidget {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        debug!("Building HomeWidget");
        let theme = ThemeData::copied(ctx);

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .children(vec![
                        Text::new("Home Page")
                            .text_align(TextAlign::MidCenter)
                            .text_style(TextStyle::new().color(theme.on_background_color))
                            .boxed(),
                        Row::new()
                            .gaps(LayoutSpacing {
                                right: Spacing::Px(10),
                                ..Default::default()
                            })
                            .children(vec![
                                Button::new()
                                    .on_press({
                                        let navi = NavigatorController::<AppRouting>::of(ctx);
                                        move || {
                                            navi.push(AppRouting::Settings);
                                        }
                                    })
                                    .decoration(
                                        BoxDecoration::new().background_color(theme.primary_color),
                                    )
                                    .child(
                                        Container::new()
                                            .width(Dimension::Px(200.0))
                                            .height(Dimension::Px(50.0))
                                            .child(
                                                Text::new("Setting")
                                                    .text_align(TextAlign::MidCenter)
                                                    .text_style(
                                                        TextStyle::new()
                                                            .color(theme.on_primary_color),
                                                    ),
                                            ),
                                    )
                                    .boxed(),
                                Button::new()
                                    .on_press({
                                        let navi = NavigatorController::<AppRouting>::of(ctx);
                                        move || {
                                            navi.push(AppRouting::Profile {
                                                name: "John".to_string(),
                                            });
                                        }
                                    })
                                    .decoration(
                                        BoxDecoration::new().background_color(theme.primary_color),
                                    )
                                    .child(
                                        Container::new()
                                            .width(Dimension::Px(200.0))
                                            .height(Dimension::Px(50.0))
                                            .child(
                                                Text::new("Profile")
                                                    .text_align(TextAlign::MidCenter)
                                                    .text_style(
                                                        TextStyle::new()
                                                            .color(theme.on_primary_color),
                                                    ),
                                            ),
                                    )
                                    .boxed(),
                            ])
                            .boxed(),
                    ]),
            )
    }
}
