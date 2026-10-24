pub mod backend;
#[cfg(all(test, feature = "native", not(feature = "wgpu")))]
mod backend_tests {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    mod metal;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    mod opengl;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    mod vulkan;
}
pub mod compositor;
pub mod custom_pipeline;
#[doc(hidden)]
pub mod damage_region;
pub mod draw_cmd;
pub mod draw_cmd_v2;
// Font parsing, shaping and rasterization live in `aimer_cupid_font`.
pub use aimer_cupid_font::font;
pub mod frame;
#[cfg(feature = "wgpu")]
pub mod gpu_context;
mod persistent_target;
pub mod utilities;
#[doc(hidden)]
pub mod v2_frame_adapter;

pub mod canvas;
mod lru_map;
mod pipeline;
#[cfg(feature = "wgpu")]
pub use aimer_cupid_wgpu::pipeline_cache;
pub mod renderer;
pub mod shape;
// SVG parsing and tessellation live in `aimer_cupid_svg`; the path stays so
// `aimer_cupid::svg` and `crate::svg` keep resolving.
pub use aimer_cupid_svg as svg;
#[cfg(target_arch = "wasm32")]
pub mod wasm_fonts;

pub use pipeline::{
    AntiAlias, image_pipeline, material, rect_pipeline, svg_pipeline, text_pipeline,
};

pub use crate::text_pipeline::{glyph_atlas, glyph_rasterizer, text_layout};

// ── Backend-generic public surface ─────────────────────────────────────────
#[cfg(all(feature = "native", not(feature = "wgpu"), target_os = "windows"))]
pub use backend::dx12::Dx12Backend;
#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "linux", target_os = "android")
))]
pub use backend::opengl::{OpenGlBackend, OpenGlError, OpenGlSurface, OpenGlSurfaceFrame};
#[cfg(all(
    feature = "native",
    not(feature = "wgpu"),
    any(target_os = "linux", target_os = "android")
))]
pub use backend::vulkan::{VulkanBackend, VulkanError, VulkanSurface, VulkanSurfaceFrame};
#[cfg(all(feature = "web", target_arch = "wasm32", not(feature = "wgpu")))]
pub use backend::webgl::{WebGl2Backend, WebGl2Error, WebGl2TextureFormat, WebGl2TextureView};
#[cfg(all(feature = "web", target_arch = "wasm32", not(feature = "wgpu")))]
pub use backend::webgpu::{WebGpuBackend, WebGpuError, WebGpuTextureFormat, WebGpuTextureView};
#[cfg(feature = "wgpu")]
pub use backend::wgpu::WgpuBackend;
pub use backend::{GpuBackend, GpuLimits, GpuRenderPass};
pub use custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
#[cfg(feature = "wgpu")]
pub use gpu_context::GpuDevice;
pub use renderer::{Renderer, RendererImpl};

/// Hidden cargo-fuzz entry point for the checked font reader.
#[doc(hidden)]
pub fn fuzz_aimer_font_directory(data: &[u8]) {
    crate::pipeline::text_pipeline::aimer_font::fuzz_directory(data);
}

/// Hidden cargo-fuzz entry point for glyph and outline decoding.
#[doc(hidden)]
pub fn fuzz_aimer_font_outlines(data: &[u8]) {
    crate::pipeline::text_pipeline::aimer_font::fuzz_outlines(data);
}

#[cfg(all(test, feature = "wgpu"))]
mod wgpu_api_test_adapter {
    use crate::backend::wgpu::WgpuBackend;
    use crate::compositor::CompositorStats;
    use crate::damage_region::DamageSet;
    use crate::draw_cmd::DrawList;
    use crate::frame::{FramePacket, FrameRenderMetadata};
    use crate::renderer::{Renderer as GenericRenderer, RendererMemoryStats};
    use crate::text_pipeline::{
        TextDecorationDraw, TextDrawRequest, TextPipelineV2 as GenericTextPipeline,
        TextPreparationProfile,
    };

    /// Adapter used only by the existing GPU regression fixtures while they
    /// exercise the stable renderer through WgpuBackend.
    pub(super) struct Renderer {
        device: wgpu::Device,
        format: wgpu::TextureFormat,
        antialiasing: crate::AntiAlias,
        backend: Option<WgpuBackend>,
        inner: Option<GenericRenderer<WgpuBackend>>,
    }

    impl Renderer {
        pub fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
            Self::with_antialiasing(device, format, crate::AntiAlias::default())
        }

        pub fn with_antialiasing(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            antialiasing: crate::AntiAlias,
        ) -> Self {
            Self {
                device: device.clone(),
                format,
                antialiasing,
                backend: None,
                inner: None,
            }
        }

        fn with_backend<R>(
            &mut self,
            queue: &wgpu::Queue,
            f: impl FnOnce(&WgpuBackend, &mut GenericRenderer<WgpuBackend>) -> R,
        ) -> R {
            if self.backend.is_none() {
                self.backend = Some(WgpuBackend::new(self.device.clone(), queue.clone()));
            }
            if self.inner.is_none() {
                self.inner = Some(GenericRenderer::with_antialiasing(
                    self.backend.as_ref().expect("backend initialized"),
                    self.format,
                    self.antialiasing,
                ));
            }
            f(
                self.backend.as_ref().expect("backend initialized"),
                self.inner.as_mut().expect("renderer initialized"),
            )
        }

        pub fn render(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            width: u32,
            height: u32,
            is_srgb: bool,
            draw_list: &DrawList,
        ) {
            self.with_backend(queue, |backend, renderer| {
                renderer.render(backend, view, width, height, is_srgb, draw_list)
            });
        }

        pub fn render_with_source_texture(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            source: &wgpu::Texture,
            width: u32,
            height: u32,
            is_srgb: bool,
            draw_list: &DrawList,
        ) {
            self.with_backend(queue, |backend, renderer| {
                renderer.render_with_source_texture(
                    backend,
                    view,
                    Some(source),
                    width,
                    height,
                    is_srgb,
                    draw_list,
                )
            });
        }

        pub fn render_packet(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            packet: &FramePacket,
            is_srgb: bool,
        ) {
            self.with_backend(queue, |backend, renderer| {
                renderer.render_packet(backend, view, packet, is_srgb)
            });
        }

        pub fn render_packet_with_source_texture(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            source: &wgpu::Texture,
            packet: &FramePacket,
            is_srgb: bool,
        ) {
            self.with_backend(queue, |backend, renderer| {
                renderer.render_packet_with_source_texture(backend, view, source, packet, is_srgb)
            });
        }

        pub fn render_frame_with_metadata(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            view: &wgpu::TextureView,
            width: u32,
            height: u32,
            is_srgb: bool,
            draw_list: &DrawList,
            metadata: &FrameRenderMetadata,
        ) {
            self.with_backend(queue, |backend, renderer| {
                renderer.render_frame_with_metadata(
                    backend, view, width, height, is_srgb, draw_list, metadata,
                )
            });
        }

        pub fn memory_stats(&self) -> RendererMemoryStats {
            self.inner
                .as_ref()
                .map_or_else(RendererMemoryStats::default, GenericRenderer::memory_stats)
        }

        pub fn compositor_stats(&self) -> CompositorStats {
            self.inner
                .as_ref()
                .map_or_else(CompositorStats::default, GenericRenderer::compositor_stats)
        }
    }

    /// Adapter for text regression fixtures; all work is delegated to the
    /// backend-generic text pipeline.
    pub(super) struct TextPipelineV2 {
        device: wgpu::Device,
        format: wgpu::TextureFormat,
        antialiasing: crate::AntiAlias,
        backend: Option<WgpuBackend>,
        inner: Option<GenericTextPipeline<WgpuBackend>>,
    }

    impl TextPipelineV2 {
        pub fn new(
            device: &wgpu::Device,
            format: wgpu::TextureFormat,
            _cache: Option<&wgpu::PipelineCache>,
            antialiasing: crate::AntiAlias,
        ) -> Self {
            Self {
                device: device.clone(),
                format,
                antialiasing,
                backend: None,
                inner: None,
            }
        }

        fn with_backend<R>(
            &mut self,
            queue: &wgpu::Queue,
            f: impl FnOnce(&WgpuBackend, &mut GenericTextPipeline<WgpuBackend>) -> R,
        ) -> R {
            if self.backend.is_none() {
                self.backend = Some(WgpuBackend::new(self.device.clone(), queue.clone()));
            }
            if self.inner.is_none() {
                self.inner = Some(GenericTextPipeline::new(
                    self.backend.as_ref().unwrap(),
                    self.format,
                    self.antialiasing,
                ));
            }
            f(self.backend.as_ref().unwrap(), self.inner.as_mut().unwrap())
        }

        pub fn prepare(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            width: u32,
            height: u32,
            is_srgb: bool,
            requests: &[TextDrawRequest],
            decorations: &[TextDecorationDraw],
        ) {
            self.with_backend(queue, |backend, pipeline| {
                pipeline.prepare(backend, width, height, is_srgb, requests, decorations)
            });
        }

        pub fn prepare_profiled(
            &mut self,
            _device: &wgpu::Device,
            queue: &wgpu::Queue,
            width: u32,
            height: u32,
            is_srgb: bool,
            requests: &[TextDrawRequest],
            decorations: &[TextDecorationDraw],
        ) -> TextPreparationProfile {
            self.with_backend(queue, |backend, pipeline| {
                pipeline.prepare_profiled(backend, width, height, is_srgb, requests, decorations)
            })
        }

        pub fn frame_glyph_instances(&self) -> (usize, usize) {
            self.inner
                .as_ref()
                .map_or((0, 0), GenericTextPipeline::frame_glyph_instances)
        }

        pub fn layout_cache_entries(&self) -> usize {
            self.inner
                .as_ref()
                .map_or(0, GenericTextPipeline::layout_cache_entries)
        }

        pub fn has_postponed_preparation(&self) -> bool {
            self.inner
                .as_ref()
                .is_some_and(GenericTextPipeline::has_postponed_preparation)
        }
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod deferred_frame_uploads {
    //! Pixel-level regression guard for Cupid's per-frame instance uploads.
    //!
    //! The renderer interleaves rect, image and text batches inside one render
    //! pass, each batch drawing from its own region of a shared instance buffer,
    //! and the frame's data reaches the GPU in a single deferred write per
    //! pipeline that is skipped entirely when the bytes did not change. These
    //! tests pin down the observable contract of that scheme:
    //!
    //! * batches split by z-order interleaving must not alias or vanish;
    //! * a frame identical to the previous one must render the same pixels even
    //!   though its uploads were skipped;
    //! * a frame that *does* change must not be poisoned by the skip cache.
    //!
    //! Runs against the first available adapter; skips (with a note) on machines
    //! without one.

    use std::sync::Arc;

    use super::wgpu_api_test_adapter::Renderer;
    use crate::damage_region::{DamageRect, DamageSet};
    use crate::draw_cmd::{DrawList, RetainedLayerContent};
    use crate::frame::FrameRenderMetadata;
    use crate::pipeline::material::{MATERIAL_PIPELINE_NAME, MaterialKind, MaterialRequest};
    use crate::utilities::{Color, Rect};
    use aimer_utils::SyncFuture;

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .block()
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cupid upload regression device"),
                ..Default::default()
            })
            .block()
            .ok()
    }

    /// Renders draw into an offscreen target and returns the RGBA8 pixels.
    fn render_and_read(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        draw: &DrawList,
    ) -> Vec<u8> {
        render_and_read_with_capture(device, queue, renderer, draw, false)
    }

    /// Renders a draw list with either a caller-provided copy source or the
    /// renderer-owned material frame target and returns the RGBA8 pixels.
    fn render_and_read_with_capture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &mut Renderer,
        draw: &DrawList,
        capture_backdrop: bool,
    ) -> Vec<u8> {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("upload regression target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        if capture_backdrop {
            renderer
                .render_with_source_texture(device, queue, &view, &target, SIZE, SIZE, false, draw);
        } else {
            renderer.render(device, queue, &view, SIZE, SIZE, false, draw);
        }

        read_target(device, queue, &target)
    }

    /// Reads an offscreen RGBA8 target after the submitted work completes.
    fn read_target(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
        // SIZE * 4 = 256 bytes per row, which happens to satisfy wgpu's row
        // alignment requirement for texture-to-buffer copies.
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("upload regression readback"),
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("the readback buffer to map");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device to finish the readback");
        let pixels = slice
            .get_mapped_range()
            .expect("the mapped readback range")
            .to_vec();
        readback.unmap();
        pixels
    }

    fn solid_frame(background: Color, patch: Option<(Rect, Color)>) -> DrawList {
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            background,
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        if let Some((rect, color)) = patch {
            draw.fill_rect(rect, color, [0.0; 4], [0.0; 4], Color::transparent());
        }
        draw
    }

    fn material_frame(background: Color) -> DrawList {
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            background,
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let mut material = MaterialRequest::new(MaterialKind::Glass, [16.0, 16.0, 32.0, 32.0]);
        material.tint = [0.0, 0.0, 0.0, 1.0];
        material.opacity = 0.68;
        material.blur_radius = 0.0;
        material.border_color = [0.0, 0.0, 0.0, 0.0];
        material.corner_radii = [0.0; 4];
        material.shadow_color = [0.0, 0.0, 0.0, 0.0];
        draw.draw_custom(MATERIAL_PIPELINE_NAME, material.encode_for_test());
        draw
    }

    fn striped_material_frame(blur_radius: f32) -> DrawList {
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, 32.0, SIZE as f32),
            Color::black(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        draw.fill_rect(
            Rect::new(32.0, 0.0, 32.0, SIZE as f32),
            Color::white(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let mut material = MaterialRequest::new(MaterialKind::Glass, [16.0, 16.0, 32.0, 32.0]);
        // Keep tint out of this fixture so it measures backdrop diffusion alone.
        material.tint = [0.0, 0.0, 0.0, 0.0];
        material.opacity = 0.0;
        material.blur_radius = blur_radius;
        material.border_color = [0.0, 0.0, 0.0, 0.0];
        material.corner_radii = [0.0; 4];
        material.shadow_color = [0.0, 0.0, 0.0, 0.0];
        draw.draw_custom(MATERIAL_PIPELINE_NAME, material.encode_for_test());
        draw
    }

    fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
        let offset = ((y * SIZE + x) * 4) as usize;
        pixels[offset..offset + 4].try_into().expect("4 channels")
    }

    /// Asserts the pixel is saturated in exactly the dominant RGB channel.
    fn assert_dominant(pixels: &[u8], x: u32, y: u32, dominant: usize, what: &str) {
        let sample = pixel(pixels, x, y);
        for channel in 0..3 {
            if channel == dominant {
                assert!(
                    sample[channel] > 200,
                    "{what}: expected channel {channel} saturated at ({x}, {y}), got {sample:?}"
                );
            } else {
                assert!(
                    sample[channel] < 50,
                    "{what}: expected channel {channel} dark at ({x}, {y}), got {sample:?}"
                );
            }
        }
        assert!(sample[3] > 200, "{what}: expected opaque at ({x}, {y})");
    }

    /// A frame whose rect stream is split by an image draw, forcing multiple
    /// rect flushes and an interleaved image batch inside the same pass: a red
    /// background, a blue 1×1 image stretched over the middle, and a green square
    /// on top of everything.
    fn interleaved_frame(background: Color, square: Color) -> DrawList {
        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            background,
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let texture_id = draw.load_image(&[0, 0, 255, 255], 1, 1);
        draw.draw_image(Rect::new(16.0, 16.0, 16.0, 16.0), texture_id);
        draw.fill_rect(
            Rect::new(40.0, 40.0, 8.0, 8.0),
            square,
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        draw
    }

    fn assert_interleaved_pixels(pixels: &[u8], what: &str) {
        // Background rect (first batch).
        assert_dominant(pixels, 2, 2, 0, what);
        // Image drawn between the two rect batches.
        assert_dominant(pixels, 20, 20, 2, what);
        // Rect batch after the image — the region that would vanish if a later
        // upload overwrote an earlier batch, or alias if offsets were wrong.
        assert_dominant(pixels, 44, 44, 1, what);
    }

    #[test]
    fn interleaved_batches_render_identically_across_static_frames() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        // Frame 1: every buffer is fresh, everything uploads.
        let frame = interleaved_frame(Color::red(), Color::green());
        let first = render_and_read(&device, &queue, &mut renderer, &frame);
        assert_interleaved_pixels(&first, "first frame");

        // Frame 2: byte-identical draw list — uploads may be skipped, pixels must
        // not change.
        let frame = interleaved_frame(Color::red(), Color::green());
        let second = render_and_read(&device, &queue, &mut renderer, &frame);
        assert_interleaved_pixels(&second, "identical second frame");
        assert_eq!(first, second, "a static frame must be pixel-stable");
    }

    #[test]
    fn a_changed_frame_is_not_poisoned_by_the_skip_cache() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        let frame = interleaved_frame(Color::red(), Color::green());
        let first = render_and_read(&device, &queue, &mut renderer, &frame);
        assert_interleaved_pixels(&first, "baseline frame");

        // Same shape, swapped colors: the changed bytes must reach the GPU.
        let frame = interleaved_frame(Color::green(), Color::red());
        let pixels = render_and_read(&device, &queue, &mut renderer, &frame);
        assert_dominant(&pixels, 2, 2, 1, "swapped background");
        assert_dominant(&pixels, 20, 20, 2, "image after swap");
        assert_dominant(&pixels, 44, 44, 0, "swapped square");
    }

    #[test]
    fn material_reads_the_already_rendered_backdrop() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        let red = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::red()),
            true,
        );
        let blue = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::blue()),
            true,
        );
        let red_sample = pixel(&red, 32, 32);
        let blue_sample = pixel(&blue, 32, 32);
        let difference = red_sample[..3]
            .iter()
            .zip(&blue_sample[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .map(u16::from)
            .sum::<u16>();
        let red_with_internal_target = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::red()),
            false,
        );
        let internal_sample = pixel(&red_with_internal_target, 32, 32);
        let path_difference = red_sample[..3]
            .iter()
            .zip(&internal_sample[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .map(u16::from)
            .sum::<u16>();
        assert!(
            difference > 24,
            "material backdrop should respond to the preceding draw: red={red_sample:?}, blue={blue_sample:?}"
        );
        assert!(
            path_difference <= 3,
            "the internal shader target should preserve the explicit capture result: explicit={red_sample:?}, internal={internal_sample:?}"
        );
    }

    #[test]
    fn multisampled_material_reads_the_already_rendered_backdrop() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::with_antialiasing(&device, FORMAT, crate::AntiAlias::Msaa4x);

        let red = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::red()),
            true,
        );
        let blue = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::blue()),
            true,
        );
        let red_sample = pixel(&red, 32, 32);
        let blue_sample = pixel(&blue, 32, 32);
        let difference = red_sample[..3]
            .iter()
            .zip(&blue_sample[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .map(u16::from)
            .sum::<u16>();
        let red_with_internal_target = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &material_frame(Color::red()),
            false,
        );
        let internal_sample = pixel(&red_with_internal_target, 32, 32);
        let path_difference = red_sample[..3]
            .iter()
            .zip(&internal_sample[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .map(u16::from)
            .sum::<u16>();

        assert!(
            difference > 24,
            "multisampled material backdrop should respond to the preceding draw: red={red_sample:?}, blue={blue_sample:?}"
        );
        assert!(
            path_difference <= 3,
            "the multisampled internal shader target should preserve the explicit capture result: explicit={red_sample:?}, internal={internal_sample:?}"
        );
    }

    #[test]
    fn material_frosted_kernel_softens_a_backdrop_boundary() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        let sharp = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &striped_material_frame(0.0),
            true,
        );
        let blurred = render_and_read_with_capture(
            &device,
            &queue,
            &mut renderer,
            &striped_material_frame(96.0),
            true,
        );

        let sharp_edge = pixel(&sharp, 32, 32);
        let blurred_edge = pixel(&blurred, 32, 32);
        let difference = sharp_edge[..3]
            .iter()
            .zip(&blurred_edge[..3])
            .map(|(left, right)| left.abs_diff(*right))
            .map(u16::from)
            .sum::<u16>();
        assert!(
            difference > 12,
            "changing the frosted radius should change a backdrop edge: sharp={sharp_edge:?}, blurred={blurred_edge:?}"
        );
    }

    #[test]
    fn retained_layer_is_rasterized_and_then_composited_at_a_new_offset() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        let mut recorded = DrawList::new();
        recorded.fill_rect(
            Rect::new(0.0, 0.0, 16.0, 16.0),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );
        let content = Arc::new(RetainedLayerContent::from_snapshot(
            recorded
                .retained_snapshot()
                .expect("the layer content should be retainable"),
        ));

        let mut first_frame = DrawList::new();
        first_frame.draw_retained_layer(7, Rect::new(4.0, 4.0, 16.0, 16.0), content.clone());
        let first = render_and_read(&device, &queue, &mut renderer, &first_frame);
        assert_dominant(&first, 8, 8, 0, "first retained layer position");
        assert_eq!(renderer.memory_stats().retained_layer_count, 1);

        let mut second_frame = DrawList::new();
        second_frame.draw_retained_layer(7, Rect::new(12.0, 12.0, 16.0, 16.0), content);
        let second = render_and_read(&device, &queue, &mut renderer, &second_frame);
        assert_dominant(&second, 16, 16, 0, "composited retained layer position");
        assert_eq!(renderer.memory_stats().retained_layer_count, 1);
        assert_eq!(pixel(&second, 8, 8)[3], 0, "the layer should have moved");
    }

    #[test]
    fn partial_damage_repaints_the_region_and_preserves_the_persistent_target() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("partial damage regression target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let first_frame = solid_frame(Color::red(), None);
        renderer.render_frame_with_metadata(
            &device,
            &queue,
            &view,
            SIZE,
            SIZE,
            false,
            &first_frame,
            &FrameRenderMetadata::full(SIZE, SIZE),
        );
        let first = read_target(&device, &queue, &target);
        assert_dominant(&first, 2, 2, 0, "persistent target baseline");

        let second_frame = solid_frame(
            Color::red(),
            Some((Rect::new(8.0, 8.0, 8.0, 8.0), Color::blue())),
        );
        let mut damage = DamageSet::new(SIZE, SIZE);
        damage.add(DamageRect::new(6, 6, 12, 12));
        let metadata = FrameRenderMetadata::new(1.0, 0, 0, 0, 0, damage);
        renderer.render_frame_with_metadata(
            &device,
            &queue,
            &view,
            SIZE,
            SIZE,
            false,
            &second_frame,
            &metadata,
        );
        let second = read_target(&device, &queue, &target);

        assert_dominant(&second, 2, 2, 0, "undamaged persistent pixels");
        assert_dominant(&second, 10, 10, 2, "repainted damaged pixels");
    }

    #[test]
    fn disjoint_damage_regions_are_repainted_without_touching_the_rest() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("multi-region damage regression target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        let baseline = solid_frame(Color::red(), None);
        renderer.render_frame_with_metadata(
            &device,
            &queue,
            &view,
            SIZE,
            SIZE,
            false,
            &baseline,
            &FrameRenderMetadata::full(SIZE, SIZE),
        );

        let second_frame = {
            let mut frame = solid_frame(Color::red(), None);
            frame.fill_rect(
                Rect::new(4.0, 4.0, 8.0, 8.0),
                Color::blue(),
                [0.0; 4],
                [0.0; 4],
                Color::transparent(),
            );
            frame.fill_rect(
                Rect::new(44.0, 44.0, 8.0, 8.0),
                Color::green(),
                [0.0; 4],
                [0.0; 4],
                Color::transparent(),
            );
            frame
        };
        let mut damage = DamageSet::new(SIZE, SIZE);
        damage.add(DamageRect::new(2, 2, 12, 12));
        damage.add(DamageRect::new(42, 42, 12, 12));
        assert_eq!(damage.regions().len(), 2);
        renderer.render_frame_with_metadata(
            &device,
            &queue,
            &view,
            SIZE,
            SIZE,
            false,
            &second_frame,
            &FrameRenderMetadata::new(1.0, 0, 0, 0, 0, damage),
        );

        let pixels = read_target(&device, &queue, &target);
        let compositor_stats = renderer.compositor_stats();
        assert_eq!(compositor_stats.damage_regions, 2);
        assert_eq!(compositor_stats.composition_passes, 1);
        assert!(!compositor_stats.full_repaint);
        assert_dominant(&pixels, 8, 8, 2, "first disjoint damaged region");
        assert_dominant(&pixels, 48, 48, 1, "second disjoint damaged region");
        assert_dominant(&pixels, 32, 32, 0, "undamaged pixels between regions");
    }

    #[test]
    fn an_empty_frame_between_static_frames_keeps_the_scene_intact() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let mut renderer = Renderer::new(&device, FORMAT);

        let frame = interleaved_frame(Color::red(), Color::green());
        render_and_read(&device, &queue, &mut renderer, &frame);

        // An empty frame draws nothing and must not disturb the retained buffers.
        let empty = DrawList::new();
        let blank = render_and_read(&device, &queue, &mut renderer, &empty);
        assert_eq!(
            pixel(&blank, 2, 2)[3],
            0,
            "an empty frame clears the target"
        );

        let frame = interleaved_frame(Color::red(), Color::green());
        let third = render_and_read(&device, &queue, &mut renderer, &frame);
        assert_interleaved_pixels(&third, "frame after an empty frame");
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod resized_text_preparation {
    //! Regression guard for text preparation during a live window resize.
    //!
    //! A wrapped layout is keyed by the width it wraps at, so a resize frame
    //! misses the layout cache for everything on screen — that re-layout is what
    //! the frame owes the screen. The off-screen tail a viewport asked for ahead
    //! of itself is a different matter: laying it out at a width that will be
    //! different again next frame is work the next frame throws away, and it
    //! floods the layout cache with entries nothing will ever read. A resize
    //! frame must therefore postpone the tail and prepare it on the first frame
    //! whose size has settled.
    //!
    //! Runs against the first available adapter; skips (with a note) on machines
    //! without one.

    use std::sync::Arc;

    use super::wgpu_api_test_adapter::TextPipelineV2;
    use crate::AntiAlias;
    use crate::font::{FontFamily, FontStyle};
    use crate::text_layout::TextHorizontalAlign;
    use crate::text_pipeline::{
        RichTextSpan, TextDrawRequest, TextOverflowMode, TextShadowRequest,
    };
    use crate::utilities::Rgba8;
    use aimer_utils::SyncFuture;

    const HEIGHT: u32 = 300;
    const LINE_HEIGHT: f32 = 24.0;
    /// How many times taller than the viewport the document is.
    const OVERDRAW_FACTOR: usize = 4;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .block()
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("resized text preparation device"),
                ..Default::default()
            })
            .block()
            .ok()
    }

    fn pipeline(device: &wgpu::Device) -> TextPipelineV2 {
        TextPipelineV2::new(
            device,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            AntiAlias::Analytic,
        )
    }

    /// One wrapped line of a document, sized against the surface width the way a
    /// full-width column of text is — resizing the window changes every line's
    /// wrapping width.
    fn line(index: usize, surface_width: u32, scroll_offset: f32) -> TextDrawRequest {
        TextDrawRequest {
            x: 8.0,
            y: index as f32 * LINE_HEIGHT - scroll_offset,
            text: Arc::from(format!("line {index} with text that wraps to its column").as_str()),
            font_size: 16.0,
            color: Rgba8::new(0, 0, 0, 255),
            bounds_width: surface_width as f32 - 16.0,
            bounds_height: LINE_HEIGHT,
            overflow: TextOverflowMode::Wrap,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: crate::text_pipeline::text_layout::TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        }
    }

    fn document(surface_width: u32, scroll_offset: f32) -> Vec<TextDrawRequest> {
        let line_count = (HEIGHT as f32 / LINE_HEIGHT).ceil() as usize * OVERDRAW_FACTOR;
        (0..line_count)
            .map(|index| line(index, surface_width, scroll_offset))
            .collect()
    }

    /// Prepares requests at a fixed size until nothing is postponed.
    fn prepare_until_settled(
        pipeline: &mut TextPipelineV2,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        requests: &[TextDrawRequest],
    ) {
        for _ in 0..100 {
            pipeline.prepare(device, queue, width, HEIGHT, false, requests, &[]);
            if !pipeline.has_postponed_preparation() {
                return;
            }
        }
        panic!("text preparation never settled");
    }

    #[test]
    fn moving_shadowed_rich_text_reuses_its_paint_templates() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let mut pipeline = pipeline(&device);
        let mut first = line(0, 400, 0.0);
        first.text = Arc::from("shadowed rich text");
        first.spans = vec![
            RichTextSpan::new("shadowed "),
            RichTextSpan::new("rich text"),
        ];
        first.overflow = TextOverflowMode::Clip;
        first.bounds_height = 0.0;
        first.shadow = Some(TextShadowRequest {
            offset_x: 2.0,
            offset_y: 1.0,
            blur: 3.0,
            color: Rgba8::new(0, 0, 0, 160),
        });
        let first_profile = pipeline.prepare_profiled(
            &device,
            &queue,
            400,
            HEIGHT,
            false,
            std::slice::from_ref(&first),
            &[],
        );
        assert_eq!(first_profile.paint_cache_misses, 2);

        let mut moved = first;
        moved.x += 17.5;
        moved.color = Rgba8::new(32, 64, 96, 255);
        moved.spans[1].color = Some(Rgba8::new(200, 40, 80, 255));
        moved.clip_rect = [8.0, 0.0, 360.0, HEIGHT as f32];
        let second_profile = pipeline.prepare_profiled(
            &device,
            &queue,
            400,
            HEIGHT,
            false,
            std::slice::from_ref(&moved),
            &[],
        );

        assert_eq!(second_profile.paint_cache_hits, 2);
        assert_eq!(second_profile.paint_cache_misses, 0);
        assert!(pipeline.frame_glyph_instances().0 > 0);
    }

    // The heart of the guard: the frame whose surface size changed must draw its
    // visible text and postpone the tail, not spend its budget laying the tail
    // out at a width the next resize frame invalidates.
    #[test]
    fn a_resize_frame_postpones_the_off_screen_tail() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let mut pipeline = pipeline(&device);
        prepare_until_settled(&mut pipeline, &device, &queue, 400, &document(400, 0.0));

        // One live-resize frame: a new surface width, every wrapping width new.
        let resized = document(360, 0.0);
        pipeline.prepare(&device, &queue, 360, HEIGHT, false, &resized, &[]);

        let (alpha, _) = pipeline.frame_glyph_instances();
        assert!(alpha > 0, "the resize frame must still draw visible text");
        assert!(
            pipeline.has_postponed_preparation(),
            "a resize frame must postpone the off-screen tail instead of \
             laying it out at a width the next frame invalidates"
        );
    }

    // A wrapped layout only depends on the width when the text wraps at it. A
    // document of lines that fit their column at every width of a drag must
    // therefore not mint a fresh layout set per width — after the first step of
    // the resize, every later step reuses the same width-independent layouts.
    #[test]
    fn a_width_change_that_wraps_nothing_reuses_every_layout() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let mut pipeline = pipeline(&device);
        prepare_until_settled(&mut pipeline, &device, &queue, 400, &document(400, 0.0));

        // The first resize step pays the one-time canonicalization of the
        // document's layouts; none of the lines wrap at 398px.
        pipeline.prepare(
            &device,
            &queue,
            398,
            HEIGHT,
            false,
            &document(398, 0.0),
            &[],
        );
        prepare_until_settled(&mut pipeline, &device, &queue, 398, &document(398, 0.0));
        let after_first_step = pipeline.layout_cache_entries();

        // Every further step of the drag must reuse those layouts wholesale.
        for width in [396, 394, 392, 390] {
            let resized = document(width, 0.0);
            pipeline.prepare(&device, &queue, width, HEIGHT, false, &resized, &[]);
            prepare_until_settled(&mut pipeline, &device, &queue, width, &resized);

            let (alpha, _) = pipeline.frame_glyph_instances();
            assert!(alpha > 0, "the resize frame must still draw visible text");
            assert_eq!(
                pipeline.layout_cache_entries(),
                after_first_step,
                "a width change that wraps nothing must not mint new layouts at {width}px"
            );
        }
    }

    // Postponing must not lose the tail: once the size settles, the whole
    // document is prepared, and scrolling to its far end draws glyphs.
    #[test]
    fn text_arriving_after_a_resize_settles_draws_its_glyphs() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let mut pipeline = pipeline(&device);
        prepare_until_settled(&mut pipeline, &device, &queue, 400, &document(400, 0.0));

        // The resize frame itself, then the settled frames after it.
        pipeline.prepare(
            &device,
            &queue,
            360,
            HEIGHT,
            false,
            &document(360, 0.0),
            &[],
        );
        prepare_until_settled(&mut pipeline, &device, &queue, 360, &document(360, 0.0));
        let (top_alpha, _) = pipeline.frame_glyph_instances();
        assert!(top_alpha > 0, "settled text must draw glyphs");

        // Scroll to the bottom: the postponed tail is what is visible now.
        let visible_lines = (HEIGHT as f32 / LINE_HEIGHT).ceil() as usize;
        let line_count = visible_lines * OVERDRAW_FACTOR;
        let bottom_offset = (line_count - visible_lines) as f32 * LINE_HEIGHT;
        pipeline.prepare(
            &device,
            &queue,
            360,
            HEIGHT,
            false,
            &document(360, bottom_offset),
            &[],
        );
        let (bottom_alpha, _) = pipeline.frame_glyph_instances();
        assert!(
            bottom_alpha * 2 > top_alpha,
            "the tail prepared after the resize lost most of its glyphs: \
             top {top_alpha}, bottom {bottom_alpha}"
        );
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod scrolled_text_culling {
    //! Regression guard for request-level text culling.
    //!
    //! A scroll viewport hands its child more text than the screen can show, so a
    //! text-heavy document reaches TextPipelineV2::prepare with most of its
    //! requests off screen. Those requests must not cost the frame anything
    //! per-glyph: no instances built, no bytes uploaded, no atlas capacity
    //! reserved. The visibility rule is the one the pipeline already trusts for
    //! postponing preparation — a request whose bounds meet neither the surface
    //! nor its clip cannot show a pixel.
    //!
    //! Runs against the first available adapter; skips (with a note) on machines
    //! without one.

    use std::sync::Arc;

    use super::wgpu_api_test_adapter::TextPipelineV2;
    use crate::AntiAlias;
    use crate::font::{FontFamily, FontStyle};
    use crate::text_layout::TextHorizontalAlign;
    use crate::text_pipeline::{TextDrawRequest, TextOverflowMode};
    use crate::utilities::Rgba8;
    use aimer_utils::SyncFuture;

    const WIDTH: u32 = 400;
    const HEIGHT: u32 = 300;
    const LINE_HEIGHT: f32 = 24.0;
    const FONT_SIZE: f32 = 16.0;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .block()
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("text culling regression device"),
                ..Default::default()
            })
            .block()
            .ok()
    }

    fn pipeline(device: &wgpu::Device) -> TextPipelineV2 {
        TextPipelineV2::new(
            device,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            AntiAlias::Analytic,
        )
    }

    /// One line of a document, index line-heights below the document origin,
    /// shifted up by scroll_offset — unclipped, the way content that relies on
    /// the surface edge alone arrives.
    fn line(index: usize, scroll_offset: f32) -> TextDrawRequest {
        TextDrawRequest {
            x: 8.0,
            y: index as f32 * LINE_HEIGHT - scroll_offset,
            text: Arc::from(format!("line {index} with some scrolling text").as_str()),
            font_size: FONT_SIZE,
            color: Rgba8::new(0, 0, 0, 255),
            bounds_width: WIDTH as f32 - 16.0,
            bounds_height: LINE_HEIGHT,
            overflow: TextOverflowMode::Clip,
            horizontal_align: TextHorizontalAlign::Left,
            writing_mode: crate::text_pipeline::text_layout::TextWritingMode::HorizontalTb,
            line_height: None,
            shadow: None,
            draw_glyphs: true,
            font_family: FontFamily::SANS_SERIF,
            font_style: FontStyle::Normal,
            font_weight: None,
            language: None,
            italic: false,
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            spans: Vec::new(),
        }
    }

    /// Prepares requests until nothing is postponed, so every layout — visible
    /// or ahead of view — is cached and the last frame is a steady-state one.
    fn prepare_until_settled(
        pipeline: &mut TextPipelineV2,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        requests: &[TextDrawRequest],
    ) {
        for _ in 0..100 {
            pipeline.prepare(device, queue, WIDTH, HEIGHT, false, requests, &[]);
            if !pipeline.has_postponed_preparation() {
                return;
            }
        }
        panic!("text preparation never settled");
    }

    // The heart of the guard: a fully prepared document whose tail hangs far
    // below the surface must produce exactly the instances its visible head
    // produces alone. Anything more means the frame is building and uploading
    // quads the screen can never show.
    #[test]
    fn off_screen_requests_build_no_instances() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        // Ceiling: a line straddling the bottom edge still shows its top row.
        //
        // The pipeline also keeps a request whose glyphs overhang its declared
        // box — an ascender or italic left-bearing reaches up to a font size past
        // the origin — so the head the screen actually shows runs a font size
        // deeper than the raw line count. FONT_SIZE is that overhang padding.
        let visible_lines = ((HEIGHT as f32 + FONT_SIZE) / LINE_HEIGHT).ceil() as usize;
        let document: Vec<_> = (0..visible_lines * 4).map(|i| line(i, 0.0)).collect();

        let mut head_only = pipeline(&device);
        prepare_until_settled(&mut head_only, &device, &queue, &document[..visible_lines]);
        let (head_alpha, head_color) = head_only.frame_glyph_instances();
        assert!(head_alpha > 0, "the visible head must produce glyphs");

        let mut whole_document = pipeline(&device);
        prepare_until_settled(&mut whole_document, &device, &queue, &document);
        assert_eq!(
            whole_document.frame_glyph_instances(),
            (head_alpha, head_color),
            "off-screen requests must contribute no glyph instances"
        );
    }

    // Culling must not eat text on its way in: after scrolling a culled line into
    // view, its glyphs must be drawn — the ahead-of-view preparation it received
    // while off screen has to pay off on the arrival frame.
    #[test]
    fn a_culled_request_scrolled_into_view_draws_its_glyphs() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let visible_lines = (HEIGHT as f32 / LINE_HEIGHT).ceil() as usize;
        let line_count = visible_lines * 4;

        let mut pipeline = pipeline(&device);
        let document: Vec<_> = (0..line_count).map(|i| line(i, 0.0)).collect();
        prepare_until_settled(&mut pipeline, &device, &queue, &document);
        let (top_alpha, _) = pipeline.frame_glyph_instances();

        // Scroll to the bottom: a completely different set of lines is visible.
        let bottom_offset = (line_count - visible_lines) as f32 * LINE_HEIGHT;
        let scrolled: Vec<_> = (0..line_count).map(|i| line(i, bottom_offset)).collect();
        pipeline.prepare(&device, &queue, WIDTH, HEIGHT, false, &scrolled, &[]);
        let (bottom_alpha, _) = pipeline.frame_glyph_instances();

        assert!(
            bottom_alpha > 0,
            "lines scrolled into view must draw glyphs"
        );
        // The bottom shows as many full lines as the top did (same line height,
        // same surface), so the instance count must be in the same ballpark —
        // a fraction of it would mean arrived text is missing glyphs.
        assert!(
            bottom_alpha * 2 > top_alpha,
            "arrived text lost most of its glyphs: top {top_alpha}, bottom {bottom_alpha}"
        );
    }
}

// ── Generic WgpuBackend path tests ──────────────────────────────────────
//
// These tests exercise the backend-generic pipeline path through `WgpuBackend`,
// proving that the stable API produces the expected pixels.

#[cfg(all(test, feature = "wgpu"))]
mod generic_backend_tests {
    use aimer_utils::SyncFuture;

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions::default())
            .block()
            .ok()?;
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("cupid generic-backend device"),
                ..Default::default()
            })
            .block()
            .ok()
    }

    fn read_target(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> Vec<u8> {
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("generic-backend readback"),
            size: (SIZE * SIZE * 4) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(SIZE * 4),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(encoder.finish()));

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |result| {
            result.expect("the readback buffer to map");
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the device to finish the readback");
        let pixels = slice
            .get_mapped_range()
            .expect("the mapped readback range")
            .to_vec();
        readback.unmap();
        pixels
    }

    fn pixel(data: &[u8], x: u32, y: u32) -> [u8; 4] {
        let stride = SIZE as usize * 4;
        let offset = y as usize * stride + x as usize * 4;
        [
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]
    }

    fn red_rect_instance() -> crate::rect_pipeline::RectInstance {
        crate::rect_pipeline::RectInstance {
            position: [0.0, 0.0],
            size: [SIZE as f32, SIZE as f32],
            color: crate::utilities::Rgba8::new(255, 0, 0, 255),
            border_radius: [0.0; 4],
            border_width: [0.0; 4],
            border_color: crate::utilities::Rgba8::TRANSPARENT,
            outline_width: [0.0; 4],
            outline_color: crate::utilities::Rgba8::TRANSPARENT,
            // Shader contract: width < 0 disables clipping. width == 0 fully
            // clips (alpha 0). The renderer uses [-1 width] via clip_to_array.
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            shadow_params: [0.0; 4],
            shadow_color: crate::utilities::Rgba8::TRANSPARENT,
            shadow_flags: [0.0; 4],
        }
    }

    /// Control: concrete RectPipeline with the same lifecycle the renderer uses
    /// (begin_frame → push → pass/flush → end_frame → submit).
    #[test]
    fn concrete_rect_path_control_renders_red() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device.clone(), queue.clone());
        let mut rect =
            crate::rect_pipeline::RectPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concrete rect control target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        // Exact renderer lifecycle.
        rect.begin_frame(&backend, 1, SIZE, SIZE, false);
        rect.push(red_rect_instance());

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concrete rect control pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            rect.flush(&mut pass);
        }
        rect.end_frame(&backend);
        backend.submit(encoder);

        let pixels = read_target(&device, &queue, &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "concrete control path: center must be red, got {sample:?}"
        );
    }

    #[test]
    fn generic_rect_path_renders_through_wgpu_backend() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device, queue);

        // Create rect pipeline through the generic (backend-driven) path.
        let mut rect =
            crate::rect_pipeline::RectPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        // Create an offscreen render target.
        let target = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic rect test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopySrc,
            ],
        });
        let target_view = backend.create_texture_view(&target, "generic rect test target view");

        // Exact renderer lifecycle via generic methods:
        // begin_frame → push → pass/flush → end_frame → submit.
        rect.begin_frame(&backend, 1, SIZE, SIZE, false);
        rect.push(red_rect_instance());

        let mut encoder = backend.create_command_encoder("generic rect test encoder");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &crate::backend::RenderPassDescriptor {
                    label: Some("generic rect test pass".to_string()),
                    color_attachments: &[crate::backend::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: crate::backend::Operations {
                            load: crate::backend::LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                            store: crate::backend::StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );

            // Concrete flush is valid because
            // WgpuBackend::RenderPass == wgpu::RenderPass.
            rect.flush(&mut pass);
        }

        rect.end_frame(&backend);
        backend.submit(encoder);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "generic rect path: center must be red, got {sample:?}"
        );
    }

    /// Exercise FrameCompositePipeline::new + create_bind_group
    /// by compositing a solid-red source texture into a black dest and reading
    /// back the center pixel.
    #[test]
    fn generic_frame_composite_path_copies_source() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device, queue);

        let composite =
            crate::pipeline::frame_composite::FrameCompositePipeline::new(&backend, FORMAT);

        // Source: solid red, readable as a texture binding + filled via write_texture.
        let source = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic composite source".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::TextureBinding,
                crate::backend::TextureUsage::CopyDst,
            ],
        });
        let source_view = backend.create_texture_view(&source, "generic composite source view");
        let red_pixel = [255u8, 0, 0, 255];
        let mut red_bytes = vec![0u8; (SIZE * SIZE * 4) as usize];
        for chunk in red_bytes.chunks_exact_mut(4) {
            chunk.copy_from_slice(&red_pixel);
        }
        backend.write_texture(&crate::backend::WriteTextureDescriptor {
            texture: &source,
            mip_level: 0,
            origin: crate::backend::Origin3d { x: 0, y: 0, z: 0 },
            aspect: crate::backend::TextureAspect::All,
            data: &red_bytes,
            buffer_layout: crate::backend::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
            extent: crate::backend::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        });

        let dest = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic composite dest".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopySrc,
            ],
        });
        let dest_view = backend.create_texture_view(&dest, "generic composite dest view");

        let bind_group = composite.create_bind_group(&backend, &source_view);

        let mut encoder = backend.create_command_encoder("generic composite encoder");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &crate::backend::RenderPassDescriptor {
                    label: Some("generic composite pass".to_string()),
                    color_attachments: &[crate::backend::RenderPassColorAttachment {
                        view: &dest_view,
                        resolve_target: None,
                        ops: crate::backend::Operations {
                            load: crate::backend::LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                            store: crate::backend::StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            // Concrete render is valid: WgpuBackend::RenderPass == wgpu::RenderPass.
            composite.render(&mut pass, &bind_group);
        }
        backend.submit(encoder);

        let pixels = read_target(backend.device(), backend.queue(), &dest);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "generic frame composite path: center must be red, got {sample:?}"
        );
    }

    /// Control: concrete ImagePipeline with the same lifecycle the renderer
    /// uses (begin_frame → upload_image → pass/draw_batch → end_frame → submit).
    #[test]
    fn concrete_image_path_control_renders_red() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device.clone(), queue.clone());
        let mut image_pipeline = crate::pipeline::image_pipeline::ImagePipeline::new(
            &backend,
            FORMAT,
            crate::AntiAlias::Analytic,
        );

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concrete image control target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        // Upload a solid-red 2x2 texture.
        let red = [255u8, 0, 0, 255];
        let mut tex_data = Vec::with_capacity(2 * 2 * 4);
        for _ in 0..4 {
            tex_data.extend_from_slice(&red);
        }
        let tex_id = image_pipeline.upload_image(&backend, 2, 2, &tex_data);
        assert!(image_pipeline.has_texture(tex_id));

        // Exact renderer lifecycle.
        image_pipeline.begin_frame(&backend, 1, SIZE, SIZE, false);

        let instance = crate::pipeline::image_pipeline::ImageInstance {
            position: [0.0, 0.0],
            size: [SIZE as f32, SIZE as f32],
            uv_offset: [0.0, 0.0],
            uv_scale: [1.0, 1.0],
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            alpha: 1.0,
            source_premultiplied: 0.0,
        };

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concrete image control pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            image_pipeline.draw_batch(&backend, &mut pass, tex_id, &[instance]);
        }
        image_pipeline.end_frame(&backend);
        backend.submit(encoder);

        let pixels = read_target(&device, &queue, &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "concrete image control path: center must be red, got {sample:?}"
        );
    }

    /// Exercise ImagePipeline::new + upload_image +
    /// begin_frame + end_frame by uploading a texture through
    /// the backend and rendering it.
    #[test]
    fn generic_image_path_renders_uploaded_texture() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device, queue);

        // Create image pipeline through the generic (backend-driven) path.
        let mut image_pipeline = crate::pipeline::image_pipeline::ImagePipeline::new(
            &backend,
            FORMAT,
            crate::AntiAlias::Analytic,
        );

        // Upload a solid-red 2x2 texture through the generic path.
        let red = [255u8, 0, 0, 255];
        let mut tex_data = Vec::with_capacity(2 * 2 * 4);
        for _ in 0..4 {
            tex_data.extend_from_slice(&red);
        }
        let tex_id = image_pipeline.upload_image(&backend, 2, 2, &tex_data);
        assert!(image_pipeline.has_texture(tex_id));
        assert_eq!(image_pipeline.texture_count(), 1);
        // 2x2 RGBA8 = 16 bytes.
        assert_eq!(image_pipeline.texture_bytes(), 16);

        // Create an offscreen render target.
        let target = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic image test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopySrc,
            ],
        });
        let target_view = backend.create_texture_view(&target, "generic image test target view");

        // Exact renderer lifecycle via generic methods.
        image_pipeline.begin_frame(&backend, 1, SIZE, SIZE, false);

        let instance = crate::pipeline::image_pipeline::ImageInstance {
            position: [0.0, 0.0],
            size: [SIZE as f32, SIZE as f32],
            uv_offset: [0.0, 0.0],
            uv_scale: [1.0, 1.0],
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            alpha: 1.0,
            source_premultiplied: 0.0,
        };

        let mut encoder = backend.create_command_encoder("generic image test encoder");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &crate::backend::RenderPassDescriptor {
                    label: Some("generic image test pass".to_string()),
                    color_attachments: &[crate::backend::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: crate::backend::Operations {
                            load: crate::backend::LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                            store: crate::backend::StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            // Concrete draw_batch is valid because
            // WgpuBackend::RenderPass == wgpu::RenderPass.
            image_pipeline.draw_batch(&backend, &mut pass, tex_id, &[instance]);
        }

        image_pipeline.end_frame(&backend);
        backend.submit(encoder);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "generic image path: center must be red, got {sample:?}"
        );
    }

    /// Exercise ImagePipeline::upload_if_absent returns false when
    /// the texture already exists.
    #[test]
    fn generic_image_upload_if_absent_skips_existing() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device, queue);
        let mut image_pipeline = crate::pipeline::image_pipeline::ImagePipeline::new(
            &backend,
            FORMAT,
            crate::AntiAlias::Analytic,
        );

        let red = [255u8, 0, 0, 255];
        let tex_data = vec![red.to_vec(), red.to_vec(), red.to_vec(), red.to_vec()]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

        // First upload succeeds.
        assert!(image_pipeline.upload_if_absent(&backend, 42, 2, 2, &tex_data));
        assert_eq!(image_pipeline.texture_count(), 1);

        // Second upload with same ID returns false.
        assert!(!image_pipeline.upload_if_absent(&backend, 42, 2, 2, &tex_data));
        assert_eq!(image_pipeline.texture_count(), 1);
    }

    /// Exercise ImagePipeline::create_external_bind_group.
    #[test]
    fn generic_image_external_bind_group_creates() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};

        let backend = WgpuBackend::new(device, queue);
        let image_pipeline = crate::pipeline::image_pipeline::ImagePipeline::new(
            &backend,
            FORMAT,
            crate::AntiAlias::Analytic,
        );

        // Create a placeholder texture and view.
        let tex = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic external-bind-group tex".to_string()),
            size: (2, 2, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![crate::backend::TextureUsage::TextureBinding],
        });
        let view = backend.create_texture_view(&tex, "external view");

        let bg = image_pipeline.create_external_bind_group(&backend, &view);
        // If we got here without panicking, the bind group was created
        // successfully.  Assert the type is a wgpu::BindGroup.
        let _: &wgpu::BindGroup = &bg;
    }

    // ── SvgPipeline ─────────────────────────────────────────────────────

    /// Control: SVG pipeline through the WGPU generic backend adapter.
    #[test]
    fn concrete_svg_path_control_renders_filled_rect() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::wgpu::WgpuBackend;
        use std::sync::Arc;

        let backend = WgpuBackend::new(device.clone(), queue.clone());
        let mut svg_pipeline =
            crate::svg_pipeline::SvgPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        let scene = Arc::new(crate::svg::SvgScene {
            viewport: crate::svg::SvgViewport {
                width: 10.0,
                height: 10.0,
            },
            nodes: Arc::new([crate::svg::SvgNode {
                node_id: crate::svg::SvgNodeId(0),
                svg_id: None,
                classes: Arc::new([]),
                element: crate::svg::SvgElementKind::Path,
                is_definition: false,
                definition_owner: None,
                clip_path: None,
                mask: None,
                filter: None,
                parent: None,
                children: Arc::new([]),
                transform: crate::svg::SvgTransform::default(),
                opacity: 1.0,
                geometry: Some(0),
                fill_rule: crate::svg::SvgFillRule::NonZero,
                fill: Some(crate::svg::SvgFill {
                    color: crate::svg::SvgColor::rgba8(255, 0, 0, 255),
                    rule: crate::svg::SvgFillRule::NonZero,
                }),
                stroke: None,
                fill_paint: None,
                stroke_paint: None,
                paint_order: crate::svg::SvgPaintOrder::FillAndStroke,
                visible: true,
            }]),
            geometries: Arc::new([crate::svg::SvgGeometry {
                commands: Arc::new([
                    crate::svg::SvgPathCommand::MoveTo { x: 0.0, y: 0.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 0.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 10.0, y: 10.0 },
                    crate::svg::SvgPathCommand::LineTo { x: 0.0, y: 10.0 },
                    crate::svg::SvgPathCommand::Close,
                ]),
            }]),
            resources: Arc::new(crate::svg::SvgResourceGraph::empty()),
        });

        let item = crate::renderer::SvgRenderItem {
            scene,
            destination: crate::utilities::Rect {
                x: 0.0,
                y: 0.0,
                width: SIZE as f32,
                height: SIZE as f32,
            },
            overrides: Arc::new([]),
            world_transform: crate::utilities::Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };

        svg_pipeline.prepare(&backend, &[item], SIZE, SIZE, false);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("concrete svg control target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("concrete svg control pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            svg_pipeline.draw_item(&mut pass, 0);
        }
        queue.submit(Some(encoder.finish()));

        let pixels = read_target(&device, &queue, &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "WGPU generic SVG path: center must be red, got {sample:?}"
        );
    }

    /// Exercise parsed gradients and a local rectangular clip through the
    /// WgpuBackend-driven SVG path.
    #[test]
    fn generic_svg_path_renders_gradient_and_clip_path() {
        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        use crate::backend::{GpuBackend, wgpu::WgpuBackend};
        use std::sync::Arc;

        let backend = WgpuBackend::new(device, queue);

        let mut svg_pipeline =
            crate::svg_pipeline::SvgPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        let parsed = crate::svg::parse_svg_document(
            br##"<svg width="10" height="10"><defs>
                <linearGradient id="g" x1="0" y1="0" x2="1" y2="0">
                    <stop offset="0" stop-color="#ff0000"/>
                    <stop offset="1" stop-color="#0000ff"/>
                </linearGradient>
                <clipPath id="clip" clipPathUnits="userSpaceOnUse"><rect width="5" height="10"/></clipPath>
                <pattern id="dots" patternUnits="userSpaceOnUse" width="2" height="2">
                    <rect width="1" height="2" fill="#00ff00"/>
                </pattern>
            </defs><rect width="10" height="10" fill="url(#g)" clip-path="url(#clip)"/><rect y="6" width="10" height="4" fill="url(#dots)"/></svg>"##,
        ).expect("gradient and clip resources should parse");
        let scene = Arc::new(parsed.scene);

        let item = crate::renderer::SvgRenderItem {
            scene,
            destination: crate::utilities::Rect {
                x: 0.0,
                y: 0.0,
                width: SIZE as f32,
                height: SIZE as f32,
            },
            overrides: Arc::new([]),
            world_transform: crate::utilities::Mat3::identity(),
            clip_rect: [0.0, 0.0, -1.0, 0.0],
            clip_border_radius: [0.0; 4],
            opacity: 1.0,
        };

        svg_pipeline.prepare(&backend, &[item], SIZE, SIZE, false);

        let target = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic svg test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopySrc,
            ],
        });
        let target_view = backend.create_texture_view(&target, "generic svg test target view");

        let mut encoder = backend.create_command_encoder("generic svg encoder");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &crate::backend::RenderPassDescriptor {
                    label: Some("generic svg pass".to_string()),
                    color_attachments: &[crate::backend::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: crate::backend::Operations {
                            load: crate::backend::LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                            store: crate::backend::StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            // Concrete draw_item is valid because
            // WgpuBackend::RenderPass == wgpu::RenderPass.
            svg_pipeline.draw_item(&mut pass, 0);
        }
        backend.submit(encoder);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        let inside = pixel(&pixels, SIZE / 4, SIZE / 2);
        let clipped = pixel(&pixels, SIZE * 3 / 4, SIZE / 2);
        let patterned = pixel(&pixels, SIZE / 16, SIZE * 7 / 10);
        let pattern_gap = pixel(&pixels, SIZE * 3 / 16, SIZE * 7 / 10);
        assert!(
            inside[0] > inside[2],
            "left side should retain the red gradient start: {inside:?}"
        );
        assert!(
            inside[2] > 0,
            "the gradient should vary across the painted path: {inside:?}"
        );
        assert_eq!(
            clipped,
            [0, 0, 0, 255],
            "the clip path should suppress the right half: {clipped:?}"
        );
        assert_eq!(
            patterned,
            [0, 255, 0, 255],
            "the pattern tile should paint the first half of its repeat: {patterned:?}"
        );
        assert!(
            pattern_gap[1] < 32 && pattern_gap[0] > 0 && pattern_gap[2] > 0,
            "the pattern gap should preserve the underlying gradient: {pattern_gap:?}"
        );
    }

    // ── MaterialPipeline ─────────────────────────────────────────────

    /// MaterialPipeline (Glass) through the WGPU generic backend adapter.
    /// Verifies that a Glass material with neutral backdrop produces non-clear
    /// pixels (shader runs correctly).
    #[test]
    fn concrete_material_glass_renders_non_clear() {
        use crate::backend::wgpu::WgpuBackend;
        use crate::custom_pipeline::{CustomPipelineGeneric, RenderContextGeneric};
        use crate::pipeline::material::{MaterialKind, MaterialPipeline, MaterialRequest};
        use std::any::Any;

        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let backend = WgpuBackend::new(device.clone(), queue.clone());
        let mut mat_pipeline = MaterialPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        // Push a Glass request via the trait API (requests field is private).
        let request =
            MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, SIZE as f32, SIZE as f32]);
        mat_pipeline.begin_frame();
        mat_pipeline.prepare_command(&request as &(dyn Any + Send));

        let ctx = RenderContextGeneric {
            backend: &backend,
            width: SIZE,
            height: SIZE,
            is_srgb: false,
            format: FORMAT,
            sample_count: 1,
            source_texture: None,
        };
        mat_pipeline.prepare(&ctx);

        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("WGPU generic material control target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&Default::default());

        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("WGPU generic material control pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.0,
                            g: 0.0,
                            b: 0.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            mat_pipeline.render_command(Some(0), &mut pass);
        }
        queue.submit(Some(encoder.finish()));

        let pixels = read_target(&device, &queue, &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_ne!(
            sample,
            [0, 0, 0, 255],
            "WGPU generic material path: center should not be clear, got {sample:?}"
        );
    }

    /// Exercise MaterialPipeline::new + prepare by rendering a
    /// Glass material through the WgpuBackend-driven path.  Uses the concrete
    /// render_command (valid because WgpuBackend::RenderPass == wgpu::RenderPass).
    #[test]
    fn generic_material_glass_renders_non_clear() {
        use crate::backend::{GpuBackend, wgpu::WgpuBackend};
        use crate::custom_pipeline::CustomPipelineGeneric;
        use crate::pipeline::material::{MaterialKind, MaterialPipeline, MaterialRequest};
        use std::any::Any;

        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let backend = WgpuBackend::new(device, queue);

        let mut mat_pipeline = MaterialPipeline::new(&backend, FORMAT, crate::AntiAlias::Analytic);

        // Push a Glass request via the trait API (requests field is private).
        let request =
            MaterialRequest::new(MaterialKind::Glass, [0.0, 0.0, SIZE as f32, SIZE as f32]);
        mat_pipeline.begin_frame();
        mat_pipeline.prepare_command(&request as &(dyn Any + Send));

        let ctx = crate::custom_pipeline::RenderContextGeneric {
            backend: &backend,
            width: SIZE,
            height: SIZE,
            is_srgb: false,
            format: FORMAT,
            sample_count: 1,
            source_texture: None,
        };
        mat_pipeline.prepare(&ctx);

        let target = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("generic material test target".to_string()),
            size: (SIZE, SIZE, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format: FORMAT,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopySrc,
            ],
        });
        let target_view = backend.create_texture_view(&target, "generic material test target view");

        let mut encoder = backend.create_command_encoder("generic material encoder");
        {
            let mut pass = backend.begin_render_pass(
                &mut encoder,
                &crate::backend::RenderPassDescriptor {
                    label: Some("generic material pass".to_string()),
                    color_attachments: &[crate::backend::RenderPassColorAttachment {
                        view: &target_view,
                        resolve_target: None,
                        ops: crate::backend::Operations {
                            load: crate::backend::LoadOp::Clear([0.0, 0.0, 0.0, 1.0]),
                            store: crate::backend::StoreOp::Store,
                        },
                    }],
                    depth_stencil_attachment: None,
                },
            );
            // Concrete render_command is valid because
            // WgpuBackend::RenderPass == wgpu::RenderPass.
            mat_pipeline.render_command(Some(0), &mut pass);
        }
        backend.submit(encoder);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_ne!(
            sample,
            [0, 0, 0, 255],
            "generic material path: center should not be clear, got {sample:?}"
        );
    }

    // ── Renderer / GpuContext public surface ───────────────────────────────

    /// End-to-end: `GpuDevice` → `WgpuBackend` → `Renderer::new` →
    /// `render` a solid red fill-rect → CPU readback. Proves the crate-level
    /// wiring (public exports + renderer generic constructors) is usable and
    /// produces correct pixels through the backend-driven pipeline path.
    #[test]
    fn generic_renderer_with_wgpu_backend_renders_fill_rect() {
        use crate::WgpuBackend;
        use crate::draw_cmd::DrawList;
        use crate::gpu_context::GpuDevice;
        use crate::renderer::RendererImpl;
        use crate::utilities::{Color, Rect};

        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        // Public headless device pair → backend adapter → generic renderer.
        let gpu_device = GpuDevice::new(device, queue);
        let backend: WgpuBackend = gpu_device.backend();
        let mut renderer = RendererImpl::<WgpuBackend>::new(&backend, FORMAT);

        assert_eq!(renderer.surface_format(), FORMAT);

        let mut draw = DrawList::new();
        draw.fill_rect(
            Rect::new(0.0, 0.0, SIZE as f32, SIZE as f32),
            Color::red(),
            [0.0; 4],
            [0.0; 4],
            Color::transparent(),
        );

        let target = backend.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("generic renderer e2e target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());

        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        let sample = pixel(&pixels, SIZE / 2, SIZE / 2);
        assert_eq!(
            sample,
            [255, 0, 0, 255],
            "generic Renderer::new path: center should be opaque red, got {sample:?}"
        );
    }

    /// Exercises image upload/drawing and the generic text preparation path
    /// through the integrated renderer.
    #[test]
    fn generic_renderer_with_wgpu_backend_renders_image_and_text() {
        use crate::WgpuBackend;
        use crate::draw_cmd::DrawList;
        use crate::gpu_context::GpuDevice;
        use crate::renderer::RendererImpl;
        use crate::utilities::{Color, Rect, Vec2d};
        use std::sync::Arc;

        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };
        let backend = GpuDevice::new(device, queue).backend();
        let mut renderer = RendererImpl::<WgpuBackend>::new(&backend, FORMAT);

        let mut draw = DrawList::new();
        let blue = [0, 0, 255, 255];
        let texture_id = draw.load_image(&[blue; 4].concat(), 2, 2);
        draw.draw_image(Rect::new(0.0, 0.0, 24.0, 24.0), texture_id);
        draw.draw_text(
            Vec2d::new(32.0, 24.0),
            Arc::from("A"),
            18.0,
            Color::white(),
            400,
        );

        let target = backend.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("generic renderer image and text target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        renderer.render(&backend, &view, SIZE, SIZE, false, &draw);

        let pixels = read_target(backend.device(), backend.queue(), &target);
        assert_eq!(pixel(&pixels, 12, 12), [0, 0, 255, 255]);
        assert!(
            (32..SIZE).any(|x| {
                (16..48).any(|y| {
                    let [red, green, blue, alpha] = pixel(&pixels, x, y);
                    alpha > 0 && red > 0 && green > 0 && blue > 0
                })
            }),
            "generic text path should draw antialiased white glyph pixels"
        );
    }

    /// Confirms the public re-exports (`WgpuBackend`, `GpuBackend`, `GpuDevice`,
    /// `GpuLimits`) compile and that `GpuDevice::backend` yields a usable
    /// backend whose limits are reachable through the trait surface.
    #[test]
    fn generic_public_exports_gpu_device_backend_roundtrip() {
        use crate::backend::GpuBackend;
        use crate::gpu_context::GpuDevice;
        use crate::{GpuLimits, WgpuBackend};

        let Some((device, queue)) = gpu() else {
            eprintln!("skipping: no GPU adapter available");
            return;
        };

        let gpu_device = GpuDevice::new(device, queue);
        let backend: WgpuBackend = gpu_device.backend();

        // Limits must be reachable through the trait surface and the public
        // `GpuLimits` re-export.
        let limits: GpuLimits = backend.limits();
        assert!(
            limits.max_texture_dimension_2d >= 64,
            "expected a usable max texture dimension, got {}",
            limits.max_texture_dimension_2d
        );

        // GpuDevice still owns live device/queue handles after backend().
        let _ = gpu_device.device();
        let _ = gpu_device.queue();
        let _ = backend.device();
        let _ = backend.queue();
    }
}
