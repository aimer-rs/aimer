---
sessionId: session-260921-184321-4xw1
---

# Requirements

### Overview & Goals

Make `aimer_cupid`'s GPU backend **pluggable** behind an experimental feature flag, with wgpu as the default backend and a native Metal backend as the first alternative.

**Goals:**
- Define a `GpuBackend` trait that abstracts all GPU operations (device, queue, resources, render pass encoding, submission)
- Provide a `WgpuBackend` adapter that delegates to wgpu — the default, zero-cost when selected
- Provide a `MetalBackend` adapter using `objc2-metal` with pre-compiled MSL shaders
- Keep the existing wgpu direct path unchanged when neither flag is enabled
- Maintain backward compatibility via type aliases

### Scope

**In Scope:**
- `backend/` module with `GpuBackend` trait and associated type hierarchy (behind `pluggable-backend-exp`)
- `WgpuBackend` adapter implementing the trait for all wgpu operations (behind `pluggable-backend-exp`)
- `MetalBackend` adapter implementing the trait via `objc2-metal` (behind `metal` feature)
- Generic `RendererImpl<B: GpuBackend>` alongside the existing `Renderer` compatibility API
- All 6 pipelines generic over `B`: `RectPipeline`, `ImagePipeline`, `TextPipelineV2`, `SvgPipeline`, `FrameCompositePipeline`, `MaterialPipeline`
- Backend-generic `CustomPipelineGeneric<B>` alongside the existing `CustomPipeline` API
- `PersistentTarget`, `FrameUpload` generic over `B`
- `RenderContext` generic over `B`
- MSL shaders for rect pipeline as Metal proof-of-concept
- `GpuContext` keeps its wgpu surface management and exposes a `WgpuBackend` adapter

**Out of Scope:**
- Removing WGPU support; WGPU remains the default backend, and Metal-only builds
  can disable its dependency with `--no-default-features --features metal`
- WASM/WebGPU Metal backend
- Vulkan, DirectX, or OpenGL backends
- Runtime backend switching (compile-time selection only)
- Runtime translation of arbitrary custom shaders; custom pipelines must provide source in the selected backend's language
- Performance optimization of the Metal path

### Non-Functional Requirements

- The non-flag path must produce identical code (zero overhead when `pluggable-backend-exp` is off)
- The generic path must not regress wgpu renderer performance when the default `WgpuBackend` is used
- The `metal` feature must only compile on `target_os = "macos"` or `target_os = "ios"`

# Technical Design

### Current Implementation

The original compatibility renderer remains tied to wgpu across **15+ source files** (~12,330 lines), while the feature-gated `RendererImpl<B>` now drives all six built-in pipelines through `GpuBackend`. `WgpuBackend` preserves the generic WGPU path; `MetalBackend` supports the built-in pipeline set through checked-in MSL. The generic renderer has not replaced the compatibility renderer's scene/damage processing, compositor statistics, pipeline cache, and multisampling APIs.

The WGPU-specific compatibility path still directly uses:

- `gpu_context.rs` (384 lines) — wraps `wgpu::Device`, `wgpu::Queue`, `wgpu::Surface`
- `renderer.rs` (2,828 lines) — orchestration, takes `&wgpu::Device`, `&wgpu::Queue`, `&wgpu::TextureView`
- `custom_pipeline.rs` (174 lines) — `trait CustomPipeline { fn render(&self, pass: &mut wgpu::RenderPass); }`
- 6 pipeline files (~4,200 lines) — each owns wgpu resources and calls wgpu directly
- `persistent_target.rs` (323 lines) — `wgpu::Texture`, `wgpu::TextureView`
- `frame_upload.rs` — `wgpu::Buffer` upload dedup
- 10 WGSL shader files consumed by wgpu pipeline creation

Consumers on the compatibility path (`aimer_quiver/src/render_ctx/wgpu_ctx.rs`, `bin.rs`, and existing `Renderer` users) continue to pass wgpu types directly.

### Key Decisions

**Decision 1: Full device abstraction (user-approved)**
The `GpuBackend` trait abstracts **every GPU operation** — resource creation, data upload, command encoding, render pass ops, and submission. This gives the cleanest seam and maximum leverage for future backends.

**Decision 2: Add a generic rendering path behind the feature flag**
The six GPU pipelines and `RendererImpl<B>` use `GpuBackend` when `pluggable-backend-exp` is enabled. The existing `Renderer` remains available while its scene, damage, compositor, and pipeline-cache APIs are migrated; this avoids replacing its public API before the generic implementation covers those capabilities.

**Decision 3: Generic (compile-time) dispatch via type parameter**
`RendererImpl<B: GpuBackend = WgpuBackend>` with a default type parameter. No `Box<dyn>` overhead — the concrete backend is known at compile time.

**Decision 4: Parallel wgpu-only path when flag is off**
When `pluggable-backend-exp` is disabled, the default feature set compiles to the current WGPU-only path (unchanged). With the experimental feature enabled, callers can select WGPU or Metal at compile time.

**Decision 5: Separate surface/context creation per backend**
`GpuContext` is split: a wgpu-specific path (current code, used when flag is off) and a backend-generic path (when flag is on) that works with `WgpuBackend` or `MetalBackend`.

### Proposed Changes

#### Module Structure

```
aimer_cupid/src/
├── backend/
│   ├── mod.rs              # GpuBackend trait + associated type traits
│   ├── wgpu.rs             # WgpuBackend impl (behind pluggable-backend-exp)
│   └── metal.rs            # MetalBackend impl (behind cfg(feature = "metal"))
├── renderer.rs              # Existing WGPU renderer and feature-gated generic export
├── renderer/
│   └── generic_renderer.rs  # RendererImpl<B: GpuBackend = WgpuBackend>
├── pipeline/
│   ├── rect_pipeline.rs     # RectPipeline<B: GpuBackend>
│   ├── image_pipeline.rs    # ImagePipeline<B: GpuBackend>
│   ├── svg_pipeline.rs      # SvgPipeline<B: GpuBackend>
│   ├── text_pipeline.rs     # TextPipelineV2<B: GpuBackend>
│   ├── frame_composite.rs   # FrameCompositePipeline<B: GpuBackend>
│   ├── material/
│   │   └── render.rs        # MaterialPipeline<B: GpuBackend>
│   ├── frame_upload.rs      # FrameUpload (already mostly backend-agnostic data)
│   └── ...
├── custom_pipeline.rs       # CustomPipeline<B: GpuBackend>
├── persistent_target.rs     # PersistentTarget<B: GpuBackend>
├── gpu_context.rs           # Current wgpu-specific (unchanged when flag off)
└── lib.rs
```

#### Trait Architecture

```
GpuBackend (main entry point — device + queue)
  │
  ├── associated types: Buffer, Texture, TextureView, BindGroup,
  │                     BindGroupLayout, PipelineLayout, RenderPipeline,
  │                     Sampler, ShaderModule, CommandEncoder, RenderPass,
  │                     TextureFormat, SurfaceStatus
  │
  ├── resource creation: create_buffer, create_texture, create_sampler,
  │                       create_shader_module, create_bind_group_layout,
  │                       create_pipeline_layout, create_render_pipeline,
  │                       create_bind_group
  │
  ├── data upload: write_buffer, write_texture
  │
  ├── command encoding: create_command_encoder
  │
  ├── render pass: begin_render_pass (returns RenderPass)
  │
  └── submission: submit, (surface management in separate trait)

GpuRenderPass<B: GpuBackend>
  ├── set_pipeline(&mut self, &B::RenderPipeline)
  ├── set_bind_group(&mut self, u32, &B::BindGroup, &[u32])
  ├── set_vertex_buffer(&mut self, u32, &B::Buffer, u64)
  ├── set_index_buffer(&mut self, &B::Buffer, ...)
  ├── set_scissor_rect(&mut self, u32, u32, u32, u32)
  └── draw(&mut self, Range<u32>, Range<u32>)
```

#### Core Trait Signatures

```rust
// backend/mod.rs
pub trait GpuBackend: Sized + 'static {
    type Buffer: GpuBufferOps;
    type Texture: GpuTextureOps;
    type TextureView: GpuTextureViewOps;
    type BindGroupLayout;
    type BindGroup;
    type PipelineLayout;
    type RenderPipeline;
    type Sampler;
    type ShaderModule;
    type TextureFormat: Copy + Eq + Send + Sync;

    fn create_shader_module(&self, source: &[u8], label: &str) -> Self::ShaderModule;
    fn create_buffer(&self, size: u64, usage: u32, label: &str) -> Self::Buffer;
    fn write_buffer(&self, buffer: &Self::Buffer, offset: u64, data: &[u8]);
    fn create_bind_group_layout(&self, entries: &[BindGroupLayoutEntry]) -> Self::BindGroupLayout;
    fn create_pipeline_layout(&self, layouts: &[&Self::BindGroupLayout]) -> Self::PipelineLayout;
    fn create_render_pipeline(&self, desc: &RenderPipelineDescriptor<Self>) -> Self::RenderPipeline;
    fn create_bind_group(&self, layout: &Self::BindGroupLayout, entries: &[BindGroupEntry<Self>]) -> Self::BindGroup;
    fn create_command_encoder(&self, label: &str) -> Self::CommandEncoder;
    fn begin_render_pass(&self, encoder: &mut Self::CommandEncoder, desc: &RenderPassDescriptor<Self>) -> Self::RenderPass<'_>;
    fn submit(&self, encoder: Self::CommandEncoder);
    fn limits(&self) -> GpuLimits;
}
```

#### Renderer Compatibility Boundary

With `pluggable-backend-exp`, `RendererImpl<B = WgpuBackend>` is exported as the new compile-time generic renderer. The existing `Renderer` remains the WGPU compatibility implementation, including its scene/damage processing, compositor statistics, pipeline cache, and multisampling APIs. Once those capabilities move to `RendererImpl`, `Renderer` can converge on a backward-compatible alias without dropping functionality.

#### Feature Flag Wiring

```toml

# aimer_cupid/Cargo.toml

[features]
default = ["apple-core-text", "wgpu"]
wgpu = ["dep:wgpu", "dep:winit"]
pluggable-backend-exp = []
metal = [
    "pluggable-backend-exp",
    "apple-core-text",
    "dep:objc2-metal",
    "dep:objc2-foundation",
    "dep:objc2",
    "dep:objc2-quartz-core",
]

[dependencies]
wgpu = { workspace = true, optional = true }   # ← becomes optional

[target.'cfg(any(target_os = "macos", target_os = "ios"))'.dependencies]
objc2-metal = { ... optional = true }  # Added dep
```

#### WgpuBackend Adapter Pattern

```rust
// backend/wgpu.rs
pub struct WgpuBackend {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl GpuBackend for WgpuBackend {
    type Buffer = wgpu::Buffer;
    type Texture = wgpu::Texture;
    // ... all types map directly to wgpu types

    fn create_buffer(&self, size: u64, usage: u32, label: &str) -> wgpu::Buffer {
        // Map our flags to wgpu::BufferUsages
        self.device.create_buffer(&wgpu::BufferDescriptor {
            size,
            usage: wgpu::BufferUsages::from_bits_truncate(usage),
            label: Some(label),
            mapped_at_creation: false,
        })
    }
    // ...
}
```

#### MetalBackend (initial scope: rect pipeline only)

```rust
// backend/metal.rs — behind #[cfg(feature = "metal")]
pub struct MetalBackend {
    device: metal::Device,
    queue: metal::CommandQueue,
    format: MTLPixelFormat,
}

impl GpuBackend for MetalBackend {
    type Buffer = metal::Buffer;
    type Texture = metal::Texture;
    // ...
    
    fn create_shader_module(&self, source: &[u8], label: &str) -> metal::Library {
        // Compile MSL source at runtime via Metal's runtime compilation
        self.device.new_library_with_source(
            std::str::from_utf8(source).unwrap(),
            &metal::CompileOptions::new(),
        ).unwrap()
    }
    // ...
}
```

### Data Models / Contracts

**Shader source abstraction:**
- For `WgpuBackend`: WGSL source bytes → `wgpu::ShaderModule` via naga
- For `MetalBackend`: MSL source bytes → `metal::Library` via Metal runtime compilation
- Each pipeline embeds its shader source with `include_str!` as before; the backend interprets the bytes

**Bind group model:**
`BindGroupEntry` and `BindGroupLayoutEntry` are defined as simple enums/structs in `backend/mod.rs` that each backend converts to its native representation:

```rust
pub enum BindingResource<'a, B: GpuBackend> {
    Buffer(&'a B::Buffer, u64, u64),     // buffer, offset, size
    TextureView(&'a B::TextureView),
    Sampler(&'a B::Sampler),
}
```

### Components

| Component | Before | After | Lines Affected |
|-----------|--------|-------|----------------|
| **backend/mod.rs** | (new) | `GpuBackend` trait + associated types | +~200 |
| **backend/wgpu.rs** | (new) | `WgpuBackend` adapter | +~300 |
| **backend/metal.rs** | (new) | `MetalBackend` adapter | +~500 (POC) |
| **renderer.rs** | wgpu types directly | `RendererImpl<B: GpuBackend>` | ~2,828 |
| **rect_pipeline.rs** | wgpu types directly | `RectPipeline<B: GpuBackend>` | ~383 |
| **image_pipeline.rs** | wgpu types directly | `ImagePipeline<B: GpuBackend>` | ~1,091 |
| **text_pipeline.rs** | wgpu types directly | `TextPipelineV2<B: GpuBackend>` | ~3,122 |
| **svg_pipeline.rs** | wgpu types directly | `SvgPipeline<B: GpuBackend>` | ~643 |
| **frame_composite.rs** | wgpu types directly | `FrameCompositePipeline<B: GpuBackend>` | ~111 |
| **material/render.rs** | wgpu types directly | `MaterialPipeline<B: GpuBackend>` | ~682 |
| **custom_pipeline.rs** | `&mut wgpu::RenderPass` | `&mut B::RenderPass<'_>` | ~174 |
| **persistent_target.rs** | `wgpu::Texture` | `B::Texture` + `B::TextureView` | ~323 |
| **frame_upload.rs** | `wgpu::Buffer` | `B::Buffer` | ~254 |
| **gpu_context.rs** | wgpu-specific | Split: wgpu-path + generic surface | ~385 |
| **pipeline.rs** | `wgpu::MultisampleState` | Backend-agnostic | ~70 |
| **lib.rs** | wgpu re-exports | Conditional re-exports | ~1,148 |
| **Cargo.toml** | wgpu required | wgpu optional | ~104 |

### Architecture Diagram

```mermaid
graph TD
    subgraph Consumer
        W[aimer_quiver / bin.rs]
    end

    subgraph cupid [aimer_cupid]
        R["RendererImpl&lt;B&gt;"]
        CP["CustomPipeline&lt;B&gt;"]
        PT["PersistentTarget&lt;B&gt;"]
        
        subgraph Pipelines
            RP["RectPipeline&lt;B&gt;"]
            IP["ImagePipeline&lt;B&gt;"]
            TP["TextPipelineV2&lt;B&gt;"]
            SP["SvgPipeline&lt;B&gt;"]
            MP["MaterialPipeline&lt;B&gt;"]
        end

        subgraph backend [Backend Module]
            T["GpuBackend trait"]
            WB["WgpuBackend"]
            MB["MetalBackend"]
        end
        
        R --> RP
        R --> IP
        R --> TP
        R --> SP
        R --> MP
        R --> CP
        R --> PT
        
        RP --> T
        IP --> T
        TP --> T
        SP --> T
        MP --> T
        CP --> T
        PT --> T
        
        T -->|impl| WB
        T -->|impl| MB
    end

    W --> R
    W -->|default| WB
    W -->|feature metal| MB

    style MB fill:#ff9,stroke:#333
    style T fill:#9cf,stroke:#333
```

### Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| Lifetime complexity of `RenderPass` borrowing | Trait design may require GATs | Use lifetime on `GpuBackend::RenderPass<'a>`; wgpu already uses this pattern |
| Dual path maintenance (flag on/off) | Two code paths to maintain | Keep non-flag path minimal — only used until `pluggable-backend-exp` stabilizes |
| Metal shader parity | Native sources can drift from WGSL | Keep paired MSL sources and Metal pixel coverage for built-in pipelines; custom pipelines provide their own backend-specific source |
| Pipeline cache trait abstraction | Backend-specific | Only expose cache operations that exist in both backends; fallback for Metal |
| `CustomPipeline` downstream breakage | API signature changes | Provide `CustomPipeline<WgpuBackend>` type alias for existing users |

# Testing

### Validation Approach

All validation is behind feature flags. The existing test suite runs unchanged when no flag is enabled.

### Key Scenarios

1. **Default path (no flags)** — all existing tests pass identically to current code
2. **`pluggable-backend-exp` enabled, default WgpuBackend** — all existing tests pass through the generic path (pixel-level regression tests in `deferred_frame_uploads`, `resized_text_preparation`, `scrolled_text_culling`)
3. **`metal` enabled on macOS** — Metal initializes all built-in pipelines and renders image, text, decoration, SVG, material, and rectangle content correctly

### Test Changes

- Existing tests in `lib.rs` remain unchanged for the non-flag path
- When `pluggable-backend-exp` is on, test `gpu()` helpers must use `WgpuBackend`-based initialization
- Metal backend needs platform-gated integration tests: `#[cfg(all(feature = "metal", target_os = "macos"))]`

### Edge Cases

- Metal-only builds without `wgpu` or `winit` in the normal dependency graph
- `pluggable-backend-exp` with neither `wgpu` nor `metal` (compile error, as intended)
- Metal backend on non-macOS platforms (compile error, as intended)
- Pipeline cache — `WgpuBackend` preserves it, `MetalBackend` returns `None`

# Delivery Steps

### ✓ Step 1: Backend trait layer + WgpuBackend adapter
The `backend/` module is created with the `GpuBackend` trait and all associated type traits, plus the `WgpuBackend` adapter implementation that wraps `wgpu::Device` and `wgpu::Queue`.

- Create `aimer_cupid/src/backend/mod.rs`:
  - Define `GpuBackend` trait with associated types: `Buffer`, `Texture`, `TextureView`, `BindGroupLayout`, `BindGroup`, `PipelineLayout`, `RenderPipeline`, `Sampler`, `ShaderModule`, `TextureFormat`
  - Define `GpuRenderPass<B: GpuBackend>` trait with `set_pipeline`, `set_bind_group`, `set_vertex_buffer`, `set_scissor_rect`, `draw`
  - Define helper types: `BindGroupLayoutEntry`, `BindGroupEntry`, `BindingResource`, `RenderPipelineDescriptor`, `RenderPassDescriptor`, `BufferDescriptor`, `GpuLimits` — as simple enums/structs that each backend maps to native types
  - The module is gated by `#[cfg(feature = "pluggable-backend-exp")]`

- Create `aimer_cupid/src/backend/wgpu.rs`:
  - `WgpuBackend` struct holding `wgpu::Device` + `wgpu::Queue`
  - `impl GpuBackend for WgpuBackend` — map each trait method to the corresponding wgpu call
  - Conversions from our simple descriptors to wgpu descriptor types

- Update `aimer_cupid/Cargo.toml`:
  - Make `wgpu` optional: `wgpu = { workspace = true, optional = true }`
  - Add `pluggable-backend-exp = ["dep:wgpu"]` feature
  - Add `metal` feature (wires to nothing yet, just declared)

- Update `aimer_cupid/src/lib.rs`:
  - Export `pub mod backend;` behind `#[cfg(feature = "pluggable-backend-exp")]`

This stage has no functional changes — the existing wgpu-only path is completely unchanged.

### ✓ Step 2: Generic pipelines and renderer
The `pluggable-backend-exp` path now has backend-generic versions of all six built-in pipelines and an offscreen `RendererImpl<B: GpuBackend>`. The existing concrete `Renderer` remains available as the compatibility API.

- Added backend-generic construction, preparation, upload, and rendering for `RectPipeline`, `ImagePipeline`, `TextPipelineV2`, `SvgPipeline`, `FrameCompositePipeline`, and `MaterialPipeline`.
- Added generic glyph-atlas operations and a backend-neutral text preparation path.
- Added `RendererImpl<B>` with antialiasing configuration, custom-pipeline registration, draw-list rendering, material backdrop handling, and retained-layer targets.
- Added `CustomPipelineGeneric<B>`, `RenderContextGeneric<B>`, and `PersistentTargetGeneric<B>`. `FrameUpload` remains backend-neutral because it only tracks CPU-side upload deduplication state.
- Added `GpuContext::backend()` and `GpuDevice` helpers to construct `WgpuBackend`; exported the experimental backend and renderer APIs conditionally.
- Added Wgpu pixel tests for generic rect rendering and image/text rendering.
- Kept the non-feature path compiling unchanged.

**Compatibility boundary:** the generic renderer is exposed as `RendererImpl<B>` alongside the existing `renderer::Renderer`. The latter still owns WGPU-specific scene/damage processing, compositor statistics, pipeline-cache, and multisampling APIs, so replacing it with a type alias now would drop those capabilities for feature-enabled callers. The alias convergence remains follow-up work after those APIs are ported.

Validation completed: `cargo check -p aimer_cupid --lib` and `cargo check -p aimer_cupid --features pluggable-backend-exp --lib` both pass. The focused generic Wgpu rect and image/text pixel tests pass.

### ✓ Step 3: Metal backend and rectangle proof of concept
Implemented `MetalBackend` behind the Apple-only `metal` feature, with the generic renderer's rectangle path as the first supported pipeline.

- Added Metal resource, command-buffer, render-pass, copy, readback, and pipeline operations. Bind groups map through a checked manual argument-index scheme.
- Added `MetalSurface` and `MetalSurfaceFrame` around `CAMetalLayer`: layers receive the backend device and format, frames expose the drawable texture/view, and presentation is queued after earlier work on the same command queue.
- Added the MSL rectangle shader and a `GpuBackend::rect_shader_source()` hook so `RectPipeline` selects WGSL or MSL through the backend.
- Added `RendererImpl::new_rect_only()` for staged backend bring-up or rectangle-only callers. The regular constructor initializes all six pipelines for WGPU and Metal.
- Added Apple-target-only `objc2-metal` and `objc2-quartz-core` dependencies. Enabling `metal` on other targets produces a compile-time error.
- Added renderer-level and low-level Metal pixel tests, including normalized vertex-color coverage.

Validation: `cargo check -p aimer_cupid --features metal --lib` and the no-feature `cargo check -p aimer_cupid --lib` pass on this Darwin host. All three focused Metal pixel tests pass here, including the generic renderer rectangle test, two-band readback/orientation coverage, and normalized vertex-color coverage. The WGPU generic-renderer rect and image/text pixel regressions also pass.

### ✓ Step 4: MSL ports for all built-in pipelines
All built-in shader modules now resolve to backend-specific source. WGPU keeps using the existing WGSL; Metal compiles matching MSL for image, monochrome text, color glyphs, text decorations, SVG, frame compositing, and materials, alongside the rectangle shader port from Step 3.

- Added `BuiltinShader` source selection to `GpuBackend`; the default maps to the existing WGSL assets and `MetalBackend` maps to checked-in MSL sources.
- Added shared MSL helpers for image/SVG color conversion and text clipping/color conversion.
- Preserved each pipeline's existing entry-point names, vertex locations, bind-group indices, premultiplied-alpha behavior, clipping, and color-space branches.
- Added a Metal test that constructs `RendererImpl::new()` to compile and create every built-in Metal pipeline.
- Added a Metal end-to-end pixel test covering image sampling, glyph text, text decorations, SVG rendering, the material shader, and the frame-composite pass.

Validation: `cargo test -p aimer_cupid --features metal` passes (567 library tests, 3 binary tests, 1 integration test, and 6 doctests; 5 library tests and 4 doctests are ignored). This includes all five Metal backend tests and the generic WGPU renderer tests. `cargo check -p aimer_cupid --lib`, `cargo check -p aimer_cupid --features metal --lib`, and `git diff --check` also pass on this Darwin host.

### ✓ Step 5: WGPU-free Metal feature build
The `metal` feature now builds Cupid without pulling WGPU or winit into its normal dependency graph. WGPU remains enabled by default; callers choose Metal-only with `--no-default-features --features metal`.

- Made `wgpu` and `winit` optional behind the `wgpu` feature, which remains part of the default feature set.
- Gated the WGPU compatibility renderer, context, pipeline cache, legacy custom-pipeline API, and WGPU-specific pipeline code behind `wgpu`.
- Set generic pipeline and renderer type defaults to WGPU when enabled and Metal when WGPU is disabled.
- Restricted Cupid's example binary to the `wgpu` feature; the generic Metal library remains buildable without it.
- Added a separate `cupid-metal` demo behind `metal-demo`. It creates a Winit window, attaches a `CAMetalLayer`, and draws through `RendererImpl<MetalBackend>` without enabling WGPU. The Metal and WGPU binaries share the same scene recorder for multilingual text, shaping samples, font weights, and color glyphs.

Run the Metal demo with `cargo run -p aimer_cupid --no-default-features --features metal-demo --bin cupid-metal`.

Validation on this Darwin host: `cargo check -p aimer_cupid --no-default-features --features metal` passes, and `cargo check -p aimer_cupid --no-default-features --features metal --tests` compiles the test targets without running them. `cargo build -p aimer_cupid --no-default-features --features metal-demo --bin cupid-metal` links the native demo executable. Its normal dependency graph contains no `wgpu`. Default and `pluggable-backend-exp` library checks pass, as does `git diff --check`.
# Historical plan — superseded by the stable backend implementation

The migration described below has been implemented. Current backend dependency
groups are `native`, `web`, and opt-in `wgpu`; target `cfg` selects native APIs,
and the generic `Renderer<B = DefaultGpuBackend>` is the public renderer.
References below to `pluggable-backend-exp`, backend-selection Cargo features,
or a separate WGPU-only `Renderer` describe the original proposal, not the
current configuration. This file remains for its design history.

