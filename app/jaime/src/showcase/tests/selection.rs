//! Choosing an example in the sidebar: the content pane follows the selection
//! while the rest of the shell stays as it was.

use super::set_state_profile::{Captured, Labels, click_at};
use super::*;

use std::rc::Rc;

use aimer::HeadlessOptions;
use aimer::modal::ModalHost;
use aimer::quiver::winit::dpi::PhysicalSize;

/// Logical width of the sidebar; labels to its right belong to the page.
const SIDEBAR_LOGICAL_WIDTH: f32 = 292.0;

struct Scene<W: Widget + 'static> {
    app: aimer::quiver::HeadlessAimerApp<ModalHost<W>>,
    labels: Rc<Labels>,
}

fn scene() -> Scene<impl Widget + 'static> {
    let labels = Rc::new(Labels::default());
    // Tall enough that every sidebar row is laid out and clickable.
    let mut app = AimerApp::start_headless_with(
        theme::provide(Captured {
            labels: labels.clone(),
        }),
        HeadlessOptions {
            size: PhysicalSize::new(1280, 4200),
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

    fn sidebar_labels(&mut self) -> Vec<String> {
        self.texts()
            .into_iter()
            .filter(|(_, x, _)| *x < SIDEBAR_LOGICAL_WIDTH)
            .map(|(text, ..)| text)
            .collect()
    }

    /// The page title: the one text in the content pane that names an example.
    fn page_titles(&mut self) -> Vec<&'static str> {
        let texts = self.texts();
        EXAMPLES
            .iter()
            .map(|example| example.label())
            .filter(|label| {
                texts
                    .iter()
                    .any(|(text, x, _)| text == label && *x >= SIDEBAR_LOGICAL_WIDTH)
            })
            .collect()
    }

    fn click_row(&mut self, example: ExampleId) {
        let (_, x, y) = self
            .texts()
            .into_iter()
            .find(|(text, x, _)| text == example.label() && *x < SIDEBAR_LOGICAL_WIDTH)
            .unwrap_or_else(|| panic!("no sidebar row for {:?}", example.label()));
        click_at(&mut self.app, x, y, 1.0);
        self.app.pump_frames(4);
    }
}

#[test]
fn clicking_a_row_shows_that_example() {
    let mut scene = scene();
    assert_eq!(scene.page_titles(), [EXAMPLES[0].label()]);

    for example in [EXAMPLES[3], EXAMPLES[7], EXAMPLES[0]] {
        scene.click_row(example);
        assert_eq!(
            scene.page_titles(),
            [example.label()],
            "the content pane must follow the clicked row"
        );
    }
}

#[test]
fn stack_positioned_and_align_pages_are_selectable() {
    let mut scene = scene();
    for (label, samples) in [
        ("Stack + Positioned", ["Top / left", "Bottom / right", "Layer 2"]),
        ("Align", ["TopLeft", "MidCenter", "BotRight"]),
    ] {
        let example = EXAMPLES.iter().copied().find(|example| example.label() == label)
            .unwrap_or_else(|| panic!("{label} must be selectable in Jaime"));
        scene.click_row(example);
        assert_eq!(scene.page_titles(), [label]);
        scene.texts();
        let texts = scene.labels.painted_texts.borrow();
        for sample in samples {
            assert!(texts.iter().any(|text| text == sample),
                "{label} must show the {sample} sample in the content pane: {texts:?}");
        }
    }
}

#[test]
fn choosing_an_example_keeps_the_whole_sidebar() {
    let mut scene = scene();
    let before = scene.sidebar_labels();
    assert!(before.len() > EXAMPLES.len(), "every row and header is laid out");

    scene.click_row(EXAMPLES[5]);

    assert_eq!(scene.sidebar_labels(), before);
}

#[test]
fn choosing_the_selected_example_again_changes_nothing() {
    let mut scene = scene();
    scene.click_row(EXAMPLES[4]);
    let sidebar = scene.sidebar_labels();

    scene.click_row(EXAMPLES[4]);

    assert_eq!(scene.page_titles(), [EXAMPLES[4].label()]);
    assert_eq!(scene.sidebar_labels(), sidebar);
}

#[test]
fn choosing_an_example_never_rebuilds_the_shell() {
    let mut scene = scene();
    let mounted = SHELL_BUILDS.with(|builds| builds.get());
    assert_eq!(mounted, 1, "the shell builds once when it mounts");

    for example in [EXAMPLES[2], EXAMPLES[6], EXAMPLES[6], EXAMPLES[0]] {
        scene.click_row(example);
    }

    assert_eq!(
        SHELL_BUILDS.with(|builds| builds.get()),
        mounted,
        "a selection change must rebuild only the rows and the content pane"
    );
}
