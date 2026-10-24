// use aimer::flex::{Column, };
use aimer::console::debug;
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::router::NavigatorController;
use aimer::style::*;
use aimer::*;

use crate::routing::AppRouting;

#[derive(Clone)]
#[widget(Stateless)]
pub struct SettingPage {}

impl StatelessWidget for SettingPage {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        debug!("Building SettingPage");
        let theme = ThemeData::copied(ctx);

        Container::new()
            .color(theme.background_color)
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Center)
                    .vertical_alignment(BoxAlignment::Center)
                    .gaps(LayoutSpacing {
                        top: Spacing::Px(40),
                        ..Default::default()
                    })
                    .children(vec![
                        Text::new("Setting Page")
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
                                            navi.pop();
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
                                                Text::new("Back")
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
                                                name: "Javier".into(),
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
                                                Text::new("Profile Page")
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
