#![doc = include_str!("../README.md")]

mod contract;
mod effect;
mod header;
mod params;
mod pipeline;
mod registry;
mod source;
mod stdlib;

pub use contract::{EffectProgram, analyze};
pub use effect::{EffectCommand, ShaderEffect, decode_payload};
pub use header::HEADER_SIZE;
pub use params::ParamWriter;
pub use pipeline::EffectPipeline;
pub use registry::{EffectSpec, generation, register, registered, samples_backdrop};
