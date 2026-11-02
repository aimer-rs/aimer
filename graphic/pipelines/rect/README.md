# aimer_rect_pipeline

The instanced pipeline that draws Cupid's rectangles: fills, per-corner radii,
borders, outlines, shadows and rounded clips, all evaluated analytically in the
fragment shader.

[`RectPipeline`] is generic over the `aimer_cupid_gpu` backend contract, so one
implementation serves Metal, Vulkan, OpenGL, Direct3D 12, WebGL2 and WebGPU.
