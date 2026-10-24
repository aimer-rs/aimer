use hashbrown::HashMap;

use super::glyph_rasterizer::GlyphKey;

/// Region within the atlas texture for a single glyph.
#[derive(Clone, Copy, Debug)]
pub struct AtlasRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl AtlasRegion {
    /// Returns UV coordinates as (u_min, v_min, u_max, v_max) given atlas
    /// dimensions.
    pub fn uvs(&self, atlas_w: u32, atlas_h: u32) -> [f32; 4] {
        let aw = atlas_w as f32;
        let ah = atlas_h as f32;
        [
            self.x as f32 / aw,
            self.y as f32 / ah,
            (self.x + self.width) as f32 / aw,
            (self.y + self.height) as f32 / ah,
        ]
    }
}

/// Simple shelf/row packer for glyph atlas allocation.
#[derive(Clone)]
struct ShelfPacker {
    width: u32,
    height: u32,
    /// Current x cursor on the active shelf.
    cursor_x: u32,
    /// Y origin of the active shelf.
    shelf_y: u32,
    /// Height of the active shelf (tallest glyph in the row).
    shelf_height: u32,
}

impl ShelfPacker {
    fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            cursor_x: 0,
            shelf_y: 0,
            shelf_height: 0,
        }
    }

    /// Try to allocate a region of `w × h`. Returns `None` if the atlas is
    /// full.
    fn allocate(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if w == 0 || h == 0 {
            return Some((0, 0));
        }

        // Pad by 1 pixel to avoid sampling neighbours.
        let pw = w + 1;
        let ph = h + 1;

        if self.cursor_x + pw > self.width {
            // Move to next shelf.
            self.shelf_y += self.shelf_height;
            self.cursor_x = 0;
            self.shelf_height = 0;
        }

        if self.shelf_y + ph > self.height {
            return None; // Atlas full.
        }

        let x = self.cursor_x;
        let y = self.shelf_y;
        self.cursor_x += pw;
        if ph > self.shelf_height {
            self.shelf_height = ph;
        }
        Some((x, y))
    }

    #[allow(dead_code)]
    fn reset(&mut self) {
        self.cursor_x = 0;
        self.shelf_y = 0;
        self.shelf_height = 0;
    }

    /// Position the packer so the next allocation starts a brand-new, empty
    /// shelf whose top edge is at `y`. Used after the atlas grows: existing
    /// glyphs keep their positions in the (now larger) atlas, and the
    /// packer resumes in the free space directly below them so newly
    /// inserted glyphs can never collide with the preserved content.
    fn start_fresh_shelf_at(&mut self, y: u32) {
        self.cursor_x = 0;
        self.shelf_y = y;
        self.shelf_height = 0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BatchCapacityPlan {
    Keep,
    Reset,
    Reject,
}

fn allocations_fit(mut packer: ShelfPacker, sizes: &[(u32, u32)]) -> bool {
    sizes
        .iter()
        .all(|&(width, height)| packer.allocate(width, height).is_some())
}

fn plan_batch(
    packer: &ShelfPacker,
    max_size: u32,
    missing_sizes: &[(u32, u32)],
    all_sizes: &[(u32, u32)],
) -> BatchCapacityPlan {
    let mut trial = packer.clone();
    for &(width, height) in missing_sizes {
        loop {
            if trial.allocate(width, height).is_some() {
                break;
            }
            if trial.width >= max_size {
                let fresh = ShelfPacker::new(max_size, max_size);
                return if allocations_fit(fresh, all_sizes) {
                    BatchCapacityPlan::Reset
                } else {
                    BatchCapacityPlan::Reject
                };
            }

            let old_height = trial.height;
            let new_width = (trial.width * 2).min(max_size);
            let new_height = (trial.height * 2).min(max_size);
            trial = ShelfPacker::new(new_width, new_height);
            trial.start_fresh_shelf_at(old_height);
        }
    }

    BatchCapacityPlan::Keep
}

/// A glyph that has been packed into the atlas this frame but whose pixels have
/// not yet been written to the GPU texture. Holds only the bytes of the single
/// glyph (dropped right after [`GlyphAtlas::upload`]), so the atlas never keeps
/// a full-size CPU mirror of the texture in RAM.
struct PendingGlyph {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Glyph bitmap rows, tightly packed (`width * height` for R8, ×4 for
    /// RGBA8).
    data: Vec<u8>,
}

/// Metadata for one rectangle copied from the reusable upload buffer.
#[derive(Clone, Copy)]
struct StagedGlyph {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    offset: u64,
    bytes_per_row: u32,
}

#[derive(Clone, Copy)]
struct PackedUploadGroup {
    start: usize,
    end: usize,
    y: u32,
    min_x: u32,
    width: u32,
    height: u32,
}

#[inline]
fn aligned_upload_row_bytes(row_bytes: usize) -> usize {
    (row_bytes + 255) & !255
}

/// Uploads all pending glyphs through one reusable buffer and one command
/// submission. `copy_buffer_to_texture` requires 256-byte row alignment, so
/// each small glyph is copied into an aligned row slice before the GPU copies
/// it to its already-packed atlas rectangle.
#[cfg(feature = "wgpu")]
fn upload_pending(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    pending: &mut Vec<PendingGlyph>,
    staging_buffer: &mut Option<wgpu::Buffer>,
    staging_capacity: &mut usize,
    staging_data: &mut Vec<u8>,
    staging_copies: &mut Vec<StagedGlyph>,
    bytes_per_pixel: usize,
    label: &'static str,
) {
    if pending.is_empty() {
        return;
    }

    // `queue.write_texture` accepts tightly packed rows, while a buffer copy
    // pads every row to 256 bytes. Small glyph rectangles are common in UI
    // text, so forcing each of them through an aligned staging slice can move
    // several times more data than the bitmap contains. Keep the direct path
    // for that shape of batch; reserve the reusable buffer for transfers where
    // its single queue write is likely to amortize the padding.
    let packed_bytes = pending
        .iter()
        .map(|glyph| glyph.width as usize * glyph.height as usize * bytes_per_pixel)
        .sum::<usize>();
    let aligned_bytes = pending
        .iter()
        .map(|glyph| {
            aligned_upload_row_bytes(glyph.width as usize * bytes_per_pixel) * glyph.height as usize
        })
        .sum::<usize>();
    if aligned_bytes > packed_bytes.saturating_mul(4) {
        // The shelf packer allocates monotonically in x and then advances y.
        // Coalesce each consecutive shelf segment into one tightly packed
        // rectangle. The one-pixel gaps are zero-filled, so bilinear sampling
        // still sees the same isolation padding as the individual writes.
        let mut groups: Vec<PackedUploadGroup> = Vec::new();
        for (index, glyph) in pending.iter().enumerate() {
            if glyph.width == 0 || glyph.height == 0 {
                continue;
            }
            let right = glyph.x + glyph.width;
            if let Some(group) = groups.last_mut()
                && group.y == glyph.y
                && group.end == index
            {
                group.end += 1;
                group.width = right - group.min_x;
                group.height = group.height.max(glyph.height);
            } else {
                groups.push(PackedUploadGroup {
                    start: index,
                    end: index + 1,
                    y: glyph.y,
                    min_x: glyph.x,
                    width: glyph.width,
                    height: glyph.height,
                });
            }
        }

        for group in groups {
            let row_bytes = group.width as usize * bytes_per_pixel;
            staging_data.resize(row_bytes * group.height as usize, 0);
            staging_data.fill(0);
            for glyph in &pending[group.start..group.end] {
                if glyph.width == 0 || glyph.height == 0 {
                    continue;
                }
                let source_row_bytes = glyph.width as usize * bytes_per_pixel;
                let x_offset = (glyph.x - group.min_x) as usize * bytes_per_pixel;
                for row in 0..glyph.height as usize {
                    let source_start = row * source_row_bytes;
                    let target_start = row * row_bytes + x_offset;
                    staging_data[target_start..target_start + source_row_bytes].copy_from_slice(
                        &glyph.data[source_start..source_start + source_row_bytes],
                    );
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: group.min_x,
                        y: group.y,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                staging_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes as u32),
                    rows_per_image: Some(group.height),
                },
                wgpu::Extent3d {
                    width: group.width,
                    height: group.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        pending.clear();
        return;
    }

    let pending_batch = std::mem::take(pending);
    staging_data.clear();
    staging_copies.clear();
    staging_copies.reserve(pending_batch.len());

    for glyph in pending_batch.iter() {
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        let row_bytes = glyph.width as usize * bytes_per_pixel;
        let bytes_per_row = aligned_upload_row_bytes(row_bytes);
        let offset = staging_data.len();
        let glyph_bytes = bytes_per_row * glyph.height as usize;
        staging_data.resize(offset + glyph_bytes, 0);
        for row in 0..glyph.height as usize {
            let source_start = row * row_bytes;
            let source_end = source_start + row_bytes;
            let target_start = offset + row * bytes_per_row;
            staging_data[target_start..target_start + row_bytes]
                .copy_from_slice(&glyph.data[source_start..source_end]);
        }
        staging_copies.push(StagedGlyph {
            x: glyph.x,
            y: glyph.y,
            width: glyph.width,
            height: glyph.height,
            offset: offset as u64,
            bytes_per_row: bytes_per_row as u32,
        });
    }

    if staging_data.is_empty() {
        let mut pending_batch = pending_batch;
        pending_batch.clear();
        *pending = pending_batch;
        return;
    }

    if *staging_capacity < staging_data.len() {
        let capacity = staging_data.len().next_power_of_two();
        *staging_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity as u64,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
        *staging_capacity = capacity;
    }

    let buffer = staging_buffer
        .as_ref()
        .expect("staging buffer is allocated for non-empty atlas data");
    queue.write_buffer(buffer, 0, staging_data);

    let mut encoder =
        device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    for copy in staging_copies.iter().copied() {
        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: copy.offset,
                    bytes_per_row: Some(copy.bytes_per_row),
                    rows_per_image: Some(copy.height),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: copy.x,
                    y: copy.y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: copy.width,
                height: copy.height,
                depth_or_array_layers: 1,
            },
        );
    }
    queue.submit(Some(encoder.finish()));

    let mut pending_batch = pending_batch;
    pending_batch.clear();
    *pending = pending_batch;
}

pub struct GlyphAtlas<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pub texture: B::Texture,
    pub view: B::TextureView,
    pub width: u32,
    pub height: u32,
    packer: ShelfPacker,
    cache: HashMap<GlyphKey, AtlasRegion>,
    pending: Vec<PendingGlyph>,
    staging_buffer: Option<B::Buffer>,
    staging_capacity: usize,
    staging_data: Vec<u8>,
    staging_copies: Vec<StagedGlyph>,
    generation: u64,
}

// ---------------------------------------------------------------------------
// Color glyph atlas (RGBA8, for sbix PNG strikes)
// ---------------------------------------------------------------------------

/// Sibling to [`GlyphAtlas`] that stores RGBA8 color glyphs (Apple Color
/// Emoji, etc.). The shape and behavior are intentionally near-identical: a
/// shelf packer, lazy re-upload of a dirty rectangle, and 2× growth on
/// overflow. Only the per-pixel size and texture format differ.
pub struct ColorGlyphAtlas<B: crate::backend::GpuBackend = crate::backend::DefaultGpuBackend> {
    pub texture: B::Texture,
    pub view: B::TextureView,
    pub width: u32,
    pub height: u32,
    packer: ShelfPacker,
    cache: HashMap<GlyphKey, AtlasRegion>,
    pending: Vec<PendingGlyph>,
    staging_buffer: Option<B::Buffer>,
    staging_capacity: usize,
    staging_data: Vec<u8>,
    staging_copies: Vec<StagedGlyph>,
    generation: u64,
}

// ── Backend-generic atlas resources and uploads ────────────────────────────

/// Backend-driven equivalent of [`upload_pending`]. Shared by
/// [`GlyphAtlas::upload`] and [`ColorGlyphAtlas::upload`].
fn upload_pending_generic<B: crate::backend::GpuBackend>(
    backend: &B,
    texture: &B::Texture,
    pending: &mut Vec<PendingGlyph>,
    staging_buffer: &mut Option<B::Buffer>,
    staging_capacity: &mut usize,
    staging_data: &mut Vec<u8>,
    staging_copies: &mut Vec<StagedGlyph>,
    bytes_per_pixel: usize,
    label: &'static str,
) {
    use crate::backend::{
        BufferDescriptor, BufferUsage, Extent3d, Origin3d, TexelCopyBufferInfo,
        TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect, WriteTextureDescriptor,
    };

    if pending.is_empty() {
        return;
    }

    let packed_bytes = pending
        .iter()
        .map(|glyph| glyph.width as usize * glyph.height as usize * bytes_per_pixel)
        .sum::<usize>();
    let aligned_bytes = pending
        .iter()
        .map(|glyph| {
            aligned_upload_row_bytes(glyph.width as usize * bytes_per_pixel) * glyph.height as usize
        })
        .sum::<usize>();
    if aligned_bytes > packed_bytes.saturating_mul(4) {
        let mut groups: Vec<PackedUploadGroup> = Vec::new();
        for (index, glyph) in pending.iter().enumerate() {
            if glyph.width == 0 || glyph.height == 0 {
                continue;
            }
            let right = glyph.x + glyph.width;
            if let Some(group) = groups.last_mut()
                && group.y == glyph.y
                && group.end == index
            {
                group.end += 1;
                group.width = right - group.min_x;
                group.height = group.height.max(glyph.height);
            } else {
                groups.push(PackedUploadGroup {
                    start: index,
                    end: index + 1,
                    y: glyph.y,
                    min_x: glyph.x,
                    width: glyph.width,
                    height: glyph.height,
                });
            }
        }

        for group in groups {
            let row_bytes = group.width as usize * bytes_per_pixel;
            staging_data.resize(row_bytes * group.height as usize, 0);
            staging_data.fill(0);
            for glyph in &pending[group.start..group.end] {
                if glyph.width == 0 || glyph.height == 0 {
                    continue;
                }
                let source_row_bytes = glyph.width as usize * bytes_per_pixel;
                let x_offset = (glyph.x - group.min_x) as usize * bytes_per_pixel;
                for row in 0..glyph.height as usize {
                    let source_start = row * source_row_bytes;
                    let target_start = row * row_bytes + x_offset;
                    staging_data[target_start..target_start + source_row_bytes].copy_from_slice(
                        &glyph.data[source_start..source_start + source_row_bytes],
                    );
                }
            }
            backend.write_texture(&WriteTextureDescriptor {
                texture,
                mip_level: 0,
                origin: Origin3d {
                    x: group.min_x,
                    y: group.y,
                    z: 0,
                },
                aspect: TextureAspect::All,
                data: staging_data,
                buffer_layout: TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row_bytes as u32),
                    rows_per_image: Some(group.height),
                },
                extent: Extent3d {
                    width: group.width,
                    height: group.height,
                    depth_or_array_layers: 1,
                },
            });
        }
        pending.clear();
        return;
    }

    let pending_batch = std::mem::take(pending);
    staging_data.clear();
    staging_copies.clear();
    staging_copies.reserve(pending_batch.len());

    for glyph in pending_batch.iter() {
        if glyph.width == 0 || glyph.height == 0 {
            continue;
        }
        let row_bytes = glyph.width as usize * bytes_per_pixel;
        let bytes_per_row = aligned_upload_row_bytes(row_bytes);
        let offset = staging_data.len();
        let glyph_bytes = bytes_per_row * glyph.height as usize;
        staging_data.resize(offset + glyph_bytes, 0);
        for row in 0..glyph.height as usize {
            let source_start = row * row_bytes;
            let source_end = source_start + row_bytes;
            let target_start = offset + row * bytes_per_row;
            staging_data[target_start..target_start + row_bytes]
                .copy_from_slice(&glyph.data[source_start..source_end]);
        }
        staging_copies.push(StagedGlyph {
            x: glyph.x,
            y: glyph.y,
            width: glyph.width,
            height: glyph.height,
            offset: offset as u64,
            bytes_per_row: bytes_per_row as u32,
        });
    }

    if staging_data.is_empty() {
        let mut pending_batch = pending_batch;
        pending_batch.clear();
        *pending = pending_batch;
        return;
    }

    if *staging_capacity < staging_data.len() {
        let capacity = staging_data.len().next_power_of_two();
        *staging_buffer = Some(backend.create_buffer(&BufferDescriptor {
            label: Some(label.to_string()),
            size: capacity as u64,
            usage: vec![BufferUsage::CopySrc, BufferUsage::CopyDst],
        }));
        *staging_capacity = capacity;
    }

    let buffer = staging_buffer
        .as_ref()
        .expect("staging buffer is allocated for non-empty atlas data");
    backend.write_buffer(buffer, 0, staging_data);

    let mut encoder = backend.create_command_encoder(label);
    for copy in staging_copies.iter().copied() {
        backend.copy_buffer_to_texture(
            &mut encoder,
            &TexelCopyBufferInfo {
                buffer,
                layout: TexelCopyBufferLayout {
                    offset: copy.offset,
                    bytes_per_row: Some(copy.bytes_per_row),
                    rows_per_image: Some(copy.height),
                },
            },
            &TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: Origin3d {
                    x: copy.x,
                    y: copy.y,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: copy.width,
                height: copy.height,
                depth_or_array_layers: 1,
            },
        );
    }
    backend.submit(encoder);

    let mut pending_batch = pending_batch;
    pending_batch.clear();
    *pending = pending_batch;
}

impl<B: crate::backend::GpuBackend> GlyphAtlas<B> {
    const INITIAL_SIZE: u32 = 512;
    const MAX_SIZE: u32 = 2048;

    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn memory_bytes(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    /// Looks up a cached glyph region on a backend-generic atlas.
    #[inline]
    pub fn get(&self, key: &GlyphKey) -> Option<AtlasRegion> {
        self.cache.get(key).copied()
    }

    pub(super) fn plan_batch_generic(&self, glyphs: &[(GlyphKey, u32, u32)]) -> BatchCapacityPlan {
        let missing = glyphs
            .iter()
            .filter(|(key, _, _)| !self.cache.contains_key(key))
            .map(|(_, width, height)| (*width, *height))
            .collect::<Vec<_>>();
        let all = glyphs
            .iter()
            .map(|(_, width, height)| (*width, *height))
            .collect::<Vec<_>>();
        plan_batch(&self.packer, Self::MAX_SIZE, &missing, &all)
    }

    pub(super) fn apply_batch_plan_generic(&mut self, plan: BatchCapacityPlan) {
        if plan == BatchCapacityPlan::Reset {
            self.cache.clear();
            self.pending.clear();
            self.packer = ShelfPacker::new(self.width, self.height);
            self.generation += 1;
        }
    }

    /// Creates the atlas through the selected [`GpuBackend`].
    pub fn new(backend: &B) -> Self {
        let width = Self::INITIAL_SIZE;
        let height = Self::INITIAL_SIZE;
        let (texture, view) = Self::create_texture_generic(backend, width, height);
        Self {
            texture,
            view,
            width,
            height,
            packer: ShelfPacker::new(width, height),
            cache: HashMap::new(),
            pending: Vec::new(),
            staging_buffer: None,
            staging_capacity: 0,
            staging_data: Vec::new(),
            staging_copies: Vec::new(),
            generation: 0,
        }
    }

    fn create_texture_generic(
        backend: &B,
        width: u32,
        height: u32,
    ) -> (B::Texture, B::TextureView) {
        use crate::backend::{TextureDescriptor, TextureDimension, TextureUsage};
        let texture = backend.create_texture(&TextureDescriptor {
            label: Some("glyph atlas".to_string()),
            size: (width, height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: B::r8_unorm_format(),
            usage: vec![
                TextureUsage::TextureBinding,
                TextureUsage::CopyDst,
                TextureUsage::CopySrc,
            ],
        });
        let view = backend.create_texture_view(&texture, "glyph atlas view");
        (texture, view)
    }

    /// Backend-driven equivalent of [`GlyphAtlas::get_or_insert`].
    pub fn get_or_insert(
        &mut self,
        backend: &B,
        key: GlyphKey,
        glyph_w: u32,
        glyph_h: u32,
        bitmap: &[u8],
    ) -> AtlasRegion {
        if let Some(region) = self.cache.get(&key) {
            return *region;
        }

        let pos = self.packer.allocate(glyph_w, glyph_h);
        let (x, y) = match pos {
            Some(p) => p,
            None => {
                self.grow_generic(backend);
                self.packer
                    .allocate(glyph_w, glyph_h)
                    .expect("glyph too large for atlas even after grow")
            }
        };

        self.pending.push(PendingGlyph {
            x,
            y,
            width: glyph_w,
            height: glyph_h,
            data: bitmap.to_vec(),
        });

        let region = AtlasRegion {
            x,
            y,
            width: glyph_w,
            height: glyph_h,
        };
        self.cache.insert(key, region);
        region
    }

    /// Backend-driven equivalent of [`GlyphAtlas::upload`].
    pub fn upload(&mut self, backend: &B) {
        upload_pending_generic(
            backend,
            &self.texture,
            &mut self.pending,
            &mut self.staging_buffer,
            &mut self.staging_capacity,
            &mut self.staging_data,
            &mut self.staging_copies,
            1,
            "glyph atlas upload",
        );
    }

    /// Backend-driven equivalent of [`GlyphAtlas::grow`].
    fn grow_generic(&mut self, backend: &B) {
        use crate::backend::{Extent3d, Origin3d, TexelCopyTextureInfo, TextureAspect};

        if self.width >= Self::MAX_SIZE {
            self.cache.clear();
            self.pending.clear();
            self.packer = ShelfPacker::new(self.width, self.height);
            self.generation += 1;
            return;
        }

        let old_w = self.width;
        let old_h = self.height;
        let new_w = self.width * 2;
        let new_h = self.height * 2;
        let (texture, view) = Self::create_texture_generic(backend, new_w, new_h);

        let mut encoder = backend.create_command_encoder("glyph atlas grow");
        backend.copy_texture_to_texture(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: old_w,
                height: old_h,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        self.texture = texture;
        self.view = view;
        self.packer = ShelfPacker::new(new_w, new_h);
        self.packer.start_fresh_shelf_at(old_h);
        self.width = new_w;
        self.height = new_h;
        self.generation += 1;
    }
}

impl<B: crate::backend::GpuBackend> ColorGlyphAtlas<B> {
    const INITIAL_SIZE: u32 = 512;
    const BYTES_PER_PIXEL: u32 = 4;
    const MAX_SIZE: u32 = 2048;

    #[inline]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn memory_bytes(&self) -> u64 {
        self.width as u64 * self.height as u64 * Self::BYTES_PER_PIXEL as u64
    }

    /// Looks up a cached glyph region on a backend-generic color atlas.
    #[inline]
    pub fn get(&self, key: &GlyphKey) -> Option<AtlasRegion> {
        self.cache.get(key).copied()
    }

    pub(super) fn plan_batch_generic(&self, glyphs: &[(GlyphKey, u32, u32)]) -> BatchCapacityPlan {
        let missing = glyphs
            .iter()
            .filter(|(key, _, _)| !self.cache.contains_key(key))
            .map(|(_, width, height)| (*width, *height))
            .collect::<Vec<_>>();
        let all = glyphs
            .iter()
            .map(|(_, width, height)| (*width, *height))
            .collect::<Vec<_>>();
        plan_batch(&self.packer, Self::MAX_SIZE, &missing, &all)
    }

    pub(super) fn apply_batch_plan_generic(&mut self, plan: BatchCapacityPlan) {
        if plan == BatchCapacityPlan::Reset {
            self.cache.clear();
            self.pending.clear();
            self.packer = ShelfPacker::new(self.width, self.height);
            self.generation += 1;
        }
    }

    /// Creates the atlas through the selected [`GpuBackend`].
    pub fn new(backend: &B) -> Self {
        let width = Self::INITIAL_SIZE;
        let height = Self::INITIAL_SIZE;
        let (texture, view) = Self::create_texture_generic(backend, width, height);
        Self {
            texture,
            view,
            width,
            height,
            packer: ShelfPacker::new(width, height),
            cache: HashMap::new(),
            pending: Vec::new(),
            staging_buffer: None,
            staging_capacity: 0,
            staging_data: Vec::new(),
            staging_copies: Vec::new(),
            generation: 0,
        }
    }

    fn create_texture_generic(
        backend: &B,
        width: u32,
        height: u32,
    ) -> (B::Texture, B::TextureView) {
        use crate::backend::{TextureDescriptor, TextureDimension, TextureUsage};
        let texture = backend.create_texture(&TextureDescriptor {
            label: Some("color glyph atlas".to_string()),
            size: (width, height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: B::rgba8_unorm_format(),
            usage: vec![
                TextureUsage::TextureBinding,
                TextureUsage::CopyDst,
                TextureUsage::CopySrc,
            ],
        });
        let view = backend.create_texture_view(&texture, "color glyph atlas view");
        (texture, view)
    }

    /// Backend-driven equivalent of [`ColorGlyphAtlas::get_or_insert`].
    pub fn get_or_insert(
        &mut self,
        backend: &B,
        key: GlyphKey,
        glyph_w: u32,
        glyph_h: u32,
        bitmap: &[u8],
    ) -> AtlasRegion {
        if let Some(region) = self.cache.get(&key) {
            return *region;
        }

        let pos = self.packer.allocate(glyph_w, glyph_h);
        let (x, y) = match pos {
            Some(p) => p,
            None => {
                self.grow_generic(backend);
                self.packer
                    .allocate(glyph_w, glyph_h)
                    .expect("color glyph too large for atlas even after grow")
            }
        };

        self.pending.push(PendingGlyph {
            x,
            y,
            width: glyph_w,
            height: glyph_h,
            data: bitmap.to_vec(),
        });

        let region = AtlasRegion {
            x,
            y,
            width: glyph_w,
            height: glyph_h,
        };
        self.cache.insert(key, region);
        region
    }

    /// Backend-driven equivalent of [`ColorGlyphAtlas::upload`].
    pub fn upload(&mut self, backend: &B) {
        upload_pending_generic(
            backend,
            &self.texture,
            &mut self.pending,
            &mut self.staging_buffer,
            &mut self.staging_capacity,
            &mut self.staging_data,
            &mut self.staging_copies,
            Self::BYTES_PER_PIXEL as usize,
            "color glyph atlas upload",
        );
    }

    /// Backend-driven equivalent of [`ColorGlyphAtlas::grow`].
    fn grow_generic(&mut self, backend: &B) {
        use crate::backend::{Extent3d, Origin3d, TexelCopyTextureInfo, TextureAspect};

        if self.width >= Self::MAX_SIZE {
            self.cache.clear();
            self.pending.clear();
            self.packer = ShelfPacker::new(self.width, self.height);
            self.generation += 1;
            return;
        }

        let old_w = self.width;
        let old_h = self.height;
        let new_w = self.width * 2;
        let new_h = self.height * 2;
        let (texture, view) = Self::create_texture_generic(backend, new_w, new_h);

        let mut encoder = backend.create_command_encoder("color glyph atlas grow");
        backend.copy_texture_to_texture(
            &mut encoder,
            &TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            &TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: Origin3d::ZERO,
                aspect: TextureAspect::All,
            },
            Extent3d {
                width: old_w,
                height: old_h,
                depth_or_array_layers: 1,
            },
        );
        backend.submit(encoder);

        self.texture = texture;
        self.view = view;
        self.packer = ShelfPacker::new(new_w, new_h);
        self.packer.start_fresh_shelf_at(old_h);
        self.width = new_w;
        self.height = new_h;
        self.generation += 1;
    }
}

#[cfg(all(test, feature = "wgpu"))]
mod tests {
    use super::*;

    fn overlaps(a: &AtlasRegion, b: &AtlasRegion) -> bool {
        let ax2 = a.x + a.width;
        let ay2 = a.y + a.height;
        let bx2 = b.x + b.width;
        let by2 = b.y + b.height;
        a.x < bx2 && b.x < ax2 && a.y < by2 && b.y < ay2
    }

    /// A representative spread of glyph sizes that produces several shelves on
    /// a 512-wide atlas, mirroring what happens when many glyphs are
    /// rasterized.
    fn sample_glyphs() -> Vec<(u32, u32)> {
        let mut v = Vec::new();
        for i in 0..120u32 {
            let w = 8 + (i * 7) % 40;
            let h = 10 + (i * 5) % 30;
            v.push((w, h));
        }
        v
    }

    /// Pack a representative set of glyphs into a `size`×`size` packer and
    /// return the resulting regions (in allocation order, which is what
    /// `grow()` preserves) plus the live packer.
    fn pack_initial(size: u32) -> (ShelfPacker, Vec<AtlasRegion>) {
        let mut packer = ShelfPacker::new(size, size);
        let mut regions: Vec<AtlasRegion> = Vec::new();
        for &(w, h) in &sample_glyphs() {
            if let Some((x, y)) = packer.allocate(w, h) {
                regions.push(AtlasRegion {
                    x,
                    y,
                    width: w,
                    height: h,
                });
            }
        }
        (packer, regions)
    }

    /// The previous `grow()` strategy: reset the packer to the *doubled* size
    /// and replay the old allocations. Because the width changed, the
    /// packer wraps rows differently than the preserved layout, so this is
    /// unsafe.
    fn old_grow_next_allocation(
        regions: &[AtlasRegion],
        new_w: u32,
        new_h: u32,
        next: (u32, u32),
    ) -> (u32, u32) {
        let mut packer = ShelfPacker::new(new_w, new_h);
        let mut sorted = regions.to_vec();
        sorted.sort_by_key(|r| (r.y, r.x));
        for r in &sorted {
            let _ = packer.allocate(r.width, r.height);
        }
        packer.allocate(next.0, next.1).unwrap()
    }

    /// The new `grow()` strategy: keep the packer at the doubled size but
    /// resume on a fresh shelf directly below the preserved content
    /// (`start_fresh_shelf_at`).
    fn new_grow_next_allocation(
        old_h: u32,
        new_w: u32,
        new_h: u32,
        next: (u32, u32),
    ) -> (u32, u32) {
        let mut packer = ShelfPacker::new(new_w, new_h);
        packer.start_fresh_shelf_at(old_h);
        packer.allocate(next.0, next.1).unwrap()
    }

    #[test]
    fn replaying_allocations_after_a_width_changing_grow_overlaps_existing_glyphs() {
        // Regression guard: prove the old approach is genuinely broken so the
        // positive test below is not vacuous. Replaying allocations into a
        // doubled-width packer hands out a position that collides with preserved
        // glyphs.
        let (_packer, regions) = pack_initial(512);
        let next = (40, 30);
        let pos = old_grow_next_allocation(&regions, 1024, 1024, next);
        let new_region = AtlasRegion {
            x: pos.0,
            y: pos.1,
            width: next.0,
            height: next.1,
        };
        let overlap = regions.iter().any(|r| overlaps(r, &new_region));
        assert!(
            overlap,
            "expected replay-after-grow to overlap existing glyphs; got {:?}",
            new_region
        );
    }

    #[test]
    fn fresh_shelf_after_grow_never_overlaps_existing_glyphs() {
        // The fix: after growing, all preserved glyphs live within y < old_height,
        // and the packer resumes at y == old_height, so any newly allocated glyph
        // is strictly below the preserved content and cannot overlap it.
        let (_packer, regions) = pack_initial(512);
        let old_h = 512;
        for &next in &[(40u32, 30u32), (1u32, 1u32), (300u32, 200u32)] {
            let pos = new_grow_next_allocation(old_h, 1024, 1024, next);
            let new_region = AtlasRegion {
                x: pos.0,
                y: pos.1,
                width: next.0,
                height: next.1,
            };
            assert!(
                new_region.y >= old_h,
                "new glyph must start below preserved content: {:?}",
                new_region
            );
            for r in &regions {
                assert!(
                    !overlaps(r, &new_region),
                    "new glyph overlapped a preserved glyph: {:?} vs {:?}",
                    r,
                    new_region
                );
            }
        }
    }

    #[test]
    fn max_size_batch_requests_reset_before_any_frame_allocation() {
        let mut packer = ShelfPacker::new(8, 8);
        assert!(packer.allocate(7, 3).is_some());
        assert!(packer.allocate(7, 3).is_some());

        let plan = plan_batch(&packer, 8, &[(2, 2)], &[(2, 2), (2, 2)]);

        assert_eq!(plan, BatchCapacityPlan::Reset);
    }

    #[test]
    fn max_size_batch_rejects_a_frame_that_cannot_fit_after_reset() {
        let packer = ShelfPacker::new(8, 8);

        let plan = plan_batch(&packer, 8, &[(7, 7), (7, 7)], &[(7, 7), (7, 7)]);

        assert_eq!(plan, BatchCapacityPlan::Reject);
    }
}
