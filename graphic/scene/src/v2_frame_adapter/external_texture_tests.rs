//! An external texture survives lowering with its producer and generation.

use std::sync::Arc;

use crate::draw_cmd::{DrawCommand, Rect, RenderOp, RenderPaintSource, RenderTree};
use crate::external_texture::{ExternalTexture, ExternalTextureSource, NativeFrame};
use crate::frame::RetainedV2Item;
use crate::render_list::DrawCommand as LegacyDrawCommand;

struct NoFrames;

impl ExternalTextureSource for NoFrames {
    fn current_frame(&self) -> Option<NativeFrame> {
        None
    }
}

#[test]
fn the_command_is_lowered_with_its_rect_source_and_generation() {
    let source: Arc<dyn ExternalTextureSource> = Arc::new(NoFrames);
    let tree = RenderTree::new();
    let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 36.0)).unwrap();
    tree.set_paint_source(root, RenderPaintSource::LocalV2).unwrap();
    tree.context(root)
        .unwrap()
        .begin_recording()
        .unwrap()
        .commit(vec![DrawCommand::DrawExternalTexture {
            rect: Rect::new(2.0, 3.0, 60.0, 30.0),
            texture: ExternalTexture::new(source.clone(), 9),
        }])
        .unwrap();
    let item = tree
        .render_all()
        .into_iter()
        .find_map(|operation| match operation {
            RenderOp::Draw(item) => Some(item),
            _ => None,
        })
        .expect("the root is drawn");
    let snapshot = item.snapshot();
    let retained = RetainedV2Item {
        element: item.element,
        revision: snapshot.revision,
        bounds: item.bounds,
        origin: item.origin,
        transform: item.transform,
        clip: item.clip,
        clip_radius: item.clip_radius,
        commands: snapshot.commands,
    };

    let lowered = super::lower_retained_v2_commands(&retained, 2.0).unwrap();
    let (rect, texture) = lowered
        .iter()
        .find_map(|command| match command {
            LegacyDrawCommand::DrawExternalTexture { rect, texture } => Some((*rect, texture.clone())),
            _ => None,
        })
        .expect("the lowered stream carries the external texture");
    // Element-local, like an image: the node transform places it.
    assert_eq!(rect, Rect::new(2.0, 3.0, 60.0, 30.0));
    assert!(Arc::ptr_eq(texture.source(), &source));
    assert_eq!(texture.generation(), 9);
}
