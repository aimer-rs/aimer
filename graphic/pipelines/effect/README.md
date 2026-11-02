# aimer_effect_pipeline

Lets an application add its own shader effect to the Cupid renderer: a water
surface, a heat haze, a custom transition. The effect is one `.aimer` shader
plus a small Rust type that supplies its parameters. The pipeline translates the
shader for whichever backend is rendering (WGSL, MSL, HLSL, GLSL, and WGSL to
SPIR-V on Vulkan), so the same source runs everywhere.

1. Write the shader and a [`ShaderEffect`] for it. The trait documents the
   contract the shader must follow, with a complete example;
   `tests/shaders/water.aimer` (refracts the backdrop) and
   `tests/shaders/solid.aimer` (flat color) are working references.
2. [`register`] it once, before or after the window opens. A shader that breaks
   the contract is rejected here with a message naming the problem.
3. Draw it with [`EffectCommand::new`] and send `name` and `payload` through
   `draw_custom`. The rectangle is in the recording element's local space; the
   renderer applies the transform, clip and opacity in force at that point.

Through the umbrella crate this is `aimer::cupid::effects`.

## Limits

* An effect draws one axis-aligned rectangle. A rotated or skewed transform is
  approximated by the box through the rectangle's opposite corners.
* A backdrop effect reads the frame as drawn so far, copied for the rectangle
  only; sampling outside it returns stale texels.
* Backends: wgpu, WebGPU, Metal, Vulkan (through WGSL, compiled by `naga`),
  Direct3D 12 and OpenGL/GLES. WebGL takes no run-time shaders yet.
