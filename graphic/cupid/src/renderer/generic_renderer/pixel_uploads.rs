//! Pixels in system memory, uploaded once per picture.
//!
//! A software video decoder hands the renderer RGBA pixels rather than a GPU
//! texture (see `NativeTexture::Rgba8`). Each producer gets a few textures here,
//! used in turn: a picture is written into the one that has gone longest
//! without being drawn, never into one the GPU may still be sampling, and a
//! picture drawn again (a render with no new frame) is not written again.

use std::collections::HashMap;

use crate::backend::{
    Extent3d, GpuBackend, Origin3d, TexelCopyBufferLayout, TextureAspect, TextureDescriptor,
    TextureDimension, TextureUsage, WriteTextureDescriptor,
};

/// Textures per producer. A presenting surface has at most three renders in
/// flight, so the one written next was last drawn at least this many renders
/// ago: the same margin the frames themselves are kept alive for.
const RING: usize = 4;
/// Renders after which a producer nobody has drawn is forgotten.
const FORGET_AFTER: u64 = 600;

/// What to do with a picture, given what the ring holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Plan {
    /// Slot `0` already holds this picture.
    Reuse(usize),
    /// Write the picture into slot `0`, the one drawn longest ago (or never).
    Write(usize),
}

/// Plans a picture of `serial` against slots described by (serial held, render
/// last drawn), `None` for a slot that holds nothing yet.
pub(super) fn plan(slots: &[Option<(u64, u64)>], serial: u64) -> Plan {
    if let Some(index) = slots.iter().position(|slot| slot.is_some_and(|(held, _)| held == serial)) {
        return Plan::Reuse(index);
    }
    let oldest = slots
        .iter()
        .enumerate()
        .min_by_key(|(_, slot)| slot.map_or(0, |(_, used)| used + 1))
        .map_or(0, |(index, _)| index);
    Plan::Write(oldest)
}

struct Slot<B: GpuBackend> {
    texture: B::Texture,
    serial: u64,
    used: u64,
}

struct Producer<B: GpuBackend> {
    size: (u32, u32),
    slots: Vec<Option<Slot<B>>>,
    last_used: u64,
}

/// The textures of every producer of pixel frames.
pub(super) struct PixelUploads<B: GpuBackend> {
    producers: HashMap<u32, Producer<B>>,
    render: u64,
}

impl<B: GpuBackend> PixelUploads<B> {
    pub(super) fn new() -> Self {
        Self {
            producers: HashMap::new(),
            render: 0,
        }
    }

    /// Starts a render, forgetting producers nobody has drawn for a while.
    pub(super) fn begin_render(&mut self, render: u64) {
        self.render = render;
        let oldest = render.saturating_sub(FORGET_AFTER);
        self.producers.retain(|_, producer| producer.last_used >= oldest);
    }

    /// A view of `pixels` (`width * height * 4` bytes) as picture `serial` of
    /// `source`, writing them first when no texture holds that picture.
    pub(super) fn view(
        &mut self,
        backend: &B,
        source: u32,
        serial: u64,
        size: (u32, u32),
        pixels: &[u8],
    ) -> Option<B::TextureView> {
        let (width, height) = size;
        if width == 0 || height == 0 || pixels.len() < width as usize * height as usize * 4 {
            return None;
        }
        let render = self.render;
        let producer = self.producers.entry(source).or_insert_with(|| Producer {
            size,
            slots: (0..RING).map(|_| None).collect(),
            last_used: render,
        });
        if producer.size != size {
            // A new picture size: nothing in the ring fits any more.
            producer.size = size;
            producer.slots.iter_mut().for_each(|slot| *slot = None);
        }
        producer.last_used = render;
        let held: Vec<Option<(u64, u64)>> = producer
            .slots
            .iter()
            .map(|slot| slot.as_ref().map(|slot| (slot.serial, slot.used)))
            .collect();
        let index = match plan(&held, serial) {
            Plan::Reuse(index) => index,
            Plan::Write(index) => {
                let slot = producer.slots[index].get_or_insert_with(|| Slot {
                    texture: backend.create_texture(&TextureDescriptor {
                        label: Some("video pixels".into()),
                        size: (width, height, 1),
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: TextureDimension::D2,
                        format: B::rgba8_unorm_format(),
                        usage: vec![TextureUsage::TextureBinding, TextureUsage::CopyDst],
                    }),
                    serial,
                    used: render,
                });
                backend.write_texture(&WriteTextureDescriptor::<B> {
                    texture: &slot.texture,
                    mip_level: 0,
                    origin: Origin3d::ZERO,
                    aspect: TextureAspect::All,
                    data: &pixels[..width as usize * height as usize * 4],
                    buffer_layout: TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(width * 4),
                        rows_per_image: Some(height),
                    },
                    extent: Extent3d {
                        width,
                        height,
                        depth_or_array_layers: 1,
                    },
                });
                slot.serial = serial;
                index
            }
        };
        let slot = producer.slots[index].as_mut()?;
        slot.used = render;
        Some(backend.create_texture_view(&slot.texture, "video pixels"))
    }

    /// Producers held; for tests.
    #[cfg(test)]
    pub(super) fn producers(&self) -> usize {
        self.producers.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_already_in_the_ring_is_not_written_again() {
        let slots = [Some((7, 3)), Some((8, 4)), None, None];
        assert_eq!(plan(&slots, 8), Plan::Reuse(1));
    }

    #[test]
    fn a_new_picture_fills_an_empty_slot_before_replacing_a_used_one() {
        let slots = [Some((7, 3)), None, Some((9, 1)), Some((10, 2))];
        assert_eq!(plan(&slots, 11), Plan::Write(1));
    }

    #[test]
    fn a_full_ring_replaces_the_slot_drawn_longest_ago() {
        let slots = [Some((7, 5)), Some((8, 2)), Some((9, 9)), Some((10, 4))];
        assert_eq!(plan(&slots, 11), Plan::Write(1));
    }

    #[test]
    fn the_slot_just_drawn_is_the_last_to_be_replaced() {
        let mut slots = [Some((1, 10)), Some((2, 10)), Some((3, 10)), Some((4, 11))];
        // Three slots tie; the one drawn this render is never the pick.
        let Plan::Write(index) = plan(&slots, 5) else { panic!("a new picture is written") };
        assert_ne!(index, 3);
        slots[index] = Some((5, 12));
        let Plan::Write(next) = plan(&slots, 6) else { panic!("a new picture is written") };
        assert_ne!(next, index);
    }
}
