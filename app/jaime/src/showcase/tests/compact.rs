//! On a phone-width window the showcase is two screens, items then item detail,
//! instead of two panes side by side.

use super::set_state_profile::{Captured, Labels, click_at};
use super::*;

use std::rc::Rc;

use aimer::HeadlessOptions;
use aimer::modal::ModalHost;
use aimer::quiver::winit::dpi::PhysicalSize;

struct Scene<W: Widget + 'static> {
    app: aimer::quiver::HeadlessAimerApp<ModalHost<W>>,
    labels: Rc<Labels>,
}

/// A showcase mounted `width` logical pixels wide. Tall enough that every row
/// is laid out and clickable.
fn scene(width: u32) -> Scene<impl Widget + 'static> {
    let labels = Rc::new(Labels::default());
    let mut app = AimerApp::start_headless_with(
        theme::provide(Captured {
            labels: labels.clone(),
        }),
        HeadlessOptions {
            size: PhysicalSize::new(width, 4200),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(8);
    Scene { app, labels }
}

impl<W: Widget + 'static> Scene<W> {
    /// Every text node currently laid out, as `(text, x, y)` in logical pixels.
    fn texts(&mut self) -> Vec<(String, f32, f32)> {
        self.labels.capture.set(true);
        self.app.render_frame();
        self.labels.capture.set(false);
        self.labels.found.borrow().clone()
    }

    fn shows(&mut self, text: &str) -> bool {
        self.texts()
            .iter()
            .any(|(shown, ..)| shown == text)
    }

    /// Lets a slide between the two screens run to its end. The effect follows
    /// the wall clock, so this waits for it rather than counting frames.
    fn settle(&mut self) {
        for _ in 0..60 {
            if self.app.pump_frames(8) == 0 {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        panic!("the slide never came to rest");
    }

    fn tap(&mut self, text: &str) {
        let (_, x, y) = self
            .texts()
            .into_iter()
            .find(|(shown, ..)| shown == text)
            .unwrap_or_else(|| panic!("nothing on screen reads {text:?}"));
        click_at(&mut self.app, x + 2.0, y + 2.0, 1.0);
        self.settle();
    }
}

#[test]
fn a_phone_opens_on_the_items_list_alone() {
    let mut scene = scene(390);

    assert!(scene.shows(EXAMPLES[3].label()), "the list is the first screen");
    assert!(
        !scene.shows(EXAMPLES[0].description()),
        "the detail must not share the screen with the list"
    );
    assert!(!scene.shows(BACK_LABEL), "the list has nothing to go back to");
}

#[test]
fn tapping_an_item_replaces_the_list_with_its_detail() {
    let mut scene = scene(390);

    scene.tap(EXAMPLES[3].label());
    scene.settle();

    assert!(scene.shows(EXAMPLES[3].description()), "the detail is on screen");
    assert!(scene.shows(BACK_LABEL), "the detail offers a way back");
    assert!(
        !scene.shows(EXAMPLES[7].label()),
        "the list must be gone while the detail is open"
    );
}

#[test]
fn back_returns_to_the_items_list() {
    let mut scene = scene(390);
    scene.tap(EXAMPLES[3].label());

    scene.tap(BACK_LABEL);

    assert!(scene.shows(EXAMPLES[7].label()), "the list is back");
    assert!(!scene.shows(EXAMPLES[3].description()), "the detail is gone");
}

#[test]
fn a_wide_window_keeps_both_panes() {
    let mut scene = scene(1280);

    assert!(scene.shows(EXAMPLES[3].label()));
    assert!(scene.shows(EXAMPLES[0].description()));
    assert!(!scene.shows(BACK_LABEL), "side by side needs no back button");
}
