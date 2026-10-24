//! Choosing a card on the morph page keeps the outgoing card and hands both
//! pictures to the renderer as a morph group, which goes away once it is done.

use super::*;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use aimer::quiver::winit::dpi::{PhysicalPosition, PhysicalSize};
use aimer::quiver::winit::event::{DeviceId, ElementState, MouseButton, WindowEvent};
use aimer::{
    Drawable, Element, EventElement, HeadlessOptions, LayoutElement, Rebuildable, VisitorElement,
};
use aimer_cupid::draw_cmd_v2::{MorphGroup, MorphParams, MorphStyle, Rect, RenderOp};

#[derive(Default)]
struct Snapshot {
    /// Every button on the page, in build order: the three cards, the five
    /// styles, then the two headers.
    buttons: Vec<Rect>,
}

struct Captured(Rc<RefCell<Snapshot>>);
impl aimer::PortableWidget for Captured {}
impl Widget for Captured {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        let example = EXAMPLES
            .iter()
            .copied()
            .find(|example| example.key() == "morph-transition")
            .expect("Morph transition must be selectable in Jaime");
        CapturedElement {
            child: build_example(example, theme::app_theme()).to_element(ctx),
            snapshot: self.0,
        }
        .boxed()
    }
}

struct CapturedElement {
    child: AnyElement,
    snapshot: Rc<RefCell<Snapshot>>,
}
impl VisitorElement for CapturedElement {
    fn debug_name(&self) -> &'static str {
        "CapturedMorphTransition"
    }
    fn visit_children<'a>(&'a self, visitor: &mut dyn FnMut(&'a dyn Element)) {
        visitor(self.child.as_ref());
    }
}
impl EventElement for CapturedElement {}
impl Rebuildable for CapturedElement {
    fn rebuild_if_dirty(&self, ctx: &BuildContext) {
        self.child.rebuild_if_dirty(ctx);
    }
}
impl LayoutElement for CapturedElement {
    fn computed_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.computed_size(ctx)
    }
    fn content_size(&self, ctx: &BuildContext) -> ResolvedSize {
        self.child.content_size(ctx)
    }
}
impl Drawable for CapturedElement {
    fn can_paint_local_v2(&self, _ctx: &BuildContext) -> bool {
        true
    }
    fn paint_local_v2(&self, ctx: &BuildContext) {
        aimer::canvas::Canvas::of(ctx).finish();
    }
    fn update(&self, ctx: &BuildContext) {
        self.child.update(ctx);
        let mut buttons = Vec::new();
        collect_buttons(self.child.as_ref(), &mut buttons);
        self.snapshot.replace(Snapshot { buttons });
    }
}

fn collect_buttons(element: &dyn Element, buttons: &mut Vec<Rect>) {
    if element.debug_name() == "GestureDetector" {
        let (start, end) = element
            .pos_start_end()
            .expect("a visible control has interaction bounds");
        buttons.push(Rect::new(
            start.x,
            start.y,
            end.x - start.x,
            end.y - start.y,
        ));
    }
    element.visit_children(&mut |child| collect_buttons(child, buttons));
}

fn click(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>, bounds: Rect) {
    app.send_window_event(WindowEvent::CursorMoved {
        device_id: DeviceId::dummy(),
        position: PhysicalPosition::new(
            f64::from(bounds.x + bounds.width * 0.5),
            f64::from(bounds.y + bounds.height * 0.5),
        ),
    });
    app.render_frame();
    for state in [ElementState::Pressed, ElementState::Released] {
        app.send_window_event(WindowEvent::MouseInput {
            device_id: DeviceId::dummy(),
            state,
            button: MouseButton::Left,
        });
        app.pump_frames(2);
    }
}

/// The morph hosts in the frame's plan, with the source groups inside them.
fn morphs(app: &aimer::HeadlessAimerApp<impl Widget + 'static>) -> Vec<(MorphParams, usize)> {
    let mut hosts: Vec<(MorphParams, usize)> = Vec::new();
    for operation in app.render_ops() {
        if let RenderOp::BeginOpacityGroup { morph, .. } = operation {
            match morph {
                Some(MorphGroup::Host(params)) => hosts.push((params, 0)),
                Some(MorphGroup::From | MorphGroup::To) => {
                    hosts.last_mut().expect("a source sits in its host").1 += 1
                }
                None => {}
            }
        }
    }
    hosts
}

fn start() -> (
    aimer::HeadlessAimerApp<impl Widget + 'static>,
    Rc<RefCell<Snapshot>>,
) {
    let snapshot: Rc<RefCell<Snapshot>> = Rc::default();
    let mut app = AimerApp::start_headless_with(
        theme::provide(Captured(snapshot.clone())),
        HeadlessOptions {
            size: PhysicalSize::new(900, 800),
            scale_factor: 1.0,
        },
    );
    app.pump_frames(3);
    (app, snapshot)
}

/// Waits out the page's morph and lets the tree settle.
fn finish_morph(app: &mut aimer::HeadlessAimerApp<impl Widget + 'static>) {
    std::thread::sleep(Duration::from_millis(crate::morph_transition_example::MORPH_MS + 200));
    app.pump_frames(6);
}

#[test]
fn the_page_starts_without_a_morph() {
    let (app, snapshot) = start();

    assert_eq!(snapshot.borrow().buttons.len(), 10);
    assert!(morphs(&app).is_empty(), "the first card is shown as it is");
}

#[test]
fn choosing_a_card_morphs_to_it_and_then_settles() {
    let (mut app, snapshot) = start();
    let tall = snapshot.borrow().buttons[1];

    click(&mut app, tall);

    let running = morphs(&app);
    assert_eq!(running.len(), 1, "one morph while the card changes");
    assert_eq!(running[0].1, 2, "the outgoing and the incoming card");
    assert_eq!(running[0].0.style, MorphStyle::Dissolve);
    assert!(
        running[0].0.progress < 1.0,
        "the morph is still under way: {}",
        running[0].0.progress
    );

    finish_morph(&mut app);
    assert!(morphs(&app).is_empty(), "the outgoing card is dropped when done");
}

#[test]
fn the_chosen_style_reaches_the_renderer() {
    let (mut app, snapshot) = start();
    let buttons = snapshot.borrow().buttons.clone();
    let styles = [
        (3, MorphStyle::Dissolve),
        (4, MorphStyle::Wipe),
        (5, MorphStyle::Liquid),
        (6, MorphStyle::Bounds),
        (7, MorphStyle::Blur),
    ];
    // Alternate between two cards so every pick changes the child key.
    let mut next_card = 1;
    for (button, style) in styles {
        click(&mut app, buttons[button]);
        assert!(
            morphs(&app).is_empty(),
            "picking a style alone does not start a morph"
        );

        click(&mut app, buttons[next_card]);
        next_card = 1 - next_card;
        let running = morphs(&app);
        assert_eq!(running.len(), 1, "{style:?}");
        assert_eq!(running[0].0.style, style);
        finish_morph(&mut app);
    }
}

#[test]
fn choosing_the_shown_card_again_changes_nothing() {
    let (mut app, snapshot) = start();
    let compact = snapshot.borrow().buttons[0];

    click(&mut app, compact);

    assert!(morphs(&app).is_empty(), "the card did not change");
}

#[test]
fn the_navigation_bar_pushes_its_title_and_divides_its_controls() {
    let (mut app, snapshot) = start();
    let library = snapshot.borrow().buttons[9];

    click(&mut app, library);

    // Two morphs run, and the cards' is not one of them: the title is pushed
    // in from the left, the controls divide like a cell.
    let running = morphs(&app);
    assert_eq!(running.len(), 2, "{running:?}");
    let slide = running
        .iter()
        .find(|(params, _)| params.style == MorphStyle::Slide)
        .expect("the title slides");
    assert_eq!(slide.0.direction, [1.0, 0.0], "old out to the right, new in from the left");
    assert_eq!(slide.1, 2);
    let goo = running
        .iter()
        .find(|(params, _)| params.style == MorphStyle::Goo)
        .expect("the controls divide");
    assert_eq!(goo.0.blur, 14.0, "how far the shapes reach to join");
    assert_eq!(goo.1, 2);

    finish_morph(&mut app);
    assert!(morphs(&app).is_empty());
}

/// Renders the page's current render tree through Metal and writes it as a
/// binary PPM, so a transition can be looked at without a window.
#[cfg(target_os = "macos")]
fn capture(
    app: &aimer::HeadlessAimerApp<impl Widget + 'static>,
    renderer: &mut aimer_cupid::renderer::RendererImpl<aimer_cupid::backend::metal::MetalBackend>,
    backend: &aimer_cupid::backend::metal::MetalBackend,
    path: &std::path::Path,
    (width, height): (u32, u32),
) {
    use aimer_cupid::backend::*;
    use aimer_cupid::damage_region::DamageSet;
    use aimer_cupid::draw_cmd_v2::RenderFrame;
    use aimer_cupid::frame::{FramePacket, FrameRenderMetadata};

    let format = <aimer_cupid::backend::metal::MetalBackend as GpuBackend>::rgba8_unorm_format();
    let target = backend.create_texture(&TextureDescriptor {
        label: None,
        size: (width, height, 1),
        mip_level_count: 1,
        sample_count: 1,
        dimension: TextureDimension::D2,
        format,
        usage: vec![TextureUsage::RenderAttachment, TextureUsage::CopySrc],
    });
    let view = backend.create_texture_view(&target, "capture");
    let packet = FramePacket::from_v2_direct(
        RenderFrame {
            damage: vec![Rect::new(0.0, 0.0, width as f32, height as f32)],
            operations: app.render_ops(),
        },
        FrameRenderMetadata::new(1.0, 7, 1, 1, 1, DamageSet::new(width, height)),
    )
    .expect("lower the page");
    renderer.render_packet(backend, &view, &packet, false);

    let row = (width * 4).div_ceil(256) * 256;
    let readback = backend.create_buffer(&BufferDescriptor {
        label: None,
        size: u64::from(row) * u64::from(height),
        usage: vec![BufferUsage::CopyDst, BufferUsage::MapRead],
    });
    let mut encoder = backend.create_command_encoder("capture readback");
    backend.copy_texture_to_buffer(
        &mut encoder,
        &TexelCopyTextureInfo {
            texture: &target,
            mip_level: 0,
            origin: Origin3d::ZERO,
            aspect: TextureAspect::All,
        },
        &TexelCopyBufferInfo {
            buffer: &readback,
            layout: TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        Extent3d { width, height, depth_or_array_layers: 1 },
    );
    backend.submit(encoder);
    let padded = backend.read_buffer(&readback, u64::from(row) * u64::from(height));
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for line in padded.chunks(row as usize).take(height as usize) {
        for pixel in line[..(width * 4) as usize].chunks_exact(4) {
            ppm.extend_from_slice(&pixel[..3]);
        }
    }
    std::fs::write(path, ppm).expect("write the capture");
}

/// Writes frames of the header transition to `MORPH_CAPTURE_DIR`. Run with
/// `MORPH_CAPTURE_DIR=<dir> cargo nextest run -p jaime -j 1 -E 'test(capture_navigation_bar)' --run-ignored only`.
#[test]
#[ignore = "writes images for a person to look at"]
#[cfg(target_os = "macos")]
fn capture_navigation_bar_frames() {
    let dir = std::path::PathBuf::from(std::env::var("MORPH_CAPTURE_DIR").expect("MORPH_CAPTURE_DIR"));
    let backend = aimer_cupid::backend::metal::MetalBackend::new().expect("Metal");
    let format = <aimer_cupid::backend::metal::MetalBackend as aimer_cupid::backend::GpuBackend>::rgba8_unorm_format();
    let mut renderer = aimer_cupid::renderer::RendererImpl::with_antialiasing(
        &backend,
        format,
        aimer_cupid::AntiAlias::Analytic,
    );
    let (mut app, snapshot) = start();
    let size = (900, 800);
    capture(&app, &mut renderer, &backend, &dir.join("bar_00.ppm"), size);

    let library = snapshot.borrow().buttons[9];
    click(&mut app, library);
    let started = std::time::Instant::now();
    for frame in 1..=20 {
        capture(&app, &mut renderer, &backend, &dir.join(format!("bar_{frame:02}.ppm")), size);
        println!("frame {frame} at {} ms", started.elapsed().as_millis());
        std::thread::sleep(Duration::from_millis(45));
        app.render_frame();
    }
}
