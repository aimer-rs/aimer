use crate::Decoration;
use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::draw_cmd_v2::{DrawCommand, Rect, RenderTree};
use aimer_cupid::material::MaterialRequest;

/// Box size used by [`stacked`].
pub(super) const BOX: ResolvedSize = ResolvedSize { width: 80.0, height: 40.0 };

/// Paints `layers` in order through one canvas at scale 2, as a decoration
/// stack does, and decodes the one material packet they record.
pub(super) fn stacked(layers: &[&dyn Decoration]) -> MaterialRequest {
    stacked_in(BOX, layers)
}

pub(super) fn stacked_in(size: ResolvedSize, layers: &[&dyn Decoration]) -> MaterialRequest {
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 100.0, 80.0)).unwrap();
    let canvas = Canvas::of(&tree.context(root).unwrap());
    for layer in layers {
        layer.paint_layer(&canvas, Vec2d { x: 3.0, y: 4.0 }, size, 2.0);
    }
    canvas.finish();
    let commands = tree.draw_list_snapshot(root).unwrap().commands.to_vec();
    let mut packets = commands.iter().filter_map(|command| match command {
        DrawCommand::DrawCustom { data, .. } => Some(data),
        _ => None,
    });
    let data = packets.next().expect("one material packet");
    assert!(packets.next().is_none(), "one material packet");
    MaterialRequest::decode(data).unwrap()
}
