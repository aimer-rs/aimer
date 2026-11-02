pub(crate) mod frame_composite;
pub use aimer_image_pipeline as image_pipeline;
pub use aimer_material_pipeline as material;
pub(crate) mod morph_pipeline;
pub use aimer_rect_pipeline as rect_pipeline;
pub use aimer_svg_pipeline as svg_pipeline;
pub use aimer_text_pipeline as text_pipeline;

pub use aimer_pipeline_core::AntiAlias;
// Shared pipeline helpers live in `aimer_pipeline_core`; these aliases keep the
// in-crate paths short until each pipeline moves to its own crate.
pub(crate) use aimer_pipeline_core::{frame_upload, multisample_state};
