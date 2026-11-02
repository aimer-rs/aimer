# OpenGL shaders

These are Cupid's hand-maintained GLSL 3.30 Core shaders for desktop OpenGL.
Android's GLES 3 path and direct WebGL2 adapt the version line and add explicit
fragment precision before compiling them. Edit the vertex and fragment GLSL in
each `.glslpack` directly; no WGSL translation step is used for OpenGL.

The direct WebGL2 backend reuses these packages and converts the version line
to GLSL ES 3.00 with explicit fragment precision before compiling them.

Each package has `[VERTEX]`, `[FRAGMENT]`, and `[RESOURCES]` sections. Resource
rows keep the shader's UBO and texture bindings connected to Cupid's bind-group
layout. The OpenGL headless renderer test creates and links all built-in
pipelines and checks the rendered output:

```text
cargo test -p aimer_cupid --no-default-features --features opengl generic_renderer_renders_and_reads_back_with_headless_opengl
```
