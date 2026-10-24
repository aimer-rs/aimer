//! Portable reflection and retained-state support for generated Aimer code.

mod codec;
#[cfg(feature = "portable-guest")]
mod encoder;
mod identity;
#[doc(hidden)]
pub mod materializer;
mod registry;
mod schema;
#[cfg(feature = "portable-guest")]
mod semantic_graph;
#[cfg(feature = "portable-guest")]
pub(crate) mod state;
#[cfg(feature = "portable-guest")]
mod widget_ir;

pub use codec::*;
#[cfg(feature = "portable-guest")]
pub use encoder::*;
pub use identity::*;
pub use materializer::*;
pub use registry::*;
pub use schema::*;
pub use crate::widget::PortableWidget;
#[cfg(feature = "portable-guest")]
pub use semantic_graph::*;
#[doc(hidden)]
pub use aimer_anteros as __anteros;
#[cfg(feature = "portable-guest")]
pub use widget_ir::*;

#[doc(hidden)]
#[cfg(any(
    target_os = "linux",
    target_os = "macos",
    target_os = "windows",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "illumos",
))]
pub use linkme as __linkme;
