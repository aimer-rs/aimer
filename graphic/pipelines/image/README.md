# aimer_image_pipeline

The instanced pipeline that draws Cupid's decoded images and renderer-owned retained
layers: texture upload, size constraining, rounded clipping and opacity.

[`ImagePipeline`] is generic over the `aimer_cupid_gpu` backend contract.
