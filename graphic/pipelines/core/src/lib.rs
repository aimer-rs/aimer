#![doc = include_str!("../README.md")]

mod anti_alias;
pub mod custom_pipeline;
pub mod frame_upload;
mod instance_policy;

pub use anti_alias::{AntiAlias, multisample_state};
pub use custom_pipeline::{
    CommandContext, CustomPipelineGeneric, CustomPipelineSlotGeneric, RenderContextGeneric,
};
pub use frame_upload::FrameUpload;
pub use instance_policy::InstanceBufferPolicy;
