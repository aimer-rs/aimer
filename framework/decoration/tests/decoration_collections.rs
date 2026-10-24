//! `Vec<D>`, `[D]` and `[D; N]` are decorations that paint their layers in order.

use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::material::MaterialRequest;
use aimer_decoration::{
    AnyDecor, BorderRadius, BoxShadow, DecorationBox, Decoration, LightEdge, Magnification,
    MultiDecorationBox,
};

const SIZE: ResolvedSize = ResolvedSize { width: 80.0, height: 40.0 };

fn record(paint: impl FnOnce(&Canvas)) -> Vec<DrawCommand> {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    paint(&canvas);
    canvas.finish();
    tree.draw_list_snapshot(root).unwrap().commands.to_vec()
}

fn fill_reds(commands: &[DrawCommand]) -> Vec<u8> {
    commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::FillRect { color, .. } => Some(color.r),
            _ => None,
        })
        .collect()
}

fn material(commands: &[DrawCommand]) -> MaterialRequest {
    let data = commands
        .iter()
        .find_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => Some(data),
            _ => None,
        })
        .expect("a material packet");
    MaterialRequest::decode(data).unwrap()
}

/// Deliberately non-`Clone`; every answer is configurable.
#[derive(Default)]
struct Layer {
    outsets: [f32; 4],
    insets: [f32; 4],
    clip: Option<[f32; 4]>,
    background: Option<Color>,
    empty: bool,
    revision: Rc<Cell<u64>>,
    calls: Rc<Cell<u8>>,
}

impl Decoration for Layer {
    fn paint(&self, _: &Canvas, _: Vec2d, _: ResolvedSize, _: f32) {
        self.calls.set(self.calls.get() | 1);
    }
    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        self.calls.set(self.calls.get() | 2);
        self.paint(canvas, origin, size, scale);
        canvas.add_background_color(self.background);
    }
    fn paint_with_background(
        &self, _: &Canvas, _: Vec2d, _: ResolvedSize, _: f32, _: Option<Color>,
    ) {
        self.calls.set(self.calls.get() | 4);
    }
    fn paint_outsets(&self, _: ResolvedSize, _: f32) -> [f32; 4] { self.outsets }
    fn content_insets(&self, _: ResolvedSize, _: f32) -> [f32; 4] { self.insets }
    fn child_clip_radius(&self, _: ResolvedSize, _: f32) -> Option<[f32; 4]> { self.clip }
    fn background_color(&self) -> Option<Color> { self.background }
    fn is_empty(&self) -> bool { self.empty }
    fn revision(&self) -> u64 { self.revision.get() }
}

#[test]
fn vec_slice_and_array_paint_their_layers_in_order() {
    let layers = || [Color::Rgb(1, 0, 0).notype(), Color::Rgb(2, 0, 0).notype()];
    let origin = Vec2d { x: 3.0, y: 4.0 };
    let from_vec = record(|c| Vec::from(layers()).paint(c, origin, SIZE, 1.0));
    let from_array = record(|c| layers().paint(c, origin, SIZE, 1.0));
    let from_slice = record(|c| layers()[..].paint(c, origin, SIZE, 1.0));
    let from_box = record(|c| Box::<[AnyDecor]>::from(layers()).paint(c, origin, SIZE, 1.0));
    for commands in [from_vec, from_array, from_slice, from_box] {
        assert_eq!(fill_reds(&commands), [1, 2]);
    }
    let single = record(|c| vec![Color::Rgb(7, 0, 0)].paint(c, origin, SIZE, 1.0));
    assert_eq!(fill_reds(&single), [7], "typed layers need no erasure");
    assert!(record(|c| Vec::<AnyDecor>::new().paint(c, origin, SIZE, 1.0)).is_empty());
}

#[test]
fn a_stack_propagates_each_layers_background_exactly_once() {
    // The automatic rim is dark over a light surface, so a dark rim proves the
    // preceding layer's background reached it.
    let light = vec![Color::Rgb(250, 250, 250).notype(), LightEdge::new().notype()];
    let commands = record(|c| light.paint_layer(c, Vec2d::default(), SIZE, 1.0));
    assert_eq!(material(&commands).border_color[..3], [0.0; 3], "dark rim on a light surface");
    assert_eq!(
        light.background_color(),
        Some(Color::Rgb(250, 250, 250)),
        "the stack reports the composited background"
    );
    let translucent = vec![Color::Rgb(255, 255, 255), Color::Rgba(0, 0, 0, 128)];
    assert_eq!(translucent.background_color(), Some(Color::Rgba(127, 127, 127, 255)));
    assert_eq!(Vec::<AnyDecor>::new().background_color(), None);
}

#[test]
fn a_stack_unions_outsets_and_ignores_nonfinite_values() {
    let layers = vec![
        Layer { outsets: [1.0, 5.0, 2.0, 0.0], ..Layer::default() },
        Layer { outsets: [4.0, 1.0, f32::NAN, f32::INFINITY], ..Layer::default() },
    ];
    assert_eq!(layers.paint_outsets(SIZE, 1.0), [4.0, 5.0, 2.0, 0.0]);
    assert_eq!(Vec::<Layer>::new().paint_outsets(SIZE, 1.0), [0.0; 4]);
    let shadow = vec![BoxShadow::new().blur(8.0).notype(), BorderRadius::from(4).notype()];
    assert!(shadow.paint_outsets(SIZE, 1.0).iter().all(|outset| *outset >= 0.0));
}

#[test]
fn a_stack_reserves_the_largest_inset_and_clips_to_the_tightest_radius() {
    let layers = vec![
        Layer { insets: [1.0, 5.0, 2.0, 0.0], clip: Some([4.0, 0.0, 2.0, 1.0]), ..Layer::default() },
        Layer { insets: [3.0, 1.0, f32::NAN, 6.0], clip: Some([1.0, 8.0, 0.0, 1.0]), ..Layer::default() },
        Layer::default(),
    ];
    assert_eq!(layers.content_insets(SIZE, 1.0), [3.0, 5.0, 2.0, 6.0]);
    assert_eq!(layers.child_clip_radius(SIZE, 1.0), Some([4.0, 8.0, 2.0, 1.0]));
    let none = vec![Layer::default(), Layer::default()];
    assert_eq!(none.child_clip_radius(SIZE, 1.0), None);
    assert_eq!(none.content_insets(SIZE, 1.0), [0.0; 4]);
    assert_eq!(Vec::<Layer>::new().child_clip_radius(SIZE, 1.0), None);
    let rounded = vec![BorderRadius::from(8).notype()];
    assert_eq!(rounded.child_clip_radius(SIZE, 2.0), Some([8.0; 4]));
}

#[test]
fn a_stack_is_empty_only_when_every_layer_is() {
    // `Vec::is_empty` and `<[T]>::is_empty` are inherent length checks, so name the trait.
    assert!(Decoration::is_empty(&Vec::<AnyDecor>::new()));
    let empty = || Layer { empty: true, ..Layer::default() };
    assert!(Decoration::is_empty(&vec![empty(), empty()]));
    assert!(Decoration::is_empty(&[empty(), empty()][..]));
    assert!(!Decoration::is_empty(&[empty(), Layer::default()]));
    assert!(!Decoration::is_empty(&vec![Layer::default()]));
}

#[test]
fn a_stack_revision_tracks_every_layer_by_position() {
    let a = Rc::new(Cell::new(1));
    let b = Rc::new(Cell::new(2));
    let layers = vec![
        Layer { revision: a.clone(), ..Layer::default() },
        Layer { revision: b.clone(), ..Layer::default() },
    ];
    let first = layers.revision();
    assert_eq!(layers.revision(), first, "unchanged layers keep their revision");
    a.set(9);
    let after_a = layers.revision();
    assert_ne!(after_a, first);
    b.set(9);
    let after_b = layers.revision();
    assert_ne!(after_b, after_a);
    // Swapping two different revisions is a change, not a no-op.
    a.set(3);
    b.set(4);
    let ab = layers.revision();
    a.set(4);
    b.set(3);
    assert_ne!(layers.revision(), ab);
    let empty = Vec::<Layer>::new();
    assert_eq!(empty.revision(), empty.revision());
}

#[test]
fn a_container_fallback_background_goes_behind_the_first_layer_only() {
    let calls: Vec<_> = (0..3).map(|_| Rc::new(Cell::new(0))).collect();
    let layers: Vec<_> = calls
        .iter()
        .map(|calls| Layer { calls: calls.clone(), ..Layer::default() })
        .collect();
    record(|c| layers.paint_with_background(c, Vec2d::default(), SIZE, 1.0, Some(Color::RED)));
    assert_eq!(calls.iter().map(|c| c.get()).collect::<Vec<_>>(), [4, 3, 3]);
    for call in &calls {
        call.set(0);
    }
    record(|c| layers.paint_with_background(c, Vec2d::default(), SIZE, 1.0, None));
    assert_eq!(calls.iter().map(|c| c.get()).collect::<Vec<_>>(), [3, 3, 3]);
    // A stack that supplies its own background has no use for the fallback.
    let opaque = vec![Layer { background: Some(Color::BLUE), ..Layer::default() }];
    let commands = record(|c| {
        opaque.paint_with_background(c, Vec2d::default(), SIZE, 1.0, Some(Color::RED))
    });
    assert!(fill_reds(&commands).is_empty());
    // With no layers the fallback still fills the box.
    let commands = record(|c| {
        Vec::<AnyDecor>::new()
            .paint_with_background(c, Vec2d::default(), SIZE, 1.0, Some(Color::Rgb(5, 0, 0)))
    });
    assert_eq!(fill_reds(&commands), [5]);
}

#[test]
fn border_radius_in_a_stack_shapes_the_lens_and_the_rim() {
    let stack = vec![
        BorderRadius::from(12.0).notype(),
        Magnification::new().notype(),
        LightEdge::new().notype(),
    ];
    let commands = record(|c| stack.paint_layer(c, Vec2d::default(), SIZE, 2.0));
    let packets: Vec<_> = commands
        .iter()
        .filter_map(|command| match command {
            DrawCommand::DrawCustom { data, .. } => Some(MaterialRequest::decode(data).unwrap()),
            _ => None,
        })
        .collect();
    assert_eq!(packets.len(), 2);
    for packet in packets {
        assert_eq!(packet.corner_radii, [12.0; 4]);
    }
    // The same stack, shaped by the radius declared last, stays square.
    let late = [Magnification::new().notype(), BorderRadius::from(12.0).notype()];
    let commands = record(|c| late.paint_layer(c, Vec2d::default(), SIZE, 2.0));
    assert_eq!(material(&commands).corner_radii, [0.0; 4]);
}

#[test]
fn collections_are_accepted_by_decoration_widgets() {
    fn accepts_widget(_widget: impl aimer_widget::Widget) {}
    let child = || aimer_container::SizedBox::new().width(80).height(40);
    let layers = || vec![BorderRadius::from(8).notype(), Magnification::new().notype()];
    accepts_widget(DecorationBox::new().decoration(layers()).child(child()));
    accepts_widget(MultiDecorationBox::new().decoration(layers()).child(child()));
    accepts_widget(DecorationBox::new().decoration([Color::BLUE.notype()]).child(child()));
    accepts_widget(MultiDecorationBox::new().decorations(layers()).child(child()));
}
