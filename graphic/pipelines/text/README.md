# aimer_text_pipeline

The pipeline that turns text draw requests into glyph quads for Cupid: layout and
shaping caches, the alpha and color glyph atlases, deferred and batched preparation,
decorations and shadows.

Font parsing, shaping and rasterization live in `aimer_cupid_font`; this crate owns the
GPU-facing side. [`TextPipelineV2`] is generic over the `aimer_cupid_gpu` backend
contract, and [`TextDrawRequest`] is the request type the scene model records.
