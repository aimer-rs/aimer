use super::*;

use std::cell::UnsafeCell;
use std::time::Duration;

use aimer_animation::{AnimatedSwitcher, curves};
use aimer_assets::img_widget::image_widget::RawImageWidget;
use aimer_assets::{ImageProvider, ImageResult};
use aimer_cupid::draw_cmd_v2::{DrawCommand as LocalCommand, ImageResource, RenderOp};
use aimer_style::BoxFit;
use aimer_widget::LayoutCache;

#[derive(Clone, Debug)]
struct ReadyImage(Arc<ImageResource>);

impl ImageProvider for ReadyImage {
    fn get_image(&self, _ctx: &BuildContext) -> ImageResult {
        ImageResult::Success(self.0.texture_id())
    }

    fn retained_image_resource(&self, _ctx: &BuildContext) -> Option<Arc<ImageResource>> {
        Some(self.0.clone())
    }
}

impl Widget for ReadyImage {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawImageWidget {
            source: self,
            size: Size::new(Dimension::Auto, Dimension::Auto),
            cache: LayoutCache::new(),
            fit: BoxFit::None,
            keep_aspect_ratio: true,
            original_size: Cell::new(None),
            loading_element: None,
            error_element: None,
            cached_id: UnsafeCell::new(None),
            cached_texture_epoch: Cell::new(0),
            paint_changed: Cell::new(false),
            scale: 1.0,
        }
        .boxed()
    }
}
impl aimer_widget::PortableWidget for ReadyImage {}

struct ImagePage {
    updater: Rc<Cell<Option<StateUpdater<ImagePageState>>>>,
    images: [ReadyImage; 2],
}

struct ImagePageState {
    selected: usize,
    duration: Duration,
    updater: Rc<Cell<Option<StateUpdater<Self>>>>,
    images: [ReadyImage; 2],
}

impl StatefulWidget for ImagePage {
    type State = ImagePageState;

    fn create_state(self) -> Self::State {
        ImagePageState {
            selected: 0,
            duration: Duration::from_secs(86_400),
            updater: self.updater,
            images: self.images,
        }
    }
}

impl State<ImagePage> for ImagePageState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater.set(Some(updater));
    }

    fn build(&self, _ctx: &BuildContext) -> impl Widget {
        aimer_container::Container::new().height(80.0).child(
            AnimatedSwitcher::new(
                self.duration,
                curves::LINEAR,
                self.images[self.selected].clone(),
            )
            .child_key(if self.selected == 0 {
                "wide"
            } else {
                "portrait"
            })
            .key("platform-image-switcher"),
        )
    }
}

impl Widget for ImagePage {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        StatefulElement::new_with_name(self, ctx, "ImagePage", None)
            .0
            .boxed()
    }
}
impl aimer_widget::PortableWidget for ImagePage {}

struct SwitchDuringDraw {
    updater: Rc<Cell<Option<StateUpdater<ImagePageState>>>>,
    selected: Rc<Cell<Option<usize>>>,
    images: [ReadyImage; 2],
}

struct SwitchDuringDrawElement {
    updater: Rc<Cell<Option<StateUpdater<ImagePageState>>>>,
    selected: Rc<Cell<Option<usize>>>,
    child: AnyElement,
}

impl Widget for SwitchDuringDraw {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let child = ImagePage {
            updater: self.updater.clone(),
            images: self.images,
        }
        .to_element(ctx);
        SwitchDuringDrawElement {
            updater: self.updater,
            selected: self.selected,
            child,
        }
        .boxed()
    }
}
impl aimer_widget::PortableWidget for SwitchDuringDraw {}

impl VisitorElement for SwitchDuringDrawElement {
    fn debug_name(&self) -> &'static str {
        "SwitchDuringDraw"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for SwitchDuringDrawElement {}
impl Rebuildable for SwitchDuringDrawElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}
impl LayoutElement for SwitchDuringDrawElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }
}
impl Drawable for SwitchDuringDrawElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }
    fn paint_local_v2(&self, ctx: &BuildContext) {
        Canvas::of(ctx).finish();
    }
    fn update(&self, ctx: &BuildContext) {
        if let Some(selected) = self.selected.take() {
            self.updater
                .get()
                .unwrap()
                .set_state(move |state| state.selected = selected);
        }
        self.child.update(ctx);
    }
}

fn presented_images(tree: &aimer_cupid::draw_cmd_v2::RenderTree) -> Vec<(u32, f32, Rect)> {
    let mut opacity = vec![1.0];
    let mut images = Vec::new();
    for operation in tree.render_order(&[Rect::new(0.0, 0.0, 320.0, 240.0)]) {
        match operation {
            RenderOp::BeginOpacityGroup { opacity: value, .. } => {
                opacity.push(opacity.last().unwrap() * value);
            }
            RenderOp::EndOpacityGroup { .. } => {
                opacity.pop();
            }
            RenderOp::Draw(item) => {
                for command in item.snapshot().commands.iter() {
                    if let LocalCommand::DrawImageWithResource { resource, rect } = command {
                        images.push((
                            resource.texture_id(),
                            opacity.last().unwrap() * item.opacity,
                            *rect,
                        ));
                    }
                }
            }
        }
    }
    images
}

#[test]
fn returning_to_a_ready_image_starts_the_crossfade_without_a_flash_or_resize() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ready = |id, width, height| {
        ReadyImage(Arc::new(
            ImageResource::rgba8_with_intrinsic_size(
                id,
                1,
                1,
                1,
                width,
                height,
                Arc::from([255_u8; 4]),
            )
            .unwrap(),
        ))
    };
    let updater = Rc::new(Cell::new(None));
    let mut app = AimerApp::start_headless_with(
        ImagePage {
            updater: updater.clone(),
            images: [ready(1001, 400, 200), ready(1002, 100, 200)],
        },
        HeadlessOptions {
            size: PhysicalSize::new(320, 240),
            scale_factor: 1.0,
        },
    );
    app.render_frame();
    let updater = updater.get().unwrap();
    let initial = presented_images(app.app.render_tree());
    assert_eq!(initial.len(), 1);
    assert_eq!(initial[0].0, 1001);
    assert_eq!(initial[0].1, 1.0);

    for selected in [1, 0, 1, 0] {
        updater.set_state(move |state| {
            state.selected = selected;
            state.duration = Duration::from_secs(86_400);
        });
        for frame in 0..3 {
            app.render_frame();
            let images = presented_images(app.app.render_tree());
            assert_eq!(
                images.len(),
                2,
                "both ready images must be present on frame {frame}"
            );
            assert_eq!(images[0].0, 1001 + (1 - selected) as u32);
            assert!(
                images[0].1 > 0.999,
                "outgoing image must start visible: {images:?}"
            );
            assert_eq!(images[1].0, 1001 + selected as u32);
            assert!(
                images[1].1 < 0.001,
                "incoming image must start transparent: {images:?}"
            );
            let wide = images.iter().find(|image| image.0 == 1001).unwrap();
            assert_eq!(
                wide.2, initial[0].2,
                "returning to a ready image must preserve its geometry"
            );
        }
        // End the transition through configuration, without a wall-clock sleep.
        updater.set_state(|state| state.duration = Duration::ZERO);
        app.render_frame();
        app.render_frame();
        let images = presented_images(app.app.render_tree());
        assert_eq!(
            images.len(),
            1,
            "a completed transition must remove its outgoing image"
        );
        assert_eq!(images[0].0, 1001 + selected as u32);
        assert_eq!(images[0].1, 1.0);
    }
}

#[test]
fn a_ready_image_added_during_draw_starts_transparent_in_the_same_frame() {
    let _serial = VIRTUALIZED_RENDER_TEST_LOCK
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let ready = |id| {
        ReadyImage(Arc::new(
            ImageResource::rgba8(id, 1, 1, 1, Arc::from([255_u8; 4])).unwrap(),
        ))
    };
    let updater = Rc::new(Cell::new(None));
    let selected = Rc::new(Cell::new(None));
    let mut app = AimerApp::start_headless_with(
        SwitchDuringDraw {
            updater: updater.clone(),
            selected: selected.clone(),
            images: [ready(2001), ready(2002)],
        },
        HeadlessOptions {
            size: PhysicalSize::new(320, 240),
            scale_factor: 1.0,
        },
    );
    app.render_frame();
    let updater = updater.get().unwrap();
    for index in [1, 0] {
        updater.set_state(|state| state.duration = Duration::from_secs(86_400));
        selected.set(Some(index));
        app.render_frame();
        let images = presented_images(app.app.render_tree());
        assert_eq!(
            images.len(),
            2,
            "the same frame must resolve both ready images: {images:?}"
        );
        assert_eq!(images[0].0, 2001 + (1 - index) as u32);
        assert!(
            images[0].1 > 0.999,
            "the outgoing image starts visible: {images:?}"
        );
        assert_eq!(images[1].0, 2001 + index as u32);
        assert!(
            images[1].1 < 0.001,
            "the incoming image must not flash at full opacity: {images:?}"
        );
        updater.set_state(|state| state.duration = Duration::ZERO);
        app.render_frame();
        app.render_frame();
        assert_eq!(presented_images(app.app.render_tree()).len(), 1);
    }
}
