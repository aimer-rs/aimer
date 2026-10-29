use crate::accessibility_example::accessibility_example;
use crate::align_example::align_example;
use crate::animatable_example::animatable_example;
use crate::animated::MyAnimatedList;
use crate::animated_layout_example::animated_layout_example;
use crate::animated_text_field_example::AnimatedTextFieldExample;
use crate::animated_theme::AnimatedThemeExample;
use crate::assets_media_example::assets_media_example;
use crate::async_builder::async_builder_example;
use crate::backdrop_filter_example::backdrop_filter_example;
use crate::bundled_assets_example::bundled_assets_example;
use crate::collapsible_list_example::collapsible_list_example;
use crate::color_sync::ColorSync;
use crate::container_transform_example::container_transform_example;
use crate::custom_animated_theme::custom_animated_theme_example;
use crate::custom_font::custom_font_example;
use crate::custom_shape_example::custom_shape_example;
use crate::custom_text_field_caret_example::CustomTextFieldCaretExample;
use crate::data_view_example::data_view_example;
use crate::dnd_completion_example::dnd_completion_example;
use crate::drag_and_drop::DragBoard;
use crate::feedback_example::feedback_example;
use crate::file_drop_zone::FileDropShowcase;
use crate::floating::FloatingShowcase;
use crate::focus_node_example::FocusNodeExample;
use crate::foreground_decoration_example::foreground_decoration_example;
use crate::form_example::form_example;
use crate::glass_liquid_example::glass_liquid_example;
use crate::gradient_example::gradient_example;
use crate::http_request_button::http_request_button_example;
use crate::i18n_example::i18n_example;
use crate::justify_content_example::justify_content_example;
use crate::light_edge_example::light_edge_example;
use crate::liquid_tab_bar_example::liquid_tab_bar_example;
use crate::loading_animation::loading_animation_example;
use crate::lucide_icons_example::lucide_icons_example;
use crate::magnification_example::magnification_example;
use crate::markdown_example::{custom_markdown_viewer, jaime_markdown_viewer};
use crate::modal::modal_example;
use crate::morph_transition_example::morph_transition_example;
use crate::multi_decoration_box_example::multi_decoration_box_example;
use crate::navigation_example::navigation_example;
use crate::navigation_transitions_example::navigation_transitions_example;
use crate::overflow_behavior_example::overflow_behavior_example;
use crate::panic_recovery::panic_recovery_example;
use crate::passkey_example::passkey_example;
use crate::picker_example::picker_example;
use crate::range_controls_example::RangeControlsExample;
use crate::resizable_example::ResizableShowcase;
use crate::routing::state_router_widget;
use crate::routing_context_example::routing_context_example;
use crate::selectable_text::selectable_text_example;
use crate::selection_controls_example::selection_controls_example;
use crate::stack_positioned_example::stack_positioned_example;
use crate::stateful::CounterWidget;
use crate::stateful_2::MyList;
use crate::storage_example::storage_example;
use crate::style_tokens_example::style_tokens_example;
use crate::svg_example::svg_example as svg_completion_example;
use crate::svg_test::svg_example;
use crate::system_theme::SystemThemeExample;
use crate::test_animation::TestFadingAnimation;
use crate::text_area_example::TextAreaExample;
use crate::text_field_example::TextFieldExample;
use crate::text_properties_example::text_properties_example;
use crate::theme;
use crate::tween_keyframe_example::tween_keyframe_example;
use crate::window_example::WindowShowcase;
use aimer::input::button::Button;
use aimer::macros::{StatefulWidget, StatelessWidget};
use aimer::style::*;
use aimer::*;

const SIDEBAR_WIDTH: f32 = 292.0;

mod catalog;
use catalog::{EXAMPLE_CATEGORIES, EXAMPLES, ExampleCategory, ExampleId};

/// The two-pane interactive example browser used by Jaime.
///
/// The left pane owns only selection and navigation. The selected example is
/// rebuilt in the right viewport, so an example module can focus on its own
/// widget without knowing anything about the catalogue shell.
///
/// The selection lives in a provider rather than in this widget's state, so
/// choosing an example rebuilds only what depends on it: the rows whose
/// highlight flips and the content pane. The sidebar and the rest of the shell
/// are not rebuilt, and choosing the example that is already selected rebuilds
/// nothing.
#[derive(StatefulWidget)]
pub struct ExampleShowcase {}

impl ExampleShowcase {
    /// Creates the example browser with the first registered page selected.
    #[inline]
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(test)]
thread_local! {
    /// Counts `ExampleShowcaseState::build` calls so tests can prove that choosing
    /// an example does not rebuild the shell.
    static SHELL_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The example currently shown, provided to the sidebar rows and the content
/// pane. Each reader subscribes to a projection of it with
/// [`ProviderContext::select`], which wakes it only when that projection changes.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Selection {
    selected: ExampleId,
}

pub struct ExampleShowcaseState {
    selection: ProviderHandle<Selection>,
}

impl ExampleShowcaseState {
    /// Creates the state with `selected` already chosen.
    fn with_selected(selected: ExampleId) -> Self {
        Self {
            selection: ProviderHandle::new(Selection { selected }),
        }
    }
}

impl StatefulWidget for ExampleShowcase {
    type State = ExampleShowcaseState;

    fn create_state(self) -> Self::State {
        ExampleShowcaseState::with_selected(EXAMPLES[0])
    }
}

impl State<ExampleShowcase> for ExampleShowcaseState {
    fn init_state(&mut self, _updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        #[cfg(test)]
        SHELL_BUILDS.with(|builds| builds.set(builds.get() + 1));
        let app_theme = ThemeData::copied(ctx);

        Provider::new()
            .handle(
                self.selection
                    .clone(),
            )
            .child(
                Container::new()
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.background_color)
                            .border_radius(24),
                    )
                    .child(Column::new().children(vec![
                        // title_bar(),
                        Expanded::new()
                            .child(
                                Row::new().children(vec![
                                    sidebar(app_theme),
                                    SizedBox::new()
                                        .width(Dimension::Px(1.0))
                                        .color(theme::divider(&app_theme))
                                        .boxed(),
                                    Expanded::new()
                                        .child(ContentPane::new(app_theme))
                                        .boxed(),
                                ]),
                            )
                            .boxed(),
                    ])),
            )
    }
}

fn sidebar(app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .width(Dimension::Px(SIDEBAR_WIDTH))
        .height(Dimension::Percent(100.0))
        .padding(
            LayoutSpacing::new()
                .left(18)
                .right(14)
                .bottom(16),
        )
        .color(app_theme.surface_color)
        .child(
            Scrollable::new()
                .vertical_scroll_bar(None)
                .axis(ScrollAxis::Vertical)
                .child(
                    Column::new()
                        .gaps(LayoutSpacing::new().bottom(3))
                        .children(
                            EXAMPLE_CATEGORIES
                                .iter()
                                .enumerate()
                                .map(|(index, category)| {
                                    let category = example_category(category, app_theme);
                                    if cfg!(target_os = "macos") && index == 0 {
                                        Container::new()
                                            .margin(LayoutSpacing::new().top(38))
                                            .child(category)
                                            .boxed()
                                    } else {
                                        category
                                    }
                                }),
                        ),
                ),
        )
        .boxed()
}

fn example_category(category: &ExampleCategory, app_theme: ThemeData) -> AnyWidget {
    CollapsibleList::new()
        .animation_millis(300)
        .header(
            ListHeader::new().child(
                Text::new(category.label).text_style(
                    TextStyle::new()
                        .font_size(14)
                        .font_weight(FontWeight::Bold)
                        .color(theme::muted_text(&app_theme)),
                ),
            ),
        )
        .body(
            ListBody::new().child(
                Column::new()
                    .gaps(LayoutSpacing::new().bottom(3))
                    .children(
                        category
                            .examples
                            .iter()
                            .copied()
                            .map(|example| ExampleButton::new(example, app_theme).notype()),
                    ),
            ),
        )
        .notype()
}

/// One sidebar row. It reads whether it is the selected example itself, so a
/// selection change rebuilds only the two rows whose answer changes.
#[derive(StatelessWidget)]
struct ExampleButton {
    example: ExampleId,
    app_theme: ThemeData,
}

impl ExampleButton {
    fn new(example: ExampleId, app_theme: ThemeData) -> Self {
        Self { example, app_theme }
    }
}

impl StatelessWidget for ExampleButton {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let example = self.example;
        let app_theme = self.app_theme;
        let icon = example.icon();
        let label = example.label();
        let selected =
            ctx.select::<Selection, bool>(move |selection| selection.selected == example);
        let selection = ProviderHandle::<Selection>::of(ctx);
        let background = if selected {
            app_theme.primary_color
        } else {
            Color::Transparent
        };
        let foreground = if selected {
            app_theme.on_primary_color
        } else {
            app_theme.on_surface_color
        };

        Button::new()
            .on_press(move || {
                selection.update(|selection| selection.selected = example);
            })
            .decoration(
                BoxDecoration::new()
                    .background_color(background)
                    .border_radius(10),
            )
            .hover_decoration(
                BoxDecoration::new()
                    .background_color(if selected {
                        app_theme.primary_color
                    } else {
                        theme::raised_surface(&app_theme)
                    })
                    .border_radius(10),
            )
            .press_decoration(
                BoxDecoration::new()
                    .background_color(
                        app_theme
                            .primary_color
                            .darken(0.08),
                    )
                    .border_radius(10),
            )
            .box_child(
                Container::new()
                    .width(100.pc())
                    .height(39)
                    .padding(
                        LayoutSpacing::new()
                            .left(11)
                            .right(9),
                    )
                    .child(
                        Row::new()
                            .vertical_alignment(BoxAlignment::Center)
                            .gaps(LayoutSpacing::all(Spacing::Px(10)))
                            .children(vec![
                                Container::new()
                                    .width(Dimension::Px(24.0))
                                    .child(
                                        Text::new(icon)
                                            .text_align(TextAlign::MidCenter)
                                            .text_style(
                                                TextStyle::new()
                                                    .font_size(18)
                                                    .color(foreground),
                                            ),
                                    )
                                    .boxed(),
                                Text::new(label)
                                    .text_style(
                                        TextStyle::new()
                                            .font_size(15)
                                            .font_weight(if selected {
                                                FontWeight::Bold
                                            } else {
                                                FontWeight::Normal
                                            })
                                            .color(foreground),
                                    )
                                    .boxed(),
                            ]),
                    ),
            )
    }
}

/// The page header and the selected example. It reads the selection itself, so
/// a change rebuilds this pane and nothing else of the shell.
#[derive(StatelessWidget)]
struct ContentPane {
    app_theme: ThemeData,
}

impl ContentPane {
    fn new(app_theme: ThemeData) -> Self {
        Self { app_theme }
    }
}

impl StatelessWidget for ContentPane {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let selected = ctx.select::<Selection, ExampleId>(|selection| selection.selected);
        content(selected, self.app_theme)
    }
}

fn content(selected: ExampleId, app_theme: ThemeData) -> AnyWidget {
    Container::new()
        .height(Dimension::Percent(100.0))
        .padding(
            LayoutSpacing::new()
                .top(20)
                .right(24)
                .bottom(22)
                .left(24),
        )
        .color(app_theme.background_color)
        .child(Column::new().children(vec![
            SizedBox::new().height(18).boxed(),
            Text::new(selected.label())
                .text_style(
                    TextStyle::new()
                        .font_size(30)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_background_color),
                )
                .boxed(),
            SizedBox::new().height(7).boxed(),
            Text::new(selected.description())
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .boxed(),
            SizedBox::new().height(18).boxed(),
            Expanded::new()
                .child(
                    Container::new()
                        .width(Dimension::Percent(100.0))
                        // .padding(LayoutSpacing::all(Spacing::Px(24)))
                        // .box_decoration(
                        //     BoxDecoration::new()
                        //         .background_color(theme::raised_surface(&app_theme))
                        //         .border_radius(14),
                        // )
                        .child(build_example(selected, app_theme)),
                )
                .boxed(),
        ]))
        .boxed()
}

fn build_example(example: ExampleId, app_theme: ThemeData) -> AnyWidget {
    match example {
        ExampleId::Accessibility => accessibility_example().boxed(),
        ExampleId::ChoiceControls => selection_controls_example().boxed(),
        ExampleId::RangeControls => RangeControlsExample::new().boxed(),
        ExampleId::Forms => form_example().boxed(),
        ExampleId::Pickers => picker_example().boxed(),
        ExampleId::Feedback => feedback_example().boxed(),
        ExampleId::Navigation => navigation_example().boxed(),
        ExampleId::NavigationTransitions => navigation_transitions_example().boxed(),
        ExampleId::ContainerTransform => container_transform_example().boxed(),
        ExampleId::LiquidTabBar => liquid_tab_bar_example().boxed(),
        ExampleId::RoutingContext => routing_context_example().boxed(),
        ExampleId::Animatable => animatable_example().boxed(),
        ExampleId::DataView => data_view_example().boxed(),
        ExampleId::DndCompletion => dnd_completion_example().boxed(),
        ExampleId::StyleTokens => style_tokens_example().boxed(),
        ExampleId::I18n => i18n_example().boxed(),
        ExampleId::SvgCompletion => svg_completion_example().boxed(),
        ExampleId::LucideIcons => lucide_icons_example(app_theme).boxed(),
        ExampleId::AssetsMedia => assets_media_example().boxed(),
        ExampleId::GlassLiquid => glass_liquid_example().boxed(),
        ExampleId::BackdropFilter => backdrop_filter_example().boxed(),
        ExampleId::Magnification => magnification_example().boxed(),
        ExampleId::LightEdge => light_edge_example().boxed(),
        ExampleId::Gradient => gradient_example().boxed(),
        ExampleId::ForegroundDecoration => foreground_decoration_example().boxed(),
        ExampleId::MultiDecorationBox => multi_decoration_box_example().boxed(),
        ExampleId::Storage => storage_example().boxed(),
        ExampleId::CustomShape => custom_shape_example().boxed(),
        ExampleId::AnimatedLayout => animated_layout_example().boxed(),
        ExampleId::TweenKeyframes => tween_keyframe_example().boxed(),
        ExampleId::MorphTransition => morph_transition_example().boxed(),
        ExampleId::TextProperties => text_properties_example().boxed(),
        ExampleId::Window => WindowShowcase::new().boxed(),
        ExampleId::Resizable => ResizableShowcase::new().boxed(),
        ExampleId::FileDrop => FileDropShowcase::new().boxed(),
        ExampleId::Floating => FloatingShowcase::new().boxed(),
        ExampleId::FocusNode => FocusNodeExample::new().boxed(),
        ExampleId::TextArea => TextAreaExample::new().boxed(),
        ExampleId::TextField => TextFieldExample::new().boxed(),
        ExampleId::CustomTextFieldCaret => CustomTextFieldCaretExample::new().boxed(),
        ExampleId::AnimatedTextField => AnimatedTextFieldExample::new().boxed(),
        ExampleId::Markdown => jaime_markdown_viewer().boxed(),
        ExampleId::CustomMarkdown => custom_markdown_viewer().boxed(),
        ExampleId::Counter => CounterWidget::new(1).boxed(),
        ExampleId::StatefulList => MyList::new().boxed(),
        ExampleId::CollapsibleList => collapsible_list_example().boxed(),
        ExampleId::AsyncBuilder => async_builder_example().boxed(),
        ExampleId::SelectableText => selectable_text_example().boxed(),
        ExampleId::SystemTheme => SystemThemeExample::new().boxed(),
        ExampleId::AnimatedTheme => AnimatedThemeExample::new().boxed(),
        ExampleId::CustomAnimatedTheme => custom_animated_theme_example().boxed(),
        ExampleId::DragAndDrop => DragBoard::new().boxed(),
        ExampleId::HttpRequest => http_request_button_example().boxed(),
        ExampleId::Passkeys => passkey_example().boxed(),
        ExampleId::Loading => loading_animation_example().boxed(),
        ExampleId::Overflow => overflow_behavior_example().boxed(),
        ExampleId::JustifyContent => justify_content_example().boxed(),
        ExampleId::Svg => svg_example().boxed(),
        ExampleId::CustomFont => custom_font_example().boxed(),
        ExampleId::PanicRecovery => panic_recovery_example().boxed(),
        ExampleId::Modal => modal_example().boxed(),
        ExampleId::ColorSync => ColorSync.boxed(),
        ExampleId::AnimatedList => MyAnimatedList.boxed(),
        ExampleId::Animation => TestFadingAnimation.boxed(),
        ExampleId::Routing => state_router_widget().boxed(),
        ExampleId::Text => text_example(app_theme),
        ExampleId::Positioned => stack_positioned_example(app_theme).boxed(),
        ExampleId::Align => align_example(app_theme).boxed(),
        ExampleId::BorderOutline => border_outline_example(app_theme),
        ExampleId::AssetImage => asset_image_example(app_theme),
        ExampleId::BundledAssets => bundled_assets_example().boxed(),
    }
}

fn text_example(app_theme: ThemeData) -> AnyWidget {
    demo_card(
        "Text",
        "A baseline text widget with wrapping and multiple writing systems.",
        Text::new("Hello, Aimer!\nEnglish · Français · 中文 · សួស្តី")
            .text_style(
                TextStyle::new()
                    .font_size(24)
                    .color(app_theme.on_surface_color),
            )
            .boxed(),
        app_theme,
    )
}

fn border_outline_example(app_theme: ThemeData) -> AnyWidget {
    demo_card(
        "Border and outline",
        "A border participates in the decorated box; an outline is painted outside it.",
        Container::new()
            .width(400.0)
            .height(400.0)
            .margin(LayoutSpacing::all(Spacing::Px(30)))
            .box_decoration(
                BoxDecoration::new()
                    .background_color(theme::recessed_surface(&app_theme))
                    .border_radius(10)
                    .border(BoxBorder::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(Stroke::Px(8.0))
                            .color(app_theme.primary_color),
                    ))
                    .outline(BoxOutline::all(
                        BorderSlice::new()
                            .style(BorderStyle::Solid)
                            .stroke(Stroke::Px(8.0))
                            .color(
                                app_theme
                                    .primary_color
                                    .lighten(0.16),
                            ),
                    )),
            )
            .child(
                Text::new("Border inside · outline outside").text_style(
                    TextStyle::new()
                        .font_size(18)
                        .color(app_theme.on_surface_color),
                ),
            )
            .boxed(),
        app_theme,
    )
}

fn asset_image_example(app_theme: ThemeData) -> AnyWidget {
    demo_card(
        "Asset image",
        "A bundled image can be scaled and decorated without a platform-specific API.",
        AssetImage::new("assets/my_image.png")
            .fit(BoxFit::FitWidth)
            .boxed(),
        app_theme,
    )
}

fn demo_card(
    title: &'static str,
    description: &'static str,
    demo: AnyWidget,
    app_theme: ThemeData,
) -> AnyWidget {
    Container::new()
        .padding(LayoutSpacing::all(Spacing::Px(28)))
        .child(Column::new().children(vec![
            Text::new(title)
                .text_style(
                    TextStyle::new()
                        .font_size(26)
                        .font_weight(FontWeight::Bold)
                        .color(app_theme.on_surface_color),
                )
                .boxed(),
            SizedBox::new().height(8).boxed(),
            Text::new(description)
                .text_style(TextStyle::new().font_size(15).color(theme::muted_text(&app_theme)))
                .boxed(),
            SizedBox::new().height(24).boxed(),
            demo,
        ]))
        .boxed()
}

#[cfg(test)]
mod tests;
