//! Metal Shading Language emitter.

use super::clike::{self, Dialect};
use crate::error::Result;
use crate::resolve::Program;

pub fn emit(p: &Program) -> Result<String> {
    clike::emit(p, Dialect::Msl)
}
