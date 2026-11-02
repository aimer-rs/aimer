#![doc = include_str!("../README.md")]

// Aliases keep the scene code reading like the rest of Cupid.
pub(crate) use aimer_cupid_font::font;
pub(crate) use aimer_cupid_svg as svg;
pub(crate) use aimer_primitives as utilities;
pub(crate) use aimer_text_pipeline as text_pipeline;

pub mod compositor;
#[doc(hidden)]
pub mod damage_region;
pub mod draw_cmd;
pub mod draw_cmd_v2;
pub mod frame;
#[doc(hidden)]
pub mod v2_frame_adapter;
