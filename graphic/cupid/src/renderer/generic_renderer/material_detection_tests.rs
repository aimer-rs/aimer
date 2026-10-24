use super::*;
use crate::pipeline::material::{MaterialKind, MaterialRequest};

#[test]
fn light_edge_only_draw_lists_do_not_need_an_offscreen_backdrop() {
    let edge = MaterialRequest::new(MaterialKind::LightEdge, [0.0, 0.0, 80.0, 40.0]);
    let mut encoded = DrawList::new();
    encoded.draw_custom(MATERIAL_PIPELINE_NAME, edge.encode_for_test());
    assert!(!draw_list_uses_material(&encoded));
    let mut typed = DrawList::new();
    typed.draw_custom(MATERIAL_PIPELINE_NAME, edge);
    assert!(!draw_list_uses_material(&typed));
    encoded.draw_custom(
        MATERIAL_PIPELINE_NAME,
        MaterialRequest::new(MaterialKind::Glass, edge.bounds).encode_for_test(),
    );
    assert!(
        draw_list_uses_material(&encoded),
        "mixed frames still need capture"
    );
}

#[test]
fn retained_light_edges_do_not_need_an_offscreen_backdrop() {
    use crate::damage_region::DamageSet;
    use crate::draw_cmd_v2::{DrawCommand, Rect, RenderFrame, RenderPaintSource, RenderTree};
    use crate::frame::{FramePacket, FrameRenderMetadata};
    use std::sync::Arc;
    for kind in [MaterialKind::LightEdge, MaterialKind::Magnification] {
        let tree = RenderTree::new();
        let root = tree.add_root(Rect::new(0.0, 0.0, 64.0, 64.0)).unwrap();
        tree.set_paint_source(root, RenderPaintSource::LocalV2)
            .unwrap();
        tree.context(root)
            .unwrap()
            .begin_recording()
            .unwrap()
            .commit(vec![DrawCommand::DrawCustom {
                pipeline_name: Arc::from(MATERIAL_PIPELINE_NAME),
                data: Arc::from(
                    MaterialRequest::new(kind, [0.0, 0.0, 64.0, 64.0]).encode_for_test(),
                ),
            }])
            .unwrap();
        let packet = FramePacket::from_v2_direct(
            RenderFrame {
                damage: vec![Rect::new(0.0, 0.0, 64.0, 64.0)],
                operations: tree.render_all(),
            },
            FrameRenderMetadata::new(1.0, 1, 1, 1, 1, DamageSet::new(64, 64)),
        )
        .unwrap();
        let plan = packet.render_plan().unwrap();
        assert!(render_plan_uses_custom_pipeline(plan, 64, 64));
        assert_eq!(
            render_plan_uses_material(plan, 64, 64),
            kind != MaterialKind::LightEdge
        );
    }
}
