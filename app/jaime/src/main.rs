#![allow(dead_code, clippy::main_recursion)]

mod accessibility_example;
mod align_example;
mod animatable_example;
mod animated;
mod animated_layout_example;
pub mod animated_text_field_example;
mod animated_theme;
mod assets_media_example;
mod async_builder;
mod backdrop_filter_example;
mod collapsible_list_example;
mod color_sync;
mod container_transform_example;
mod custom_animated_theme;
mod custom_font;
mod custom_shape_example;
pub mod custom_text_field_caret_example;
mod data_view_example;
mod dnd_completion_example;
mod drag_and_drop;
mod feedback_example;
pub mod file_drop_zone;
mod floating;
mod focus_node_example;
mod foreground_decoration_example;
mod form_example;
mod glass_liquid_example;
mod gradient_example;
mod http_request_button;
mod i18n_example;
mod justify_content_example;
mod light_edge_example;
mod liquid_tab_bar_example;
mod loading_animation;
mod lucide_icons_example;
mod magnification_example;
mod markdown_example;
mod modal;
mod morph_transition_example;
mod multi_decoration_box_example;
mod navigation_example;
mod navigation_transitions_example;
mod overflow_behavior_example;
mod panic_recovery;
mod passkey_example;
mod picker_example;
mod range_controls_example;
mod resizable_example;
pub mod routing;
mod routing_context_example;
mod selectable_text;
mod selection_controls_example;
mod showcase;
mod stack_positioned_example;
mod starter;
pub mod stateful;
mod stateful_2;
mod storage_example;
mod style_tokens_example;
mod svg_example;
mod svg_test;
mod system_theme;
mod test_animation;
pub mod text_area_example;
pub mod text_field_example;
mod text_properties_example;
mod theme;
mod tween_keyframe_example;
mod window_example;

use aimer::input::TextEditingController;
use aimer::input::input::{InputType, TextField};
use aimer::native::macos_windowing::MacosWindowing;
use aimer::share::{ShareRef, Shared};
#[allow(unused_imports)]
use aimer::style::*;
#[allow(unused_imports)]
use aimer::*;
use std::cell::UnsafeCell;

// this is the entry point of the app
#[aimer::main]
fn main() {
    AimerApp::new()
        .render_mode(render_mode_from_args())
        .setup(|| {
            MacosWindowing::new()
                .titlebar_transparent(true)
                .title_hidden(true)
                .fullsize_content_view(true)
                .traffic_light_position(16.0, 14.0)
                .has_shadow(true)
                .accepts_first_mouse(true)
                .install();
        })
        .child(
            AnimatedTheme::new()
                .data(theme::app_theme())
                .child(showcase::ExampleShowcase::new()),
        )
        .run();
}

#[allow(unused)]
fn test_text() {
    AimerApp::start(
        Scrollable::new()
            .vertical_scroll_bar(None)
            .axis(ScrollAxis::Vertical)
            .child(Container::new()
                .padding(LayoutSpacing::all(12).top(50))
                .child(Text::new(
                    r#"
你好吗
English — Hello / Hi               Khmer — សួស្តី (Suosdei)               French — BonjourEnglish — Hello / Hi
Spanish — Hola                            Portuguese — Olá                          Italian — Ciao
German — Hallo                            Dutch — Hallo                             Swedish — Hej
Norwegian — Hei                           Danish — Hej                              Finnish — Hei
Icelandic — Halló                         Russian — Привет (Privet)                 Ukrainian — Привіт (Pryvit)
Polish — Cześć                            Czech — Ahoj                              Slovak — Ahoj
Hungarian — Szia                          Romanian — Salut                          Greek — Γεια σου (Yia sou)
Turkish — Merhaba                         Arabic — مرحبا (Marhaban)                 Hebrew — שלום (Shalom)
Persian — سلام (Salam)                    Hindi — नमस्ते (Namaste)                  Bengali — হ্যালো / নমস্কার
Punjabi — ਸਤ ਸ੍ਰੀ ਅਕਾਲ                    Urdu — السلام علیکم                       Tamil — வணக்கம்
Telugu — నమస్తే                           Kannada — ನಮಸ್ಕಾರ                         Malayalam — നമസ്കാരം
Thai — สวัสดี                             Lao — ສະບາຍດີ                             Vietnamese — Xin chào
Indonesian — Halo                         Malay — Hai / Halo                        Filipino — Kumusta
Chinese (Mandarin) — 你好 (Nǐ hǎo)          Cantonese — 你好 (Néih hóu)                 Japanese — こんにちは (Konnichiwa)
Korean — 안녕하세요 (Annyeonghaseyo)           Mongolian — Сайн байна уу                 Swahili — Jambo
Zulu — Sawubona                           Afrikaans — Hallo                         Esperanto — Saluton
Latin — Salve                             Hawaiian — Aloha                          Māori — Kia ora
អរគុណ 你哈皮  With State 你好 きみなと  👉
"#
                )
                    .text_style(TextStyle::new()
                        .text_overflow(TextOverflow::Clip)
                        .font_size(16)
                        .color(Colors::White)
                        .font_weight(FontWeight::Thin))
                )
            )
    )
}

#[allow(unused)]
fn test_positioned() {
    AimerApp::start(
        Container::new()
            .color(Color::WHITE)
            .child(
                Stack::new().children([
                    Positioned::new()
                        .top(80.0)
                        .left(80.0)
                        .child(
                            Container::new()
                                .box_decoration(
                                    BoxDecoration::new()
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .stroke(Stroke::Px(30.0))
                                                .color(Colors::Black),
                                        ))
                                        .outline(BoxOutline::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .stroke(Stroke::Px(3.0))
                                                .color(Colors::Black),
                                        ))
                                        .border_radius((55, 6, 25, 6))
                                        .background_color(Colors::Red)
                                        .box_shadow(vec![
                                            BoxShadow::new()
                                                .color(Colors::Black.alpha(120))
                                                .blur(10.0)
                                                .inset(true),
                                        ]),
                                )
                                .width(Dimension::Px(400.0))
                                .height(Dimension::Px(400.0))
                                .child(
                                    Text::new("Hello, World!")
                                        .text_style(TextStyle::new().color(Colors::Black)),
                                ),
                        )
                        .boxed(),
                    Positioned::new()
                        .top(280.0)
                        .left(180.0)
                        .child(
                            Container::new()
                                .box_decoration(
                                    BoxDecoration::new()
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .stroke(Stroke::Px(30.0))
                                                .color(Colors::Black),
                                        ))
                                        .outline(BoxOutline::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .stroke(Stroke::Px(3.0))
                                                .color(Colors::Black),
                                        ))
                                        .border_radius((55, 6, 25, 6))
                                        .background_color(Colors::Red)
                                        .box_shadow(vec![
                                            BoxShadow::new()
                                                .color(Colors::Black.alpha(120))
                                                .blur(10.0)
                                                .inset(true),
                                        ]),
                                )
                                .width(Dimension::Px(400.0))
                                .height(Dimension::Px(400.0))
                                .child(
                                    Text::new("Hello, World!")
                                        .text_style(TextStyle::new().color(Colors::Black)),
                                ),
                        )
                        .boxed(),
                ]),
            ),
    )
}

#[allow(unused)]
fn test_border_outline() {
    AimerApp::start(
        Container::new()
            .padding(LayoutSpacing::all(Spacing::Px(50)))
            .child(
                Container::new().child(
                    Container::new()
                        .padding(LayoutSpacing::all(Spacing::Px(10)))
                        .child(
                            TextField::new()
                                .padding(LayoutSpacing::all(Spacing::Px(10)))
                                .controller(TextEditingController::new())
                                .text_align(TextAlign::MidLeft)
                                .input_type(InputType::Text)
                                .prompt("Input any here....")
                                .decoration(
                                    BoxDecoration::new()
                                        .background_color(Colors::Gray.alpha(140))
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Black)
                                                .stroke(2),
                                        ))
                                        .outline(BoxOutline::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Black)
                                                .stroke(2),
                                        )),
                                )
                                .hover_decoration(
                                    BoxDecoration::new()
                                        .background_color(Colors::Gray.alpha(70))
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Black)
                                                .stroke(2),
                                        ))
                                        .outline(BoxOutline::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Green)
                                                .stroke(2),
                                        )),
                                )
                                .focus_decoration(
                                    BoxDecoration::new()
                                        .background_color(Colors::Gray.alpha(100))
                                        .border(BoxBorder::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Green)
                                                .stroke(2),
                                        ))
                                        .outline(BoxOutline::all(
                                            BorderSlice::new()
                                                .style(BorderStyle::Solid)
                                                .color(Colors::Black)
                                                .stroke(2),
                                        )),
                                ),
                        ),
                ),
            ),
    )
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

#[cfg(all(test, not(feature = "portable-guest")))]
mod text_editing_example_tests {
    use aimer::{AimerApp, Widget};
    use std::any::{Any, TypeId};

    use crate::animated_text_field_example::AnimatedTextFieldExample;
    use crate::custom_text_field_caret_example::CustomTextFieldCaretExample;
    use crate::text_area_example::TextAreaExample;
    use crate::text_field_example::TextFieldExample;

    #[test]
    fn text_field_example_is_constructible_as_a_widget() {
        let example = TextFieldExample::new();

        assert_eq!(Widget::debug_name(&example), "TextFieldExample");
    }

    #[test]
    fn text_area_example_is_constructible_as_a_widget() {
        let example = TextAreaExample::new();

        assert_eq!(Widget::debug_name(&example), "TextAreaExample");
    }

    #[test]
    fn custom_text_field_caret_example_is_constructible_as_a_widget() {
        let example = CustomTextFieldCaretExample::new();

        assert_eq!(Widget::debug_name(&example), "CustomTextFieldCaretExample");
    }

    #[test]
    fn animated_text_field_example_is_constructible_as_a_widget() {
        let example = AnimatedTextFieldExample::new();

        assert_eq!(Widget::debug_name(&example), "AnimatedTextFieldExample");
    }

    #[test]
    fn animated_text_field_example_mounts_headlessly() {
        let mut app = AimerApp::start_headless(crate::theme::provide(
            AnimatedTextFieldExample::new().boxed(),
        ));

        app.pump_frames(2);
    }

    #[test]
    fn custom_text_field_caret_example_mounts_headlessly() {
        let mut app = AimerApp::start_headless(crate::theme::provide(
            CustomTextFieldCaretExample::new().boxed(),
        ));

        app.pump_frames(2);
    }
}

fn moew() {
    struct DrawCommand;
    struct DrawList(UnsafeCell<Vec<DrawCommand>>);
    struct DrawCommandList {
        cmds: Vec<DrawList>,
    }

    struct CurrentBuildContext {
        draw_cmd: Shared<DrawCommandList>,
    }

    struct CanvasInstance {
        draw_list: Shared<DrawList>,
    }
}

/// Select the native profiler path without rebuilding the application.
fn render_mode_from_args() -> aimer::RenderMode {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            if arg == "--render-mode" {
                return match args
                    .next()
                    .as_deref()
                {
                    Some("inline") => aimer::RenderMode::Inline,
                    Some("threaded") => aimer::RenderMode::Threaded,
                    _ => panic!("--render-mode requires inline or threaded"),
                };
            }
        }
    }
    aimer::RenderMode::default()
}
