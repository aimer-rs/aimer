//! HLSL emitter (Direct3D 12, shader model 6).

use super::clike::{self, Dialect};
use crate::error::Result;
use crate::resolve::Program;

pub fn emit(p: &Program) -> Result<String> {
    clike::emit(p, Dialect::Hlsl)
}
