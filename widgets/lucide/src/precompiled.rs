use std::collections::HashMap;
use std::sync::Arc;

use aimer_cupid::svg::*;
use aimer_svg::SvgDocument;

pub(super) fn document(
    source: &'static str,
    nodes: Arc<[SvgNode]>,
    geometries: Arc<[SvgGeometry]>,
    viewport: SvgViewport,
    view_box: SvgViewBox,
    preserve_aspect_ratio: SvgPreserveAspectRatio,
    has_view_box: bool,
    root_transform: SvgTransform,
) -> SvgDocument {
    let resources = Arc::new(SvgResourceGraph { root_transform, ..SvgResourceGraph::empty() });
    let node_paints: HashMap<_, _> = nodes.iter().map(|node| (node.node_id, SvgParsedPaints {
        fill: node.fill_paint.clone(), stroke: node.stroke_paint.clone(),
    })).collect();
    SvgDocument::__from_precompiled(source, SvgParsedDocument {
        scene: SvgScene { viewport, nodes, geometries, resources: resources.clone() },
        view_box, preserve_aspect_ratio, has_view_box, root_transform,
        gradients: Arc::from([]), resources, node_paints,
        unsupported_elements: Arc::from([]),
    })
}
