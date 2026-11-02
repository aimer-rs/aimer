# aimer_material_pipeline

The custom pipeline behind Cupid's materials: backdrop blur, light-edge glass,
magnification and gradients, with downsampled blur passes and a bounded intermediate
texture budget.

[`MaterialRequest`] is the packet a draw command carries; [`MaterialPipeline`] implements
`CustomPipelineGeneric` from `aimer_pipeline_core` so the renderer can drive it like any
other custom pipeline.
