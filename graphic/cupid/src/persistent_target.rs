use crate::backend::GpuBackend;

/// Whether the pixels currently stored in a persistent target may be reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetValidity {
    /// The target has not been initialized or its contents are unknown.
    Unknown,
    /// The target resource exists, but a full repaint is required.
    Invalid,
    /// The target contains the complete, current frame.
    Valid,
}

/// Identity of the resource and rendering context for a persistent target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PersistentTargetKey {
    width: u32,
    height: u32,
    scale_bits: u32,
    surface_identity: u64,
    renderer_generation: u64,
    context_generation: u64,
    resource_generation: u64,
    validity: TargetValidity,
}

impl PersistentTargetKey {
    /// Creates a target identity from physical size, scale, and owner epochs.
    #[inline]
    pub(crate) fn new(
        width: u32,
        height: u32,
        scale: f32,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        validity: TargetValidity,
    ) -> Self {
        Self::new_with_resource(
            width,
            height,
            scale,
            surface_identity,
            renderer_generation,
            context_generation,
            0,
            validity,
        )
    }

    /// Creates a target identity including the resource epoch used by a
    /// retained frame.
    #[inline]
    pub(crate) fn new_with_resource(
        width: u32,
        height: u32,
        scale: f32,
        surface_identity: u64,
        renderer_generation: u64,
        context_generation: u64,
        resource_generation: u64,
        validity: TargetValidity,
    ) -> Self {
        Self {
            width,
            height,
            scale_bits: scale.to_bits(),
            surface_identity,
            renderer_generation,
            context_generation,
            resource_generation,
            validity,
        }
    }

    /// Returns this identity with a new content-validity state.
    #[inline]
    pub(crate) const fn with_validity(self, validity: TargetValidity) -> Self {
        Self { validity, ..self }
    }

    /// Reuses the same renderer and surface identity for an intermediate
    /// target whose dimensions are local to one retained group.
    #[inline]
    pub(crate) const fn with_size(self, width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ..self
        }
    }

    #[inline]
    fn same_resource(self, other: Self) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.scale_bits == other.scale_bits
            && self.surface_identity == other.surface_identity
            && self.renderer_generation == other.renderer_generation
            && self.context_generation == other.context_generation
            && self.resource_generation == other.resource_generation
    }
}

/// The result of checking or creating a persistent target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TargetEnsureResult {
    /// The requested size cannot produce a GPU target.
    Unavailable,
    /// No target existed and one was initialized.
    Created,
    /// A target existed, but its resource identity or format changed.
    Recreated,
    /// The initialized target and its contents can be reused.
    ReusedValid,
    /// The target allocation can be reused, but its pixels require a repaint.
    ReusedInvalid,
}

/// CPU-side state shared by target policy and the GPU-backed target.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PersistentTargetState {
    key: Option<PersistentTargetKey>,
}

impl PersistentTargetState {
    #[inline]
    pub(crate) fn initialized(key: PersistentTargetKey) -> Self {
        Self { key: Some(key) }
    }

    #[inline]
    pub(crate) fn same_resource(&self, key: PersistentTargetKey) -> bool {
        self.key.is_some_and(|current| current.same_resource(key))
    }

    #[inline]
    pub(crate) fn can_reuse_contents(&self, key: PersistentTargetKey) -> bool {
        self.key == Some(key) && key.validity == TargetValidity::Valid
    }

    #[inline]
    fn invalidate(&mut self) {
        if let Some(key) = self.key {
            self.key = Some(key.with_validity(TargetValidity::Invalid));
        }
    }

    #[inline]
    fn mark_valid(&mut self) {
        if let Some(key) = self.key {
            self.key = Some(key.with_validity(TargetValidity::Valid));
        }
    }

    #[inline]
    pub(crate) fn validity(&self) -> TargetValidity {
        self.key.map_or(TargetValidity::Unknown, |key| key.validity)
    }

    #[inline]
    pub(crate) fn key(&self) -> Option<PersistentTargetKey> {
        self.key
    }
}

/// A renderer-owned color target that survives between frames.

pub(crate) struct PersistentTargetGeneric<B: GpuBackend> {
    state: PersistentTargetState,
    format: Option<B::TextureFormat>,
    texture: Option<B::Texture>,
    view: Option<B::TextureView>,
}

impl<B: GpuBackend> Default for PersistentTargetGeneric<B> {
    fn default() -> Self {
        Self {
            state: PersistentTargetState::default(),
            format: None,
            texture: None,
            view: None,
        }
    }
}

impl<B: GpuBackend> PersistentTargetGeneric<B> {
    /// Ensures that a target allocation exists for `key` and `format`.
    #[inline]
    pub(crate) fn ensure(
        &mut self,
        backend: &B,
        format: B::TextureFormat,
        key: PersistentTargetKey,
    ) -> TargetEnsureResult {
        if key.width == 0 || key.height == 0 {
            return TargetEnsureResult::Unavailable;
        }

        let resource_matches =
            self.texture.is_some() && self.format == Some(format) && self.state.same_resource(key);
        if resource_matches {
            return if self.state.can_reuse_contents(key) {
                TargetEnsureResult::ReusedValid
            } else {
                TargetEnsureResult::ReusedInvalid
            };
        }

        let had_target = self.texture.is_some();
        let texture = backend.create_texture(&crate::backend::TextureDescriptor {
            label: Some("aimer persistent color target".into()),
            size: (key.width, key.height, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: crate::backend::TextureDimension::D2,
            format,
            usage: vec![
                crate::backend::TextureUsage::RenderAttachment,
                crate::backend::TextureUsage::CopyDst,
                crate::backend::TextureUsage::CopySrc,
                crate::backend::TextureUsage::TextureBinding,
            ],
        });
        let view = backend.create_texture_view(&texture, "persistent target view");
        self.texture = Some(texture);
        self.view = Some(view);
        self.format = Some(format);
        self.state = PersistentTargetState::initialized(key.with_validity(TargetValidity::Invalid));
        if had_target {
            TargetEnsureResult::Recreated
        } else {
            TargetEnsureResult::Created
        }
    }

    /// Marks the current target pixels complete and eligible for reuse.
    #[inline]
    pub(crate) fn mark_valid(&mut self) {
        self.state.mark_valid();
    }

    /// Marks the target contents unknown while retaining its allocation.
    #[inline]
    pub(crate) fn invalidate(&mut self) {
        self.state.invalidate();
    }

    /// Drops GPU resources after context loss or renderer teardown.
    #[inline]
    pub(crate) fn discard(&mut self) {
        self.state = PersistentTargetState::default();
        self.format = None;
        self.view = None;
        self.texture = None;
    }

    #[inline]
    pub(crate) fn state(&self) -> PersistentTargetState {
        self.state
    }

    #[inline]
    pub(crate) fn view(&self) -> Option<&B::TextureView> {
        self.view.as_ref()
    }

    #[inline]
    pub(crate) fn texture(&self) -> Option<&B::Texture> {
        self.texture.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_key_covers_every_resource_identity() {
        let base = PersistentTargetKey::new(800, 600, 2.0, 7, 11, 13, TargetValidity::Valid);

        assert_ne!(
            base,
            PersistentTargetKey::new(801, 600, 2.0, 7, 11, 13, TargetValidity::Valid)
        );
        assert_ne!(
            base,
            PersistentTargetKey::new(800, 601, 2.0, 7, 11, 13, TargetValidity::Valid)
        );
        assert_ne!(
            base,
            PersistentTargetKey::new(800, 600, 1.0, 7, 11, 13, TargetValidity::Valid)
        );
        assert_ne!(
            base,
            PersistentTargetKey::new(800, 600, 2.0, 8, 11, 13, TargetValidity::Valid)
        );
        assert_ne!(
            base,
            PersistentTargetKey::new(800, 600, 2.0, 7, 12, 13, TargetValidity::Valid)
        );
        assert_ne!(
            base,
            PersistentTargetKey::new(800, 600, 2.0, 7, 11, 14, TargetValidity::Valid)
        );
        assert_ne!(base, base.with_validity(TargetValidity::Invalid));
    }

    #[test]
    fn invalid_contents_require_a_full_repaint_but_can_keep_the_target_allocation() {
        let valid = PersistentTargetKey::new(800, 600, 2.0, 7, 11, 13, TargetValidity::Valid);
        let invalid = valid.with_validity(TargetValidity::Invalid);
        let state = PersistentTargetState::initialized(valid);

        assert!(state.can_reuse_contents(valid));
        assert!(!state.can_reuse_contents(invalid));
        assert!(state.same_resource(invalid));
    }

    #[test]
    fn a_new_target_starts_unknown_and_becomes_valid_only_after_paint() {
        let key = PersistentTargetKey::new(800, 600, 2.0, 7, 11, 13, TargetValidity::Invalid);
        let mut state = PersistentTargetState::initialized(key);

        assert_eq!(state.validity(), TargetValidity::Invalid);
        assert!(!state.can_reuse_contents(key.with_validity(TargetValidity::Valid)));

        state.mark_valid();

        assert_eq!(state.validity(), TargetValidity::Valid);
        assert!(state.can_reuse_contents(key.with_validity(TargetValidity::Valid)));
    }

    #[test]
    fn a_default_target_state_starts_unknown() {
        let state = PersistentTargetState::default();
        assert_eq!(state.validity(), TargetValidity::Unknown);
        assert_eq!(state.key(), None);
    }
}
