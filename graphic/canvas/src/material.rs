use aimer_attribute::position::Vec2d;
use aimer_attribute::size::ResolvedSize;
pub use aimer_cupid::material::{
    GradientShape, MAX_GRADIENT_STOPS, MagnificationStyle, MaterialGradient,
};

use crate::canvas::FrameCanvas;

/// The custom-pipeline name reserved for the W14 material bridge.
///
/// Cupid registers the material stage by default. The container's bounded
/// fallback remains visible underneath it when a renderer cannot accept a
/// request because of capability or budget limits.
pub const MATERIAL_PIPELINE_NAME: &str = "aimer.material";

/// Selects the surface family encoded in a material packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialKind {
    /// A static translucent surface.
    #[default]
    Glass = 0,
    /// A translucent surface with bounded dynamic fields.
    Liquid = 1,
    /// Filters the captured backdrop without glass styling or refraction.
    BackdropFilter = 2,
    /// Magnifies the captured backdrop with optional rounded-edge refraction.
    Magnification = 3,
    /// A directional analytic highlight with no backdrop sampling.
    LightEdge = 4,
    /// An analytic linear, radial or conic color ramp with no backdrop sampling.
    Gradient = 5,
}

/// Reduced-motion policy carried by a material packet.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialMotionPolicy {
    /// Permit dynamic Liquid fields.
    #[default]
    Full = 0,
    /// Disable distortion and animated highlight fields.
    Reduced = 1,
}

/// An optional rounded clip carried with a material surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialClip {
    /// `[x, y, width, height]` in the request's local coordinate space.
    pub rect: [f32; 4],
    /// `[top-left, top-right, bottom-right, bottom-left]` radii.
    pub corner_radii: [f32; 4],
}

impl MaterialClip {
    /// Creates and normalizes a rounded clip.
    #[inline]
    pub fn new(rect: [f32; 4], corner_radii: [f32; 4]) -> Self {
        Self {
            rect: normalize_rect(rect),
            corner_radii: normalize_radii(corner_radii),
        }
    }
}

/// A renderer-neutral material draw request.
///
/// The packet records geometry, transform, clip, z-order, and all bounded
/// effect values needed by Cupid. It intentionally contains no `wgpu` object,
/// window handle, platform visual-effect object, or arbitrary callback. The
/// values are encoded as a fixed little-endian packet so a renderer can consume
/// a request without depending on this UI-facing crate's Rust types.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialDrawRequest {
    /// Surface family.
    pub kind: MaterialKind,
    /// Local bounds `[x, y, width, height]`.
    pub bounds: [f32; 4],
    /// Column-major 3x3 transform.
    pub transform: [f32; 9],
    /// Optional effective clip.
    pub clip: Option<MaterialClip>,
    /// Stable ordering key for overlapping and nested surfaces.
    pub z_order: u32,
    /// Tint RGBA in normalized channels.
    pub tint: [f32; 4],
    /// Surface alpha multiplier.
    pub opacity: f32,
    /// Backdrop blur radius in logical pixels.
    pub blur_radius: f32,
    /// Backdrop saturation multiplier.
    pub saturation: f32,
    /// Backdrop brightness multiplier.
    pub brightness: f32,
    /// Backdrop contrast multiplier.
    pub contrast: f32,
    /// Border highlight RGBA.
    pub border_color: [f32; 4],
    /// Border width in logical pixels.
    pub border_width: f32,
    /// Per-corner surface radii.
    pub corner_radii: [f32; 4],
    /// Shadow RGBA.
    pub shadow_color: [f32; 4],
    /// Shadow blur radius in logical pixels.
    pub shadow_blur: f32,
    /// Shadow/elevation offset in logical pixels.
    pub elevation: f32,
    /// Bounded dynamic distortion strength.
    pub distortion_strength: f32,
    /// Bounded edge-lighting strength.
    /// For [`MaterialKind::LightEdge`], opposing reflection relative to the primary highlight.
    pub edge_lighting: f32,
    /// Bounded specular-highlight strength.
    pub specular_highlight: f32,
    /// Dynamic field speed.
    pub animation_speed: f32,
    /// Injected animation time.
    pub animation_time: f32,
    /// Normalized pointer/interaction influence.
    pub interaction: f32,
    /// Liquid organic-silhouette wobble strength.
    pub blob_amount: f32,
    /// Liquid deterministic silhouette variation seed, in `0.0..=1.0`.
    pub blob_seed: f32,
    /// Liquid bulge strength (`0..=1`), or Magnification zoom factor (`1..=8`).
    pub magnification: f32,
    /// Liquid teardrop-tip taper strength.
    pub tip_pull: f32,
    /// Liquid chromatic-aberration strength at the refracting edge.
    pub chromatic_aberration: f32,
    /// Liquid bevel radius or Magnification lens thickness, in logical pixels.
    /// Builds the rounded-edge normal that drives refraction.
    pub bevel_radius: f32,
    /// Composable Magnification effects, axes, and anchors.
    pub magnification_style: MagnificationStyle,
    /// Additional Magnification bulge strength in `0..=1`.
    pub bulge_strength: f32,
    /// LightEdge light direction in degrees clockwise from screen-right.
    pub light_angle: f32,
    /// Color ramp of a [`MaterialKind::Gradient`]; ignored by every other kind.
    pub gradient: MaterialGradient,
    /// Sampling quality in `1..=4`.
    pub quality: u8,
    /// Motion policy for dynamic fields.
    pub motion_policy: MaterialMotionPolicy,
}

impl MaterialDrawRequest {
    /// Packet schema version.
    pub const VERSION: u8 = 3;
    /// Exact byte length of [`Self::encode`] for every kind but a gradient.
    pub const PACKET_LEN: usize = 256;
    /// Packet schema version of a [`MaterialKind::Gradient`], whose color ramp
    /// does not fit the shared body and is appended to it.
    pub const GRADIENT_VERSION: u8 = 4;
    /// Exact byte length of [`Self::encode`] for a [`MaterialKind::Gradient`].
    pub const GRADIENT_PACKET_LEN: usize =
        Self::PACKET_LEN + aimer_cupid::material::GRADIENT_PACKET_EXTENSION_LEN;
    /// Maximum blur accepted by the bridge.
    pub const MAX_BLUR_RADIUS: f32 = 96.0;
    /// Maximum shadow blur accepted by the bridge.
    pub const MAX_SHADOW_BLUR: f32 = 128.0;
    /// Maximum corner radius accepted by the bridge.
    pub const MAX_CORNER_RADIUS: f32 = 1_024.0;
    /// Maximum dynamic animation speed accepted by the bridge.
    pub const MAX_ANIMATION_SPEED: f32 = 4.0;
    /// Maximum absolute injected animation time.
    pub const MAX_ANIMATION_TIME: f32 = 1_000_000.0;

    /// Creates a request with safe static defaults for `kind` and `bounds`.
    #[inline]
    pub fn new(kind: MaterialKind, bounds: [f32; 4]) -> Self {
        Self {
            kind,
            bounds: normalize_rect(bounds),
            transform: identity_transform(),
            clip: None,
            z_order: 0,
            tint: [1.0, 1.0, 1.0, 1.0],
            opacity: 0.72,
            blur_radius: 16.0,
            saturation: 1.0,
            brightness: 1.0,
            contrast: 1.0,
            border_color: [1.0, 1.0, 1.0, 96.0 / 255.0],
            border_width: 1.0,
            corner_radii: [16.0; 4],
            shadow_color: [0.0, 0.0, 0.0, 72.0 / 255.0],
            shadow_blur: 16.0,
            elevation: 4.0,
            distortion_strength: 0.18,
            edge_lighting: 0.35,
            specular_highlight: 0.28,
            animation_speed: 0.5,
            animation_time: 0.0,
            interaction: 0.0,
            blob_amount: 0.0,
            blob_seed: 0.5,
            magnification: if kind == MaterialKind::Magnification {
                1.5
            } else {
                0.45
            },
            tip_pull: 0.0,
            chromatic_aberration: 0.3,
            bevel_radius: if kind == MaterialKind::Magnification {
                16.0
            } else {
                28.0
            },
            magnification_style: if kind == MaterialKind::Magnification {
                MagnificationStyle::DEFAULT
            } else {
                MagnificationStyle::NONE
            },
            bulge_strength: if kind == MaterialKind::Magnification {
                0.25
            } else {
                0.0
            },
            light_angle: 225.0,
            gradient: MaterialGradient::default(),
            quality: 2,
            motion_policy: MaterialMotionPolicy::Full,
        }
    }

    /// Creates a request from a local canvas position and size.
    #[inline]
    pub fn from_layout(kind: MaterialKind, pos: Vec2d, size: ResolvedSize) -> Self {
        Self::new(kind, [pos.x, pos.y, size.width, size.height])
    }

    /// Re-normalizes every float and bounded integer in the request.
    #[inline]
    pub fn normalized(mut self) -> Self {
        self.bounds = normalize_rect(self.bounds);
        self.transform = normalize_transform(self.transform);
        self.clip = self
            .clip
            .map(|clip| MaterialClip::new(clip.rect, clip.corner_radii));
        self.tint = normalize_color(self.tint);
        self.opacity = normalize(self.opacity, 0.72, 0.0, 1.0);
        self.blur_radius = normalize(self.blur_radius, 16.0, 0.0, Self::MAX_BLUR_RADIUS);
        self.saturation = normalize(self.saturation, 1.0, 0.0, 3.0);
        self.brightness = normalize(self.brightness, 1.0, 0.0, 3.0);
        self.contrast = normalize(self.contrast, 1.0, 0.0, 3.0);
        self.border_color = normalize_color(self.border_color);
        self.border_width = normalize(self.border_width, 1.0, 0.0, 16.0);
        self.corner_radii = normalize_radii(self.corner_radii);
        self.shadow_color = normalize_color(self.shadow_color);
        self.shadow_blur = normalize(self.shadow_blur, 16.0, 0.0, Self::MAX_SHADOW_BLUR);
        self.elevation = normalize(self.elevation, 4.0, 0.0, 64.0);
        self.distortion_strength = normalize(self.distortion_strength, 0.18, 0.0, 1.0);
        self.edge_lighting = normalize(self.edge_lighting, 0.35, 0.0, 1.0);
        self.specular_highlight = normalize(self.specular_highlight, 0.28, 0.0, 1.0);
        self.animation_speed = normalize(self.animation_speed, 0.5, 0.0, Self::MAX_ANIMATION_SPEED);
        self.animation_time = normalize(
            self.animation_time,
            0.0,
            -Self::MAX_ANIMATION_TIME,
            Self::MAX_ANIMATION_TIME,
        );
        self.interaction = normalize(self.interaction, 0.0, 0.0, 1.0);
        self.blob_amount = normalize(self.blob_amount, 0.0, 0.0, 1.0);
        self.blob_seed = normalize(self.blob_seed, 0.5, 0.0, 1.0);
        self.magnification = if self.kind == MaterialKind::Magnification {
            normalize(self.magnification, 1.5, 1.0, 8.0)
        } else {
            normalize(self.magnification, 0.45, 0.0, 1.0)
        };
        self.tip_pull = normalize(self.tip_pull, 0.0, 0.0, 1.0);
        self.chromatic_aberration = normalize(self.chromatic_aberration, 0.3, 0.0, 1.0);
        self.bevel_radius = normalize(
            self.bevel_radius,
            if self.kind == MaterialKind::Magnification {
                16.0
            } else {
                28.0
            },
            0.0,
            512.0,
        );
        if self.kind == MaterialKind::Magnification {
            self.bulge_strength = normalize(self.bulge_strength, 0.25, 0.0, 1.0);
        } else {
            self.magnification_style = MagnificationStyle::NONE;
            self.bulge_strength = 0.0;
        }
        self.light_angle = if self.light_angle.is_finite() {
            self.light_angle.rem_euclid(360.0)
        } else {
            225.0
        };
        self.gradient = if self.kind == MaterialKind::Gradient {
            self.gradient.normalized()
        } else {
            MaterialGradient::default()
        };
        self.quality = self.quality.clamp(1, 4);
        if self.motion_policy == MaterialMotionPolicy::Reduced {
            self.distortion_strength = 0.0;
            self.animation_speed = 0.0;
        }
        self
    }

    /// Encodes a normalized request into the fixed bridge packet.
    #[inline]
    pub fn encode(self) -> Vec<u8> {
        let request = self.normalized();
        let mut bytes = Vec::with_capacity(Self::GRADIENT_PACKET_LEN);
        bytes.extend_from_slice(b"AIMR");
        bytes.push(if request.kind == MaterialKind::Gradient {
            Self::GRADIENT_VERSION
        } else {
            Self::VERSION
        });
        bytes.push(request.kind as u8);
        bytes.push(request.motion_policy as u8);
        bytes.push(request.quality);
        bytes.extend_from_slice(&request.z_order.to_le_bytes());
        bytes.push(request.clip.is_some() as u8);
        bytes.extend_from_slice(&[0, 0, 0]);

        push_f32s(&mut bytes, &request.bounds);
        push_f32s(&mut bytes, &request.transform);
        let clip = request.clip.unwrap_or(MaterialClip {
            rect: [0.0; 4],
            corner_radii: [0.0; 4],
        });
        push_f32s(&mut bytes, &clip.rect);
        push_f32s(&mut bytes, &clip.corner_radii);
        push_f32s(&mut bytes, &request.tint);
        push_f32(&mut bytes, request.opacity);
        push_f32(&mut bytes, request.blur_radius);
        push_f32(&mut bytes, request.saturation);
        push_f32(&mut bytes, request.brightness);
        push_f32(&mut bytes, request.contrast);
        push_f32s(&mut bytes, &request.border_color);
        push_f32(&mut bytes, request.border_width);
        push_f32s(&mut bytes, &request.corner_radii);
        push_f32s(&mut bytes, &request.shadow_color);
        push_f32(&mut bytes, request.shadow_blur);
        push_f32(&mut bytes, request.elevation);
        push_f32(&mut bytes, request.distortion_strength);
        push_f32(&mut bytes, request.edge_lighting);
        push_f32(&mut bytes, request.specular_highlight);
        push_f32(&mut bytes, request.animation_speed);
        push_f32(&mut bytes, request.animation_time);
        push_f32(&mut bytes, request.interaction);
        push_f32(&mut bytes, request.blob_amount);
        push_f32(&mut bytes, request.blob_seed);
        push_f32(&mut bytes, request.magnification);
        push_f32(&mut bytes, request.tip_pull);
        push_f32(&mut bytes, request.chromatic_aberration);
        push_f32(&mut bytes, request.bevel_radius);
        bytes.extend_from_slice(&request.magnification_style.bits().to_le_bytes());
        push_f32(&mut bytes, request.bulge_strength);
        push_f32(&mut bytes, request.light_angle);
        debug_assert_eq!(bytes.len(), Self::PACKET_LEN);
        if request.kind == MaterialKind::Gradient {
            request.gradient.encode_extension(&mut bytes);
            debug_assert_eq!(bytes.len(), Self::GRADIENT_PACKET_LEN);
        }
        bytes
    }

    /// Records this request at the current canvas ordering point.
    ///
    /// The bridge uses the existing custom-command channel so no renderer
    /// object leaks into the public canvas API. `take_draw_list`/`recycle_draw_list`
    /// is intentionally isolated here while Cupid resolves the active
    /// transform, clip, and alpha state at render time.
    #[inline]
    pub fn record(self, canvas: &FrameCanvas<'_>) {
        let inner = canvas.get_inner_canvas();
        let mut draw_list = inner.take_draw_list();
        draw_list.draw_custom(MATERIAL_PIPELINE_NAME, self.encode());
        inner.recycle_draw_list(draw_list);
    }
}

/// Records a material packet without requiring callers to name the bridge
/// type in a widget builder.
#[inline]
pub fn record_material(canvas: &FrameCanvas<'_>, request: MaterialDrawRequest) {
    request.record(canvas);
}

#[inline]
fn identity_transform() -> [f32; 9] {
    [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]
}

#[inline]
fn normalize(value: f32, fallback: f32, min: f32, max: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        fallback.clamp(min, max)
    }
}

#[inline]
fn normalize_rect(mut rect: [f32; 4]) -> [f32; 4] {
    rect[0] = normalize(rect[0], 0.0, -1_000_000.0, 1_000_000.0);
    rect[1] = normalize(rect[1], 0.0, -1_000_000.0, 1_000_000.0);
    rect[2] = normalize(rect[2], 0.0, 0.0, 1_000_000.0);
    rect[3] = normalize(rect[3], 0.0, 0.0, 1_000_000.0);
    rect
}

#[inline]
fn normalize_transform(mut transform: [f32; 9]) -> [f32; 9] {
    for value in &mut transform {
        *value = normalize(*value, 0.0, -1_000_000.0, 1_000_000.0);
    }
    transform
}

#[inline]
fn normalize_color(mut color: [f32; 4]) -> [f32; 4] {
    for channel in &mut color {
        *channel = normalize(*channel, 0.0, 0.0, 1.0);
    }
    color
}

#[inline]
fn normalize_radii(mut radii: [f32; 4]) -> [f32; 4] {
    for radius in &mut radii {
        *radius = normalize(*radius, 0.0, 0.0, MaterialDrawRequest::MAX_CORNER_RADIUS);
    }
    radii
}

#[inline]
fn push_f32(bytes: &mut Vec<u8>, value: f32) {
    bytes.extend_from_slice(&value.to_bits().to_le_bytes());
}

#[inline]
fn push_f32s<const N: usize>(bytes: &mut Vec<u8>, values: &[f32; N]) {
    for value in values {
        push_f32(bytes, *value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_edge_packet_carries_normalized_angle_and_controls() {
        let mut request = MaterialDrawRequest::new(MaterialKind::LightEdge, [1.0, 2.0, 80.0, 40.0]);
        request.light_angle = -135.0;
        request.border_color = [0.25, 0.5, 1.0, 0.5];
        request.border_width = 4.0;
        request.specular_highlight = 0.75;
        request.edge_lighting = 0.65;
        let encoded = request.encode();
        assert_eq!(encoded.len(), 256);
        assert_eq!(encoded[4], 3);
        let decoded = aimer_cupid::material::MaterialRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.kind, aimer_cupid::material::MaterialKind::LightEdge);
        assert_eq!(decoded.light_angle, 225.0);
        assert_eq!(decoded.border_color, [0.25, 0.5, 1.0, 0.5]);
        assert_eq!(decoded.border_width, 4.0);
        assert_eq!(decoded.specular_highlight, 0.75);
        assert_eq!(decoded.edge_lighting, 0.65);
        request.light_angle = f32::NAN;
        assert_eq!(request.normalized().light_angle, 225.0);
    }

    #[test]
    fn gradient_packets_append_their_ramp_and_decode_in_cupid() {
        let mut request = MaterialDrawRequest::new(MaterialKind::Gradient, [1.0, 2.0, 80.0, 40.0]);
        request.opacity = 1.0;
        request.gradient = MaterialGradient {
            shape: GradientShape::Conic,
            geometry: [40.0, 20.0, 1.0, 0.0],
            stop_count: 2,
            ..MaterialGradient::default()
        };
        request.gradient.offsets[1] = 1.0;
        request.gradient.colors = [
            MaterialGradient::pack_color([255, 0, 0, 255]),
            MaterialGradient::pack_color([0, 0, 255, 128]),
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        let encoded = request.encode();
        assert_eq!(encoded.len(), MaterialDrawRequest::GRADIENT_PACKET_LEN);
        assert_eq!(encoded[4], MaterialDrawRequest::GRADIENT_VERSION);
        let decoded = aimer_cupid::material::MaterialRequest::decode(&encoded).unwrap();
        assert_eq!(decoded.kind, aimer_cupid::material::MaterialKind::Gradient);
        assert_eq!(decoded.gradient, request.normalized().gradient);
        assert_eq!(decoded.gradient.stop_count, 2);
    }

    #[test]
    fn a_ramp_on_a_kind_that_ignores_it_changes_no_bytes() {
        let plain = MaterialDrawRequest::new(MaterialKind::Glass, [0.0, 0.0, 20.0, 10.0]);
        let mut with_ramp = plain;
        with_ramp.gradient.stop_count = 3;
        with_ramp.gradient.colors[0] = 0xffff_ffff;
        assert_eq!(plain.encode(), with_ramp.encode());
        assert_eq!(with_ramp.encode().len(), MaterialDrawRequest::PACKET_LEN);
    }

    #[test]
    fn request_normalization_removes_non_finite_and_out_of_range_values() {
        let request =
            MaterialDrawRequest::new(MaterialKind::Liquid, [0.0, 0.0, 20.0, 10.0]).normalized();
        let mut invalid = request;
        invalid.opacity = f32::NAN;
        invalid.blur_radius = f32::INFINITY;
        invalid.tint = [f32::NAN, -1.0, 2.0, 0.5];
        invalid.quality = 0;
        invalid.motion_policy = MaterialMotionPolicy::Reduced;
        invalid.distortion_strength = 0.9;
        let normalized = invalid.normalized();

        assert_eq!(normalized.opacity, 0.72);
        assert_eq!(normalized.blur_radius, 16.0);
        assert_eq!(normalized.tint, [0.0, 0.0, 1.0, 0.5]);
        assert_eq!(normalized.quality, 1);
        assert_eq!(normalized.distortion_strength, 0.0);
        assert_eq!(normalized.animation_speed, 0.0);
    }

    #[test]
    fn encoding_is_fixed_length_and_deterministic() {
        let request = MaterialDrawRequest::new(MaterialKind::Glass, [1.0, 2.0, 80.0, 40.0]);
        assert_eq!(request.encode().len(), MaterialDrawRequest::PACKET_LEN);
        assert_eq!(request.encode(), request.encode());
        assert_eq!(&request.encode()[..4], b"AIMR");
    }

    #[test]
    fn reduced_motion_is_encoded_as_a_static_request() {
        let request = MaterialDrawRequest::new(MaterialKind::Liquid, [0.0, 0.0, 10.0, 10.0]);
        let mut reduced = request;
        reduced.motion_policy = MaterialMotionPolicy::Reduced;
        reduced.distortion_strength = 1.0;
        reduced.animation_speed = 4.0;
        assert_eq!(reduced.normalized().distortion_strength, 0.0);
        assert_eq!(reduced.normalized().animation_speed, 0.0);
    }

    #[test]
    fn clip_and_transform_are_carried_without_platform_values() {
        let mut request = MaterialDrawRequest::new(MaterialKind::Glass, [0.0, 0.0, 50.0, 30.0]);
        request.transform = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 4.0, 5.0, 1.0];
        request.clip = Some(MaterialClip::new(
            [1.0, 2.0, 40.0, 20.0],
            [8.0, 8.0, 8.0, 8.0],
        ));
        let encoded = request.encode();
        assert_eq!(encoded.len(), MaterialDrawRequest::PACKET_LEN);
        assert_eq!(request.transform[6], 4.0);
        assert!(request.clip.is_some());
    }
}
