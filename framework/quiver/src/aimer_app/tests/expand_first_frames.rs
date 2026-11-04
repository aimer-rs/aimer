//! The first frames of an expanding push, read from the render tree.
//!
//! A transition adds layers to its host in the frame it starts, and a layer
//! that is shown before it has been given its window shows as a rectangle the
//! size of the host. The backing is the layer to watch: it must be clipped to
//! the growing window in every frame it is drawn, the first ones included.

use super::*;

use std::cell::RefCell;
use std::time::Duration;

use aimer_cupid::draw_cmd::{DrawCommand as LocalCommand, RenderOp};
use aimer_router::{ExpandTransition, Navigator, NavigatorController, Route, Router};
use aimer_widget::{AnyWidget, Key, StatelessElement};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Page {
    Home,
    Post,
}

impl Route for Page {
    fn parse(_: &str) -> Option<Self> { None }
    fn format(&self) -> String { format!("/{self:?}") }
}

impl Router for Page {
    fn build(&self, _: &BuildContext) -> AnyWidget { routes(*self) }
}

aimer_tls::thread_local! {
    static CONTROLLER: RefCell<Option<NavigatorController<Page>>> = const { RefCell::new(None) };
}

/// A card among other rows, so the window has something to grow from and over.
const CARD: (u8, u8, u8) = (200, 40, 40);
const BACKING: (u8, u8, u8) = (10, 200, 10);

/// The home screen: a card among other rows, so the window has something to grow from and over.
struct Home;

impl Widget for Home {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatelessElement::from_builder(
            ctx,
            |ctx| {
                CONTROLLER.with(|slot| *slot.borrow_mut() = Some(NavigatorController::<Page>::of(ctx)));
                Column::new()
                    .children(vec![
                        SizedBox::new().width(Dimension::Px(300.0)).height(Dimension::Px(60.0)).color(Color::Rgb(30, 30, 30)).boxed(),
                        aimer_animation::SharedElement::new()
                            .tag("expand-card")
                            .child_with(|| {
                                SizedBox::new().width(Dimension::Px(300.0)).height(Dimension::Px(120.0))
                                    .color(Color::Rgb(CARD.0, CARD.1, CARD.2))
                            })
                            .boxed(),
                        SizedBox::new().width(Dimension::Px(300.0)).height(Dimension::Px(200.0)).color(Color::Rgb(60, 60, 200)).boxed(),
                    ])
                    .to_element(ctx)
            },
            None,
            "ExpandHome",
        )
        .boxed()
    }
}

fn routes(page: Page) -> AnyWidget {
    match page {
        Page::Home => Home.boxed(),
        Page::Post => SizedBox::new()
            .width(Dimension::Percent(100.0))
            .height(Dimension::Percent(100.0))
            .color(Color::Rgb(250, 250, 250))
            .boxed(),
    }
}

/// The rectangles the backing shows in this frame, in the window: what it records, as the
/// node that carries it is presented.
fn backing_rects<W: Widget + 'static>(app: &HeadlessAimerApp<W>) -> Vec<Rect> {
    app.app.render_tree().render_all().into_iter().filter_map(|operation| match operation {
        RenderOp::Draw(item) if item.opacity > 0.0 => {
            item.snapshot().commands.iter().find_map(|command| match command {
                LocalCommand::FillRect { rect, color, .. } if (color.r, color.g, color.b) == BACKING => {
                    let (left, top) = item.transform.transform_point(item.origin.0 + rect.x, item.origin.1 + rect.y);
                    let (right, bottom) = item.transform.transform_point(
                        item.origin.0 + rect.x + rect.width, item.origin.1 + rect.y + rect.height);
                    Some(Rect::new(left, top, right - left, bottom - top))
                }
                _ => None,
            })
        }
        _ => None,
    }).collect()
}

#[test]
fn the_backing_of_an_expanding_push_is_only_the_window_in_every_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let transition = ExpandTransition::new()
        .tag(|page: &Page| (*page == Page::Post).then(|| Key::from("expand-card")))
        .duration(Duration::from_millis(500))
        .background(Color::Rgb(BACKING.0, BACKING.1, BACKING.2))
        .crossfade(true)
        .blur(8.0);
    let mut app = AimerApp::start_headless_with(
        Navigator::new(Page::Home, routes).transition(transition),
        HeadlessOptions { size: PhysicalSize::new(300, 400), scale_factor: 1.0 },
    );
    app.pump_frames(4);
    assert!(backing_rects(&app).is_empty(), "nothing is backed before the push");

    CONTROLLER.with(|slot| slot.borrow().clone().expect("the home screen was built")).push(Page::Post);

    let mut windows = Vec::new();
    for frame in 0..40 {
        app.pump_frames(1);
        std::thread::sleep(Duration::from_millis(8));
        let rects = backing_rects(&app);
        assert!(rects.len() <= 1, "frame {frame}: one backing, not {rects:?}");
        if let Some(rect) = rects.first() { windows.push((frame, *rect)); }
    }
    assert!(!windows.is_empty(), "the backing is drawn while the window grows");
    let (first, start) = windows[0];
    // It starts where the card is (60 down, 300 wide, 120 high), a little way into its growth:
    // never the whole host, which is what a layer that had lost its clip would be.
    assert!(start.height < 200.0 && start.width <= 300.5, "frame {first}: the backing starts as the card, not the host: {start:?}");
    assert!((start.y - 60.0).abs() < 12.0, "frame {first}: at the card's top: {start:?}");
    let mut last_height = 0.0;
    for (frame, rect) in &windows {
        assert!(rect.height + 0.01 >= last_height, "frame {frame}: the window only grows: {rect:?}");
        assert!(rect.y >= -0.01 && rect.y + rect.height <= 400.5, "frame {frame}: inside the host: {rect:?}");
        last_height = rect.height;
    }
}
