use aimer::style::*;
use aimer::{AimerApp, *};

/// The size the panel starts at, in logical pixels.
const START_WIDTH: f32 = 360.0;
const START_HEIGHT: f32 = 240.0;

/// The limits every drag is held within, in logical pixels.
const MIN_WIDTH: f32 = 160.0;
const MIN_HEIGHT: f32 = 120.0;
const MAX_WIDTH: f32 = 720.0;
const MAX_HEIGHT: f32 = 520.0;

/// How far the grab band along each edge reaches into the panel, in logical
/// pixels.
const HANDLE_THICKNESS: f32 = 10.0;

/// How far it reaches out of the panel, in logical pixels, so the cursor changes
/// as the pointer arrives at the border rather than once it is inside.
const HANDLE_OUTSET: f32 = 6.0;

/// The size the width-only bar starts at, in logical pixels.
const BAR_WIDTH: f32 = 280.0;
const BAR_HEIGHT: f32 = 44.0;

/// Starts a showcase of the `Resizable` widget.
///
/// The page holds one panel the user resizes by dragging any of its four edges
/// or four corners. The cursor turns into the matching resize shape as soon as
/// the pointer enters a grab band, the drag keeps running once the pointer
/// leaves the panel, and the panel reports every step to `on_resize`, which is
/// what the live readout under the title is built from.
///
/// The panel is placed by the surrounding `Column`, so a resize changes its size
/// alone: dragging the left or top edge outwards asks for more space, and the
/// panel grows from its fixed top-left corner.
///
/// `on_resize_zone` feeds the second line of the readout: it reports the side
/// under the pointer as a `Direction` the moment it changes, and
/// `Direction::NONE` once the pointer is back over the child — the same answer
/// the cursor shape is drawn from.
///
/// Under it sits a bar with `direction(Direction::RIGHT)`, which shows what a
/// restricted set of sides does: only its right edge is a handle, and every
/// other band belongs to its child — cursor included.
pub fn start_resizable_example() {
    AimerApp::start(crate::theme::provide(ResizableShowcase::new().boxed()))
}

#[widget(Stateful)]
pub struct ResizableShowcase {}

impl ResizableShowcase {
    pub fn new() -> Self {
        Self {}
    }
}

pub struct ResizableShowcaseState {
    size: ResolvedSize,
    zone: Direction,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for ResizableShowcase {
    type State = ResizableShowcaseState;

    fn create_state(self) -> Self::State {
        ResizableShowcaseState {
            size: ResolvedSize {
                width: START_WIDTH,
                height: START_HEIGHT,
            },
            zone: Direction::NONE,
            updater: StateUpdater::empty(),
        }
    }
}

impl State<ResizableShowcase> for ResizableShowcaseState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(Column::new().children(vec![
                self.title(app_theme),
                self.readout(app_theme),
                self.zone_readout(app_theme),
                SizedBox::new().height(16).boxed(),
                self.panel(app_theme),
                SizedBox::new().height(24).boxed(),
                self.width_only_bar(app_theme),
            ]))
    }
}

/// The name of the single side `zone` holds, for the readout.
fn zone_name(zone: Direction) -> &'static str {
    match zone {
        Direction::LEFT => "left edge",
        Direction::RIGHT => "right edge",
        Direction::TOP => "top edge",
        Direction::BOTTOM => "bottom edge",
        Direction::TOP_LEFT => "top-left corner",
        Direction::TOP_RIGHT => "top-right corner",
        Direction::BOTTOM_LEFT => "bottom-left corner",
        Direction::BOTTOM_RIGHT => "bottom-right corner",
        _ => "none",
    }
}

impl ResizableShowcaseState {
    fn title(&self, app_theme: ThemeData) -> AnyWidget {
        Container::new()
            .height(Dimension::Px(34.0))
            .child(
                Text::new("Resizable: drag an edge or a corner")
                    .text_align(TextAlign::MidLeft)
                    .text_style(
                        TextStyle::new()
                            .font_size(24)
                            .color(app_theme.on_background_color),
                    ),
            )
            .boxed()
    }

    /// The live size, refreshed by `on_resize` on every drag step.
    fn readout(&self, app_theme: ThemeData) -> AnyWidget {
        Container::new()
            .height(Dimension::Px(24.0))
            .child(
                Text::new(format!(
                    "{:.0} x {:.0}  (min {MIN_WIDTH:.0} x {MIN_HEIGHT:.0}, max {MAX_WIDTH:.0} x {MAX_HEIGHT:.0})",
                    self.size.width, self.size.height
                ))
                .text_align(TextAlign::MidLeft)
                .text_style(
                    TextStyle::new()
                        .font_size(16)
                        .color(crate::theme::muted_text(&app_theme)),
                ),
            )
            .boxed()
    }

    /// The side the pointer is over, refreshed by `on_resize_zone`.
    fn zone_readout(&self, app_theme: ThemeData) -> AnyWidget {
        Container::new()
            .height(Dimension::Px(24.0))
            .child(
                Text::new(format!("zone: {}", zone_name(self.zone)))
                    .text_align(TextAlign::MidLeft)
                    .text_style(
                        TextStyle::new()
                            .font_size(16)
                            .color(app_theme.primary_color),
                    ),
            )
            .boxed()
    }

    /// The resizable panel itself.
    ///
    /// `on_resize` writes the new size into the state, so the readout above
    /// follows the drag. The panel keeps the size it was dragged to across that
    /// rebuild, which is what makes a drag survive its own `set_state`.
    ///
    /// `on_resize_zone` writes the side under the pointer, which is how the
    /// second readout knows what a click would grab before anything is dragged.
    fn panel(&self, app_theme: ThemeData) -> AnyWidget {
        let updater = self.updater;
        let zone_updater = self.updater;

        Resizable::new()
            .width(START_WIDTH)
            .height(START_HEIGHT)
            .min_width(MIN_WIDTH)
            .min_height(MIN_HEIGHT)
            .max_width(MAX_WIDTH)
            .max_height(MAX_HEIGHT)
            .handle_thickness(HANDLE_THICKNESS)
            .handle_outset(HANDLE_OUTSET)
            .direction(Direction::ALL)
            .on_resize(move |size: ResolvedSize| {
                updater.set_state(move |state| state.size = size);
            })
            .on_resize_zone(move |zone: Direction| {
                zone_updater.set_state(move |state| state.zone = zone);
            })
            .box_child(
                Container::new()
                    .padding(LayoutSpacing::all(Spacing::Px(20)))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.surface_color)
                            .border_radius(12)
                            .border(BoxBorder::all(
                                BorderSlice::new()
                                    .stroke(Dimension::Px(2.0))
                                    .style(BorderStyle::Solid)
                                    .color(
                                        app_theme
                                            .primary_color
                                            .with_alpha(0.70),
                                    ),
                            )),
                    )
                    .child(
                        Text::new(
                            "The border of this panel is the grab zone, a few \
                            pixels either side of it.\n\n\
                             Edges resize one axis, corners resize both. The \
                             pointer keeps the handle it started on, so the drag \
                             carries on outside the panel until you let go.",
                        )
                        .text_align(TextAlign::TopLeft)
                        .text_style(
                            TextStyle::new()
                                .font_size(15)
                                .color(app_theme.on_surface_color),
                        ),
                    ),
            )
    }

    /// A bar dragged by its right edge alone.
    ///
    /// `Direction` is a set of bit flags, so any combination of the eight sides
    /// can be live: `Direction::RIGHT` here, `Direction::RIGHT |
    /// Direction::BOTTOM_RIGHT` to add one corner, `Direction::ALL -
    /// Direction::TOP_EDGES` to keep everything but the top.
    fn width_only_bar(&self, app_theme: ThemeData) -> AnyWidget {
        Resizable::new()
            .width(BAR_WIDTH)
            .height(BAR_HEIGHT)
            .min_width(MIN_WIDTH)
            .max_width(MAX_WIDTH)
            .handle_thickness(HANDLE_THICKNESS)
            .handle_outset(HANDLE_OUTSET)
            .direction(Direction::RIGHT)
            .box_child(
                Container::new()
                    .padding(LayoutSpacing::all(Spacing::Px(12)))
                    .box_decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.surface_color)
                            .border_radius(10)
                            .border(BoxBorder::all(
                                BorderSlice::new()
                                    .stroke(Dimension::Px(2.0))
                                    .color(
                                        app_theme
                                            .primary_color
                                            .lighten(0.12),
                                    ),
                            )),
                    )
                    .child(
                        Text::new("Right edge only — every other band is inert")
                            .text_align(TextAlign::MidLeft)
                            .text_style(
                                TextStyle::new()
                                    .font_size(14)
                                    .color(crate::theme::muted_text(&app_theme)),
                            ),
                    ),
            )
    }
}
