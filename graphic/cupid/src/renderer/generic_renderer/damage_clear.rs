use super::Renderer;
use crate::backend::{
    GpuBackend, LoadOp, Operations, RenderPassColorAttachment, RenderPassDescriptor, StoreOp,
};
use crate::damage_region::DamageRect;

pub(super) struct DamageClearTextureGeneric<B: GpuBackend> {
    texture: B::Texture,
    width: u32,
    height: u32,
    format: B::TextureFormat,
}

impl<B: GpuBackend> Renderer<B> {
    pub(super) fn clear_render_target(
        &self,
        backend: &B,
        view: &B::TextureView,
        resolve_target: Option<&B::TextureView>,
        color: [f64; 4],
    ) {
        let mut encoder = backend.create_command_encoder("cupid clear render target");
        let attachments = [RenderPassColorAttachment {
            view,
            resolve_target,
            ops: Operations {
                load: LoadOp::Clear(color),
                store: StoreOp::Store,
            },
        }];
        backend.begin_render_pass(
            &mut encoder,
            &RenderPassDescriptor {
                label: Some("cupid clear render target pass".to_string()),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            },
        );
        backend.submit(encoder);
    }

    pub(super) fn ensure_damage_clear_texture(
        &mut self,
        backend: &B,
        width: u32,
        height: u32,
    ) -> B::Texture {
        let recreate = self.damage_clear_texture.as_ref().is_none_or(|target| {
            target.width != width || target.height != height || target.format != self.format
        });
        if recreate {
            let byte_len = u64::from(width)
                .checked_mul(u64::from(height))
                .and_then(|pixels| pixels.checked_mul(4))
                .and_then(|bytes| usize::try_from(bytes).ok())
                .expect("damage clear texture size fits address space");
            let bytes_per_row = width
                .checked_mul(4)
                .expect("damage clear texture row size fits u32");
            let texture = backend.create_texture(&crate::backend::TextureDescriptor {
                label: Some("cupid damage clear texture".to_string()),
                size: (width, height, 1),
                mip_level_count: 1,
                sample_count: 1,
                dimension: crate::backend::TextureDimension::D2,
                format: self.format,
                usage: vec![
                    crate::backend::TextureUsage::CopySrc,
                    crate::backend::TextureUsage::CopyDst,
                ],
            });
            let zeros = vec![0; byte_len];
            backend.write_texture(&crate::backend::WriteTextureDescriptor {
                texture: &texture,
                mip_level: 0,
                origin: crate::backend::Origin3d { x: 0, y: 0, z: 0 },
                aspect: crate::backend::TextureAspect::All,
                data: &zeros,
                buffer_layout: crate::backend::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
                extent: crate::backend::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            });
            self.damage_clear_texture = Some(DamageClearTextureGeneric {
                texture,
                width,
                height,
                format: self.format,
            });
        }
        self.damage_clear_texture
            .as_ref()
            .expect("damage clear texture initialized")
            .texture
            .clone()
    }

    pub(super) fn clear_damage_regions(
        &self,
        backend: &B,
        clear_texture: &B::Texture,
        target_texture: &B::Texture,
        regions: &[DamageRect],
    ) {
        let mut encoder = backend.create_command_encoder("cupid clear damage regions");
        for region in regions {
            if region.width == 0 || region.height == 0 {
                continue;
            }
            let origin = crate::backend::Origin3d {
                x: region.x,
                y: region.y,
                z: 0,
            };
            backend.copy_texture_to_texture(
                &mut encoder,
                &crate::backend::TexelCopyTextureInfo {
                    texture: clear_texture,
                    mip_level: 0,
                    origin,
                    aspect: crate::backend::TextureAspect::All,
                },
                &crate::backend::TexelCopyTextureInfo {
                    texture: target_texture,
                    mip_level: 0,
                    origin,
                    aspect: crate::backend::TextureAspect::All,
                },
                crate::backend::Extent3d {
                    width: region.width,
                    height: region.height,
                    depth_or_array_layers: 1,
                },
            );
        }
        backend.submit(encoder);
    }
}
