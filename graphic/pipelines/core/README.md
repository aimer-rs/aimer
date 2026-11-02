# aimer_pipeline_core

Pieces every Cupid render pipeline shares: the [`AntiAlias`] strategy, the
per-frame [`FrameUpload`] gate that skips redundant instance writes, the
[`InstanceBufferPolicy`] that sizes instance buffers, and the
[`CustomPipelineGeneric`] trait the renderer drives.

It depends only on the `aimer_cupid_gpu` backend contract, so the text, image,
rect, SVG and material pipelines can be built independently of one another.
