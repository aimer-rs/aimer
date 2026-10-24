//! Keyboard focus, made visible.
//!
//! Two boxes sit side by side. Exactly one of them may own the keyboard at a
//! time, and the one that does draws a border. Clicking a box gives it focus,
//! clicking the page behind them takes focus away, and `Tab` / `Shift-Tab`
//! move it from one box to the other.
//!
//! The whole of that behaviour comes from one handle — [`aimer::FocusNode`] —
//! and one promise: an element that returns a node from
//! [`EventElement::focus_node`] is a *focusable target*. Everything else is the
//! framework's:
//!
//! - a press is routed to whatever is under it, and focus moves to the nearest
//!   focusable target it hit, or to nothing at all when it hit nothing
//!   focusable;
//! - `Tab` walks the focusable targets of the frame in tree order;
//! - the element that gains or loses focus is told so with
//!   [`ElementEvent::FocusGained`] and [`ElementEvent::FocusLost`].
//!
//! Any subtree becomes such a target by wrapping it in [`aimer::Focusable`],
//! which is all this example uses to turn two plain containers into things the
//! keyboard can be given to.
//!
//! The node itself lives in the *state*, never in the widget: widgets are
//! rebuilt on every `set_state` and thrown away, while focus ownership must
//! survive those rebuilds. Cloning a node yields another handle to the same
//! target, not a second target, which is exactly what a rebuild needs.

use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::*;
use aimer::*;

/// Starts the focus showcase.
pub fn start_focus_node_example() {
    AimerApp::start(crate::theme::provide(FocusNodeExample::new().boxed()))
}

/// Which of the two boxes is meant.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FocusBox {
    First,
    Second,
}

impl FocusBox {
    /// The label painted inside the box.
    #[inline]
    const fn label(self) -> &'static str {
        match self {
            Self::First => "First",
            Self::Second => "Second",
        }
    }
}

/// How wide a focus box is drawn.
const BOX_WIDTH: f32 = 220.0;

/// How tall a focus box is drawn.
const BOX_HEIGHT: f32 = 140.0;

/// How thick the focus border is.
///
/// The border is *always* this thick — only its colour changes — so gaining
/// focus cannot nudge the child by three pixels. Painting a border that is not
/// there is cheaper than laying the box out twice.
const BORDER_STROKE: f32 = 3.0;

/// The decoration of a focus box, with a visible border only while `focused`.
///
/// # Examples
///
/// ```ignore
/// // The two states differ, and differ only in the border.
/// assert_ne!(box_decoration(true), box_decoration(false));
/// assert_eq!(
///     box_decoration(true).background_color,
///     box_decoration(false).background_color
/// );
/// ```
fn box_decoration(focused: bool) -> BoxDecoration {
    let app_theme = crate::theme::app_theme();
    let border_color = if focused {
        app_theme.primary_color
    } else {
        Color::Transparent
    };
    BoxDecoration::new()
        .background_color(app_theme.surface_color)
        .border_radius((12, 12, 12, 12))
        .border(BoxBorder::all(
            BorderSlice::new()
                .style(BorderStyle::Solid)
                .stroke(Stroke::Px(BORDER_STROKE))
                .color(border_color),
        ))
}

/// Two boxes that show which of them owns the keyboard.
#[widget(Stateful)]
pub struct FocusNodeExample;

impl FocusNodeExample {
    /// Creates the showcase.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for FocusNodeExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the example: the two focus targets, and which of them
/// last reported that it owns the keyboard.
pub struct FocusNodeExampleState {
    first: FocusNode,
    second: FocusNode,
    focused: Option<FocusBox>,
    updater: StateUpdater<Self>,
}

impl StatefulWidget for FocusNodeExample {
    type State = FocusNodeExampleState;

    fn create_state(self) -> FocusNodeExampleState {
        FocusNodeExampleState {
            first: FocusNode::new(),
            second: FocusNode::new(),
            focused: None,
            updater: StateUpdater::empty(),
        }
    }
}

impl FocusNodeExampleState {
    /// The node of `which` box.
    #[inline]
    fn node(&self, which: FocusBox) -> &FocusNode {
        match which {
            FocusBox::First => &self.first,
            FocusBox::Second => &self.second,
        }
    }

    /// One box: a focusable target wrapped around a decorated container.
    fn focus_box(&self, which: FocusBox) -> AnyWidget {
        let focused = self.focused == Some(which);
        let updater = self.updater;
        Focusable::new()
            .node(
                self.node(which)
                    .clone(),
            )
            .on_focus_change(move |gained| {
                // The box that lost focus reports first, so recording `None`
                // here can never erase the box that gained it.
                updater.set_state(move |state| state.focused = gained.then_some(which));
            })
            .box_child(
                Container::new()
                    .width(Dimension::Px(BOX_WIDTH))
                    .height(Dimension::Px(BOX_HEIGHT))
                    .box_decoration(box_decoration(focused))
                    .box_child(
                        Text::new(which.label())
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new()
                                    .font_size(20)
                                    .color(crate::theme::app_theme().on_surface_color),
                            ),
                    ),
            )
    }

    /// A button that moves focus without pointing at a box.
    ///
    /// [`FocusNode::request_focus`] records the wish; the framework grants it
    /// while synchronizing the tree, so nothing here has to know which element
    /// currently owns the keyboard. Note what the click does on its own: it
    /// lands on a button, which is not focusable, and therefore drops focus —
    /// the request recorded by the handler is granted afterwards, before the
    /// same click is finished with.
    fn focus_button(
        &self,
        label: &'static str,
        node: Option<FocusNode>,
        app_theme: ThemeData,
    ) -> AnyWidget {
        Container::new()
            .width(Dimension::Px(160.0))
            .height(Dimension::Px(44.0))
            .box_child(
                Button::new()
                    .on_press({
                        let first = self.first.clone();
                        let second = self.second.clone();
                        move || match &node {
                            Some(node) => node.request_focus(),
                            None => {
                                first.unfocus();
                                second.unfocus();
                            }
                        }
                    })
                    .decoration(
                        BoxDecoration::new()
                            .background_color(app_theme.primary_color)
                            .border_radius((8, 8, 8, 8)),
                    )
                    .child(
                        Text::new(label)
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new()
                                    .font_size(15)
                                    .color(app_theme.on_primary_color),
                            ),
                    ),
            )
    }
}

impl State<FocusNodeExample> for FocusNodeExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let owner = match self.focused {
            Some(which) => which.label(),
            None => "nothing",
        };
        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(32)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(16))
                    .children([
                        Text::new("FocusNode")
                            .text_style(
                                TextStyle::new()
                                    .font_size(28)
                                    .font_weight(FontWeight::Bold)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Text::new("Click a box, press Tab, or use the buttons.")
                            .text_style(
                                TextStyle::new()
                                    .font_size(16)
                                    .color(crate::theme::muted_text(&app_theme)),
                            )
                            .boxed(),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(24))
                            .children([
                                self.focus_box(FocusBox::First),
                                self.focus_box(FocusBox::Second),
                            ])
                            .boxed(),
                        Text::new(format!("Focused: {owner}"))
                            .text_style(
                                TextStyle::new()
                                    .font_size(16)
                                    .color(app_theme.on_background_color),
                            )
                            .boxed(),
                        Row::new()
                            .gaps(LayoutSpacing::new().right(12))
                            .children([
                                self.focus_button(
                                    "Focus first",
                                    Some(self.first.clone()),
                                    app_theme,
                                ),
                                self.focus_button(
                                    "Focus second",
                                    Some(self.second.clone()),
                                    app_theme,
                                ),
                                self.focus_button("Clear focus", None, app_theme),
                            ])
                            .boxed(),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use aimer::quiver::winit::dpi::PhysicalPosition;
    use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};

    use super::*;

    type HeadlessApp<W> = aimer::quiver::aimer_app::HeadlessAimerApp<W>;

    /// Where the first box is drawn, in the test page below.
    const FIRST_BOX: (f64, f64) = (110.0, 70.0);

    /// Where the second box is drawn: one box plus the row's gap to the right.
    const SECOND_BOX: (f64, f64) = (110.0 + BOX_WIDTH as f64 + 24.0, 70.0);

    /// Somewhere on the page that is not a box at all.
    const NOWHERE: (f64, f64) = (110.0, BOX_HEIGHT as f64 + 200.0);

    /// Another such place.
    ///
    /// A cursor that has not actually moved reports nothing, so a test that
    /// needs a second pointer event has to move the pointer somewhere else.
    const NOWHERE_ELSE: (f64, f64) = (300.0, BOX_HEIGHT as f64 + 200.0);

    /// One box of a test page: a focusable target of a known size.
    fn test_box(node: &FocusNode, report: impl Fn(bool) + 'static) -> AnyWidget {
        Focusable::new()
            .node(node.clone())
            .on_focus_change(report)
            .box_child(
                Container::new()
                    .width(Dimension::Px(BOX_WIDTH))
                    .height(Dimension::Px(BOX_HEIGHT))
                    .box_decoration(box_decoration(false))
                    .box_child(
                        SizedBox::new()
                            .width(1)
                            .height(1),
                    ),
            )
    }

    /// Two boxes in the top-left corner, and empty space below them.
    ///
    /// The example's own page is deliberately not used: its heading and hints
    /// would put the boxes wherever text happens to wrap, and these tests need
    /// to know where to press.
    fn page(first: AnyWidget, second: AnyWidget) -> impl Widget + 'static {
        Container::new()
            .color(Colors::White.into())
            .box_child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .vertical_alignment(BoxAlignment::Start)
                    .children([
                        Row::new()
                            .gaps(LayoutSpacing::new().right(24))
                            .children([first, second])
                            .boxed(),
                        SizedBox::new()
                            .width(400)
                            .height(400)
                            .boxed(),
                    ]),
            )
    }

    /// That page, with nothing listening for focus changes.
    fn two_boxes(first: &FocusNode, second: &FocusNode) -> impl Widget + 'static {
        page(test_box(first, |_| {}), test_box(second, |_| {}))
    }

    /// The example's own pattern in miniature: the nodes live in the state, and
    /// every focus change rebuilds the whole page.
    #[widget(Stateful)]
    struct RebuildingBoxes {
        first: FocusNode,
        second: FocusNode,
        builds: Rc<Cell<usize>>,
    }

    struct RebuildingBoxesState {
        first: FocusNode,
        second: FocusNode,
        builds: Rc<Cell<usize>>,
        focused: Option<FocusBox>,
        updater: StateUpdater<Self>,
    }

    impl StatefulWidget for RebuildingBoxes {
        type State = RebuildingBoxesState;

        fn create_state(self) -> RebuildingBoxesState {
            RebuildingBoxesState {
                first: self.first,
                second: self.second,
                builds: self.builds,
                focused: None,
                updater: StateUpdater::empty(),
            }
        }
    }

    impl State<RebuildingBoxes> for RebuildingBoxesState {
        fn init_state(&mut self, updater: StateUpdater<Self>)
        where
            Self: Sized,
        {
            self.updater = updater;
        }

        fn build(&self, _: &BuildContext) -> impl Widget {
            self.builds
                .set(self.builds.get() + 1);
            let reporter = |which: FocusBox, updater: StateUpdater<Self>| {
                move |gained: bool| {
                    updater.set_state(move |state| state.focused = gained.then_some(which));
                }
            };
            page(
                test_box(&self.first, reporter(FocusBox::First, self.updater)),
                test_box(&self.second, reporter(FocusBox::Second, self.updater)),
            )
        }
    }

    /// Moves the pointer, which is enough to make the framework resolve any
    /// focus request that was made without one.
    fn move_to<W: Widget + 'static>(app: &mut HeadlessApp<W>, (x, y): (f64, f64)) {
        app.send_window_event(WindowEvent::CursorMoved {
            device_id: DeviceId::dummy(),
            position: PhysicalPosition::new(x, y),
        });
        app.render_frame();
    }

    fn press<W: Widget + 'static>(app: &mut HeadlessApp<W>, at: (f64, f64)) {
        move_to(app, at);
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Pressed,
            button: MouseButton::Left,
        });
        app.render_frame();
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state: ElementState::Released,
            button: MouseButton::Left,
        });
        app.render_frame();
    }

    /// A button that asks for focus is obeyed by the click that pressed it.
    ///
    /// The interesting part is that nothing happens afterwards: the press lands
    /// on a button, which is not focusable and therefore drops focus, and the
    /// request the handler recorded has to be granted before the pointer moves
    /// again — or the example's buttons would appear to do nothing at all.
    #[test]
    fn a_button_that_requests_focus_is_obeyed_within_the_click() {
        let box_node = FocusNode::new();
        let button = Container::new()
            .width(Dimension::Px(BOX_WIDTH))
            .height(Dimension::Px(BOX_HEIGHT))
            .box_child(
                Button::new()
                    .on_press({
                        let box_node = box_node.clone();
                        move || box_node.request_focus()
                    })
                    .child(
                        SizedBox::new()
                            .width(BOX_WIDTH)
                            .height(BOX_HEIGHT),
                    ),
            );
        let mut app = AimerApp::start_headless(page(test_box(&box_node, |_| {}), button.boxed()));
        app.render_frame();
        app.render_frame();

        press(&mut app, SECOND_BOX);

        assert!(
            box_node.has_focus(),
            "the request must be granted without waiting for another event"
        );
    }

    /// The showcase itself builds and lays out.
    ///
    /// The tests above use a page of their own, so this is the only thing that
    /// exercises the example as it is actually shipped.
    #[test]
    fn the_example_page_renders() {
        let mut app = AimerApp::start_headless(FocusNodeExample::new());
        app.render_frame();
        app.render_frame();

        press(&mut app, NOWHERE);
    }

    /// The border is the only difference between the two states, so a box
    /// cannot move when it gains focus.
    #[test]
    fn only_a_focused_box_draws_a_border() {
        let focused = box_decoration(true);
        let idle = box_decoration(false);

        assert_ne!(focused.border, idle.border, "the border must be the tell");
        assert_eq!(
            focused.background_color, idle.background_color,
            "nothing but the border may change"
        );
        assert_eq!(
            focused
                .border
                .top
                .stroke,
            idle.border
                .top
                .stroke,
            "an equally thick border in both states keeps the child still"
        );
    }

    /// Pressing a box gives it the keyboard, and pressing the page takes it
    /// away again — neither of which this example implements itself.
    #[test]
    fn pressing_a_box_focuses_it_and_pressing_elsewhere_clears_it() {
        let first = FocusNode::new();
        let second = FocusNode::new();
        let mut app = AimerApp::start_headless(two_boxes(&first, &second));
        app.render_frame();
        app.render_frame();

        press(&mut app, FIRST_BOX);
        assert!(
            first.has_focus(),
            "the box under the press owns the keyboard"
        );
        assert!(!second.has_focus());

        press(&mut app, SECOND_BOX);
        assert!(second.has_focus(), "focus follows the press");
        assert!(!first.has_focus(), "and only one target may own it");

        press(&mut app, NOWHERE);
        assert!(
            !first.has_focus() && !second.has_focus(),
            "a press on nothing focusable is how focus is dropped"
        );
    }

    /// Focus survives the rebuild that reacting to it causes.
    ///
    /// This is why the node belongs to the state: the page that reports the
    /// focus change is rebuilt because of it, every widget in it is thrown
    /// away, and the freshly built one must offer the *same* target — or the
    /// border would appear and vanish in the same breath.
    #[test]
    fn a_box_keeps_the_focus_across_the_rebuild_it_triggered() {
        let first = FocusNode::new();
        let second = FocusNode::new();
        let builds = Rc::new(Cell::new(0));
        let mut app = AimerApp::start_headless(RebuildingBoxes {
            first: first.clone(),
            second: second.clone(),
            builds: builds.clone(),
        });
        app.render_frame();
        app.render_frame();
        let before = builds.get();

        press(&mut app, FIRST_BOX);

        assert!(
            builds.get() > before,
            "the focus change must have rebuilt the page, or this proves nothing"
        );
        assert!(
            first.has_focus(),
            "the node lives in the state, so the rebuilt page offers the same target"
        );
        assert!(!second.has_focus());
    }

    /// The imperative half of the handle: asking for focus without a pointer.
    #[test]
    fn a_node_can_ask_for_focus_and_give_it_back() {
        let first = FocusNode::new();
        let second = FocusNode::new();
        let mut app = AimerApp::start_headless(two_boxes(&first, &second));
        app.render_frame();
        app.render_frame();

        second.request_focus();
        move_to(&mut app, NOWHERE);

        assert!(
            second.has_focus(),
            "a request is granted while the tree synchronizes, pointer or not"
        );

        second.unfocus();
        move_to(&mut app, NOWHERE_ELSE);

        assert!(!second.has_focus(), "and can be handed back the same way");
        assert!(!first.has_focus(), "and is not passed on to anybody else");
    }
}
