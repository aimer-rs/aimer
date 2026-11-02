//! GLSL 3.30 core and GLSL ES 3.00 emitters. One program gives a vertex and a fragment shader.

use super::clike::{self, GlslOutput};
use crate::error::Result;
use crate::resolve::Program;

/// Desktop OpenGL: `#version 330 core`.
pub fn emit(p: &Program) -> Result<GlslOutput> {
    clike::emit_glsl(p, false)
}

/// OpenGL ES 3.0 and WebGL2: `#version 300 es`.
pub fn emit_es(p: &Program) -> Result<GlslOutput> {
    clike::emit_glsl(p, true)
}
