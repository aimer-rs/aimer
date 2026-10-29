//! Portable reflection and retained-state support for generated Aimer code.
//!
//! The reflection vocabulary lives in `aimer_portable`; this module adds the
//! parts that need widgets and re-exports the rest, so the whole of it is still
//! reachable as `portable`.

#[cfg(feature = "portable-guest")]
mod encoder;
#[doc(hidden)]
pub mod materializer;
#[cfg(feature = "portable-guest")]
mod semantic_graph;
#[cfg(feature = "portable-guest")]
pub(crate) mod state;
#[cfg(feature = "portable-guest")]
mod widget_ir;

pub use aimer_portable::*;
#[cfg(feature = "portable-guest")]
pub use encoder::*;
pub use materializer::*;
pub use crate::widget::PortableWidget;
#[cfg(feature = "portable-guest")]
pub use semantic_graph::*;
#[cfg(feature = "portable-guest")]
pub use widget_ir::*;
