# aimer_svg_pipeline

The pipeline that draws tessellated SVG scenes for Cupid: mesh upload and caching,
gradient and pattern resources, clip paths, masks and filter primitives, with a
one-pixel analytic coverage fringe for edges.

[`SvgPipeline`] is generic over the `aimer_cupid_gpu` backend contract and consumes
[`SvgRenderItem`] values that the renderer builds from the draw list.
