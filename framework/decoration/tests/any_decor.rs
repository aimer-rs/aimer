use std::cell::Cell;
use std::rc::Rc;

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_color::prelude::Color;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_decoration::{AnyDecor, BorderRadius, BoxShadow, Decoration, MultiDecorationBox};
use aimer_rubick::UiMemory;

// Deliberately non-Clone. Both storage modes must move and drop the payload once.
struct Probe<const N: usize> {
    drops: Rc<Cell<usize>>,
    revision: Rc<Cell<u64>>,
    calls: Rc<Cell<u8>>,
    payload: [u8; N],
}

impl<const N: usize> Decoration for Probe<N> {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        self.calls.set(self.calls.get() | 1);
        canvas.fill_color_rect(origin, size, Color::BLUE);
    }

    fn paint_layer(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32) {
        self.calls.set(self.calls.get() | 2);
        canvas.fill_color_rect(origin, size, Color::GREEN);
    }

    fn paint_with_background(
        &self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, _scale: f32,
        background: Option<Color>,
    ) {
        self.calls.set(self.calls.get() | 4);
        canvas.fill_color_rect(origin, size, background.unwrap_or(Color::RED));
    }

    fn paint_outsets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] { [1.0, 2.0, 3.0, 4.0] }
    fn content_insets(&self, _size: ResolvedSize, _scale: f32) -> [f32; 4] { [5.0, 6.0, 7.0, 8.0] }
    fn child_clip_radius(&self, _size: ResolvedSize, _scale: f32) -> Option<[f32; 4]> { Some([9.0; 4]) }
    fn background_color(&self) -> Option<Color> { Some(Color::BLUE) }
    fn is_empty(&self) -> bool { self.payload.first() == Some(&1) }
    fn revision(&self) -> u64 { self.revision.get() }
}

impl<const N: usize> Drop for Probe<N> {
    fn drop(&mut self) { self.drops.set(self.drops.get() + 1); }
}

fn check_erasure<const N: usize>(inline: bool) {
    let drops = Rc::new(Cell::new(0));
    let revision = Rc::new(Cell::new(12));
    let calls = Rc::new(Cell::new(0));
    let erased: AnyDecor = Probe {
        drops: drops.clone(), revision: revision.clone(), calls: calls.clone(), payload: [1; N],
    }.notype();
    assert_eq!(erased.is_inline(), inline);
    assert!(erased.is_direct());
    assert_eq!(drops.get(), 0);
    let size = ResolvedSize { width: 80.0, height: 40.0 };
    let origin = Vec2d { x: 3.0, y: 5.0 };
    // A generic trait bound exercises AnyDecor's forwarding impl, rather than Deref alone.
    fn check_geometry<D: Decoration>(decoration: &D, size: ResolvedSize, empty: bool) {
        assert_eq!(decoration.paint_outsets(size, 2.0), [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(decoration.content_insets(size, 2.0), [5.0, 6.0, 7.0, 8.0]);
        assert_eq!(decoration.child_clip_radius(size, 2.0), Some([9.0; 4]));
        assert_eq!(decoration.background_color(), Some(Color::BLUE));
        assert_eq!(decoration.is_empty(), empty);
        assert_eq!(decoration.revision(), 12);
    }
    check_geometry(&erased, size, N != 0);
    revision.set(17);
    assert_eq!(Decoration::revision(&erased), 17);

    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 200.0, 160.0)).unwrap();
    let canvas = Canvas::of(&tree.context(node).unwrap());
    Decoration::paint(&erased, &canvas, origin, size, 2.0);
    Decoration::paint_layer(&erased, &canvas, origin, size, 2.0);
    Decoration::paint_with_background(&erased, &canvas, origin, size, 2.0, Some(Color::RED));
    canvas.finish();
    assert_eq!(calls.get(), 7);
    let commands = tree.draw_list_snapshot(node).unwrap();
    assert!(matches!(commands.commands.as_ref(), [
        DrawCommand::FillRect { color: blue, rect, .. },
        DrawCommand::FillRect { color: green, .. },
        DrawCommand::FillRect { color: red, .. },
    ] if blue.b == 255 && green.g > 0 && red.r == 255
        && *rect == Rect::new(3.0, 5.0, 80.0, 40.0)));

    let original_pointer = erased.as_ref() as *const dyn Decoration as *const ();
    let erased_again = erased.notype();
    assert_eq!(erased_again.is_inline(), inline, "re-erasure must not wrap another owner");
    if !inline {
        assert_eq!(erased_again.as_ref() as *const dyn Decoration as *const (), original_pointer);
    }
    drop(erased_again);
    assert_eq!(drops.get(), 1);
}

#[test]
fn notype_forwards_all_decoration_methods_for_inline_and_heap_values() {
    check_erasure::<0>(true);
    const BOUNDARY: usize = AnyDecor::INLINE_CAPACITY - 3 * std::mem::size_of::<Rc<Cell<usize>>>();
    check_erasure::<BOUNDARY>(true);
    check_erasure::<{ BOUNDARY + 1 }>(false);
    check_erasure::<128>(false);
}

#[test]
fn notype_preserves_overaligned_payloads() {
    #[repr(align(128))]
    struct Aligned(Cell<u64>);
    impl Decoration for Aligned {
        fn paint(&self, _canvas: &Canvas, _origin: Vec2d, _size: ResolvedSize, _scale: f32) {}
        fn revision(&self) -> u64 { self.0.get() }
    }
    let erased = Aligned(Cell::new(12)).notype();
    assert!(erased.is_heap());
    assert_eq!((erased.as_ref() as *const dyn Decoration as *const () as usize) % 128, 0);
    assert_eq!(erased.revision(), 12);
}

#[test]
fn notype_uses_the_active_ui_allocator_and_keeps_its_heap_payload_alive() {
    let memory = UiMemory::new(2 * 1024 * 1024);
    let allocator = memory.allocator();
    let drops = Rc::new(Cell::new(0));
    let decoration = allocator.scope(|| Probe {
        drops: drops.clone(), revision: Rc::new(Cell::new(12)),
        calls: Rc::new(Cell::new(0)), payload: [1; 128],
    }.notype());
    assert!(decoration.is_heap());
    assert!(allocator.committed_bytes() > 0);
    drop(memory);
    drop(allocator);
    assert_eq!(decoration.revision(), 12);
    drop(decoration);
    assert_eq!(drops.get(), 1);
}

#[test]
fn notype_supports_mixed_decoration_collections_and_widgets() {
    let layers: Vec<AnyDecor> = vec![
        BorderRadius::from(8).notype(), BoxShadow::new().blur(3.0).notype(),
    ];
    let widget = MultiDecorationBox::new().decorations(layers)
        .child(aimer_container::SizedBox::new().width(80).height(40));
    fn accepts_widget(_widget: impl aimer_widget::Widget) {}
    accepts_widget(widget);
}
