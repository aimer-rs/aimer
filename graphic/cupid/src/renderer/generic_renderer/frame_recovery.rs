use super::{DrawCommand, GpuBackend, Renderer};
use crate::frame::FramePacket;

impl<B: GpuBackend> Renderer<B> {
    /// Preserves resource operations from a frame that could not acquire a target.
    ///
    /// Call in frame order, on the renderer's owning thread, before processing
    /// another packet. No drawing is performed. Retained pixels become invalid
    /// so an already queued partial-damage frame cannot reuse stale output.
    #[doc(hidden)]
    pub fn discard_frame(&mut self, backend: &B, packet: &FramePacket) {
        for command in packet.frame().draw_list.commands() {
            match command {
                DrawCommand::LoadImage {
                    bytes,
                    texture_id,
                    width,
                    height,
                } => {
                    self.image_pipeline
                        .as_mut()
                        .expect("image uploads require the image pipeline")
                        .upload_if_absent(backend, *texture_id, *width, *height, bytes);
                }
                DrawCommand::LoadImageWithId {
                    bytes,
                    texture_id,
                    width,
                    height,
                } => {
                    self.image_pipeline
                        .as_mut()
                        .expect("image uploads require the image pipeline")
                        .upload_image_with_id(backend, *texture_id, *width, *height, bytes);
                }
                DrawCommand::RemoveTexture { texture_id } => {
                    if let Some(images) = self.image_pipeline.as_mut() {
                        images.remove_texture(*texture_id);
                    }
                }
                _ => {}
            }
        }
        self.scene_target.invalidate();
        self.material_target.invalidate();
        self.scene_history_key = None;
        for layer in self.retained_layers.values_mut() {
            layer.target.invalidate();
        }
        for group in &mut self.opacity_group_targets {
            group.target.invalidate();
        }
        self.group_seed_target.invalidate();
    }
}
