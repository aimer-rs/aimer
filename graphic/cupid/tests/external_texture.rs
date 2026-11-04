//! A live GPU producer in the retained render tree.

use std::sync::Arc;

use aimer_cupid::draw_cmd::{DrawCommand, Rect, RenderTree};
use aimer_cupid::external_texture::{ExternalTexture, ExternalTextureSource, NativeFrame};

struct NoFrames;

impl ExternalTextureSource for NoFrames {
    fn current_frame(&self) -> Option<NativeFrame> {
        None
    }
}

fn record(tree: &RenderTree, node: aimer_cupid::draw_cmd::RenderNodeId, texture: ExternalTexture) {
    let writer = tree.context(node).unwrap().begin_recording().unwrap();
    writer
        .commit(vec![DrawCommand::DrawExternalTexture {
            rect: Rect::new(0.0, 0.0, 64.0, 36.0),
            texture,
        }])
        .unwrap();
}

#[test]
fn a_new_frame_damages_the_node_and_an_unchanged_one_does_not() {
    let source: Arc<dyn ExternalTextureSource> = Arc::new(NoFrames);
    let tree = RenderTree::new();
    let node = tree.add_root(Rect::new(0.0, 0.0, 64.0, 36.0)).unwrap();
    record(&tree, node, ExternalTexture::new(source.clone(), 1));
    let _ = tree.take_damage();

    // The element repainted, but the video has no new frame: nothing to draw.
    tree.refresh_paint(node).unwrap();
    record(&tree, node, ExternalTexture::new(source.clone(), 1));
    assert!(tree.take_damage().is_empty(), "the same frame was damaged again");

    // The next frame arrived.
    tree.refresh_paint(node).unwrap();
    record(&tree, node, ExternalTexture::new(source, 2));
    assert!(!tree.take_damage().is_empty(), "a new frame left the node undamaged");
}
