use crate::accessibility_example::accessibility_example;
use crate::align_example::align_example;
use crate::animatable_example::animatable_example;
use crate::animated::MyAnimatedList;
use crate::animated_layout_example::animated_layout_example;
use crate::animated_text_field_example::AnimatedTextFieldExample;
use crate::animated_theme::AnimatedThemeExample;
use crate::assets_media_example::assets_media_example;
use crate::async_builder::async_builder_example;
use crate::audio_example::audio_example;
use crate::backdrop_filter_example::backdrop_filter_example;
use crate::bundled_assets_example::bundled_assets_example;
use crate::collapsible_list_example::collapsible_list_example;
use crate::color_melt_example::color_melt_example;
use crate::color_sync::ColorSync;
use crate::container_transform_example::container_transform_example;
use crate::custom_animated_theme::custom_animated_theme_example;
use crate::custom_font::custom_font_example;
use crate::custom_paint_example::custom_paint_example;
use crate::custom_shape_example::custom_shape_example;
use crate::custom_text_field_caret_example::CustomTextFieldCaretExample;
use crate::data_view_example::data_view_example;
use crate::dnd_completion_example::dnd_completion_example;
use crate::drag_and_drop::DragBoard;
use crate::feedback_example::feedback_example;
use crate::file_drop_zone::FileDropShowcase;
use crate::file_picker_example::file_picker_example;
use crate::fire_button_example::fire_button_example;
use crate::floating::FloatingShowcase;
use crate::focus_node_example::FocusNodeExample;
use crate::foreground_decoration_example::foreground_decoration_example;
use crate::form_example::form_example;
use crate::gif_example::gif_example;
use crate::glass_liquid_example::glass_liquid_example;
use crate::gradient_example::gradient_example;
use crate::http_request_button::http_request_button_example;
use crate::i18n_example::i18n_example;
use crate::justify_content_example::justify_content_example;
use crate::light_edge_example::light_edge_example;
use crate::liquid_tab_bar_example::liquid_tab_bar_example;
use crate::loading_animation::loading_animation_example;
use crate::lottie_example::lottie_example;
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
use crate::shortcuts_example::ShortcutsExample;
use crate::stack_positioned_example::stack_positioned_example;
use crate::stateful::CounterWidget;
use crate::live_activity_timer_example::LiveActivityTimerExample;
use crate::notification_example::NotificationExample;
use crate::tray_example::TrayExample;
use crate::daemon_example::DaemonExample;
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
use crate::video_example::video_example;
use crate::window_example::WindowShowcase;
use aimer::input::button::Button;
use aimer::macros::{Router, StatefulWidget, StatelessWidget};
use aimer::router::{Navigator, NavigatorController, Router, SlideTransition};
use aimer::style::*;
use aimer::*;

const SIDEBAR_WIDTH: f32 = 292.0;

/// Whether the window blurs what is behind it, which the sidebar then shows.
///
/// macOS only: Windows draws its backdrop through a swap chain that ignores
/// alpha, so there the sidebar would be a see-through hole instead of glass.
const NATIVE_BACKDROP: bool = cfg!(target_os = "macos");

/// Opacity of the sidebar's theme tint over the blurred backdrop.
const SIDEBAR_TINT: f32 = 0.66;

/// The sidebar's fill: the theme's surface color as a tint over the window's
/// blur, or the plain surface where the window has no blur.
fn sidebar_color(app_theme: &ThemeData) -> Color {
    if NATIVE_BACKDROP {
        app_theme.surface_color.with_alpha(SIDEBAR_TINT)
    } else {
        app_theme.surface_color
    }
}

/// The line between the sidebar and the content pane.
///
/// Opaque on purpose: the shell behind it is transparent over the window's blur,
/// so a translucent line would let the glass through and read as a gap. It is the
/// themed divider tint pre-mixed over the background instead.
fn separator_color(app_theme: &ThemeData) -> Color {
    app_theme
        .background_color
        .lerp(app_theme.primary_color, 0.32)
}

/// The fill behind the whole shell. Transparent while the window has a blur, so
/// that the sidebar's translucent tint is the only thing between it and the
/// glass; every other pane paints its own opaque background.
fn shell_color(app_theme: &ThemeData) -> Color {
    if NATIVE_BACKDROP {
        Color::Transparent
    } else {
        app_theme.background_color
    }
}

/// Logical width below which the list and the detail become two routed screens.
const COMPACT_WIDTH: f32 = 600.0;

/// Label of the control that returns from the detail screen to the list.
const BACK_LABEL: &str = "Back";

mod catalog;
use catalog::{EXAMPLE_CATEGORIES, EXAMPLES, ExampleCategory, ExampleId};

/// The interactive example browser used by Jaime.
///
/// On a wide window it is two panes side by side; below [`COMPACT_WIDTH`] it is
/// two screens in a [`Navigator`], the items list and then the detail of the
/// item that was tapped, which slides in over the list; a back bar, a swipe or
/// the system back pops it.
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
        // Rebuilds the shell when the window crosses the threshold, and at no
        // other width.
        let compact = ctx.select_window_metrics(|window| {
            window
                .logical_size()
                .width
                < COMPACT_WIDTH
        });

        let shell = Provider::new()
            .handle(
                self.selection
                    .clone(),
            )
            .child(
                Container::new()
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(shell_color(&app_theme))
                            .border_radius(24),
                    )
                    .child(Column::new().children(vec![
                        // title_bar(),
                        Expanded::new()
                            .child(if compact {
                                CompactScreens::new(app_theme).boxed()
                            } else {
                                Row::new()
                                    .children(list![
                                        sidebar(app_theme, Dimension::Px(SIDEBAR_WIDTH)),
                                        SizedBox::new()
                                            .width(Dimension::Px(1.0))
                                            .color(separator_color(&app_theme)),
                                        Expanded::new()
                                            .child(ContentPane::new(app_theme))
                                    ])
                                    .boxed()
                            })
                            .boxed(),
                    ])),
            );
        // The window's blur is what the sidebar tints; it applies to the whole
        // window, and the content pane covers it with its own opaque fill.
        WindowBackdrop::new()
            .material(WindowMaterial::Sidebar)
            .enabled(NATIVE_BACKDROP)
            .child(shell)
    }
}

/// The two screens of the compact layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Router)]
enum CompactRoute {
    #[route("/")]
    Items,
    #[route("/detail")]
    #[view_level(1)]
    Detail,
}

impl Router for CompactRoute {
    fn build(&self, _ctx: &BuildContext) -> AnyWidget {
        match self {
            Self::Items => ItemsScreen.boxed(),
            Self::Detail => DetailScreen.boxed(),
        }
    }
}

/// The items list, the navigator's first page.
#[derive(StatelessWidget)]
struct ItemsScreen;

impl StatelessWidget for ItemsScreen {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        sidebar(ThemeData::copied(ctx), Dimension::Percent(100.0))
    }
}

/// The chosen item's detail, pushed over the list. It reads the selection
/// through [`ContentPane`], so it needs no state of its own.
#[derive(StatelessWidget)]
struct DetailScreen;

impl StatelessWidget for DetailScreen {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        // Opaque, so the page beneath never shows through while sliding.
        Container::new()
            .width(Dimension::Percent(100.0))
            .height(Dimension::Percent(100.0))
            .color(app_theme.background_color)
            .child(Column::new().children(vec![
                BackBar::new(app_theme).boxed(),
                Expanded::new()
                    .child(ContentPane::new(app_theme))
                    .boxed(),
            ]))
    }
}

/// The compact layout: a navigator sliding between the items list and the
/// detail of the chosen item.
///
/// It pads itself clear of the system bars, which the wide layout never needs.
/// It reads nothing of the selection, so a choice never rebuilds it.
#[derive(StatelessWidget)]
struct CompactScreens;

impl CompactScreens {
    fn new(app_theme: ThemeData) -> Self {
        Self
    }
}

impl StatelessWidget for CompactScreens {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::of(ctx);
        let insets = ctx.watch_safe_area_insets();
        Container::new()
            .width(Dimension::Percent(100.0))
            .height(Dimension::Percent(100.0))
            .color(shell_color(&app_theme))
            .child(
                Navigator::new(CompactRoute::Items, |route| route.boxed())
                    .transition(SlideTransition::new()),
            )
    }
}

/// The bar above the detail screen that pops back to the items list.
#[derive(StatelessWidget)]
struct BackBar {
    app_theme: ThemeData,
}

impl BackBar {
    fn new(app_theme: ThemeData) -> Self {
        Self { app_theme }
    }
}

impl StatelessWidget for BackBar {
    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = self.app_theme;
        let navigator = NavigatorController::<CompactRoute>::of(ctx);
        let foreground = app_theme.primary_color;
        Container::new()
            .width(Dimension::Percent(100.0))
            .padding(
                LayoutSpacing::new()
                    .left(8)
                    .right(8)
                    .top(6),
            )
            .child(
                Button::new()
                    .on_press(move || navigator.pop())
                    .decoration(BoxDecoration::new().border_radius(10))
                    .hover_decoration(
                        BoxDecoration::new()
                            .background_color(theme::raised_surface(&app_theme))
                            .border_radius(10),
                    )
                    .press_decoration(
                        BoxDecoration::new()
                            .background_color(theme::raised_surface(&app_theme))
                            .border_radius(10),
                    )
                    .box_child(
                        Container::new()
                            .height(40)
                            .padding(
                                LayoutSpacing::new()
                                    .left(10)
                                    .right(14),
                            )
                            .child(
                                Row::new()
                                    .vertical_alignment(BoxAlignment::Center)
                                    .gaps(LayoutSpacing::all(Spacing::Px(6)))
                                    .children(vec![
                                        Text::new("\u{2039}")
                                            .text_style(
                                                TextStyle::new()
                                                    .font_size(26)
                                                    .color(foreground),
                                            )
                                            .boxed(),
                                        Text::new(BACK_LABEL)
                                            .text_style(
                                                TextStyle::new()
                                                    .font_size(16)
                                                    .font_weight(FontWeight::Bold)
                                                    .color(foreground),
                                            )
                                            .boxed(),
                                    ]),
                            ),
                    ),
            )
    }
}

fn sidebar(app_theme: ThemeData, width: Dimension) -> AnyWidget {
    
    let compact = BuildContext::current().get().select_window_metrics(|window| {
        window
            .logical_size()
            .width
            < COMPACT_WIDTH
    });
    Container::new()
        .width(width)
        .height(Dimension::Percent(100.0))
        .padding(
            LayoutSpacing::new()
                .left(18)
                .right(14),
        )
        .color(sidebar_color(&app_theme))
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
                                    if index == 0 {
                                        Container::new()
                                            .margin(LayoutSpacing::new().top(case!(
                                                cfg!(target_os = "macos") || compact,
                                                38
                                            )))
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
        // Only the compact layout has a navigator; the wide one shows both panes.
        let navigator = ctx
            .get_state::<NavigatorController<CompactRoute>>()
            .map(|navigator| (*navigator).clone());
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
                if let Some(navigator) = &navigator {
                    navigator.push(CompactRoute::Detail);
                }
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
        ExampleId::Shortcuts => ShortcutsExample::new().boxed(),
        ExampleId::Tray => TrayExample::new().boxed(),
        ExampleId::Daemon => DaemonExample::new().boxed(),
        ExampleId::Notifications => NotificationExample::new().boxed(),
        ExampleId::LiveActivityTimer => LiveActivityTimerExample::new().boxed(),
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
        ExampleId::FilePicker => file_picker_example().boxed(),
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
        ExampleId::Gif => gif_example().boxed(),
        ExampleId::Audio => audio_example().boxed(),
        ExampleId::Video => video_example().boxed(),
        ExampleId::BundledAssets => bundled_assets_example().boxed(),
        ExampleId::CustomPaint => custom_paint_example().boxed(),
        ExampleId::ColorMelt => color_melt_example().boxed(),
        ExampleId::FireButton => fire_button_example().boxed(),
        ExampleId::Lottie => lottie_example().boxed(),
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
