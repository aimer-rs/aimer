mod gradient;
mod magnification_style;
mod render;

pub use gradient::{
    GRADIENT_PACKET_EXTENSION_LEN, GradientShape, MAX_GRADIENT_STOPS, MaterialGradient,
};
pub use magnification_style::MagnificationStyle;
pub use render::MaterialPipeline;

/// Reserved custom-pipeline name for W14 material requests.
pub const MATERIAL_PIPELINE_NAME: &str = "aimer.material";

/// The packet version shared by the canvas bridge and Cupid.
pub const MATERIAL_PACKET_VERSION: u8 = 3;
/// The exact size of one encoded material request.
pub const MATERIAL_PACKET_LEN: usize = 256;
/// The packet version of [`MaterialKind::Gradient`] requests.
///
/// A gradient carries a color ramp that the shared body has no room for, so
/// its packets append [`GRADIENT_PACKET_EXTENSION_LEN`] bytes to the version-3
/// body. Every other kind keeps version 3 and its exact length.
pub const MATERIAL_GRADIENT_PACKET_VERSION: u8 = 4;
/// The exact size of one encoded [`MaterialKind::Gradient`] request.
pub const MATERIAL_GRADIENT_PACKET_LEN: usize = MATERIAL_PACKET_LEN + GRADIENT_PACKET_EXTENSION_LEN;
/// Maximum intermediate texture dimension used by one surface.
///
/// iOS devices get a larger budget; see [`intermediate_pixel_budget`].
pub const MAX_INTERMEDIATE_DIMENSION: u32 = intermediate_dimension_budget(cfg!(target_os = "ios"));
/// Maximum intermediate RGBA pixels used by one surface.
///
/// iOS devices get a larger budget; see [`intermediate_pixel_budget`].
pub const MAX_INTERMEDIATE_PIXELS: u64 = intermediate_pixel_budget(cfg!(target_os = "ios"));

/// The intermediate pixel budget for a device class.
///
/// A backdrop capture is clamped to the viewport, so the largest one a
/// material can request is a full screen. The conservative 4 Mpx budget (16 MB
/// of RGBA, doubled while a blur runs) fits every phone, but not an iPad:
/// the 13-inch Pro is 2064 x 2752 = 5.7 Mpx, and a surface over budget is not
/// rendered as a material at all. 8 Mpx covers every shipping iPad with room
/// for the blur padding. Textures are sized to the regions actually captured,
/// so the larger budget costs memory only when such a surface is on screen.
pub const fn intermediate_pixel_budget(roomy_device: bool) -> u64 {
    if roomy_device {
        8 * 1024 * 1024
    } else {
        4 * 1024 * 1024
    }
}

/// The intermediate texture dimension limit for a device class.
///
/// Replacement effects use the hardware limit; this bounds the downsampled
/// Glass and Liquid surfaces, whose long edge on an iPad exceeds 2,048.
pub const fn intermediate_dimension_budget(roomy_device: bool) -> u32 {
    if roomy_device { 4_096 } else { 2_048 }
}

/// Selects the surface family.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialKind {
    /// Static translucent glass.
    #[default]
    Glass = 0,
    /// Dynamic glass with bounded liquid fields.
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

impl MaterialKind {
    #[inline]
    fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Glass),
            1 => Some(Self::Liquid),
            2 => Some(Self::BackdropFilter),
            3 => Some(Self::Magnification),
            4 => Some(Self::LightEdge),
            5 => Some(Self::Gradient),
            _ => None,
        }
    }

    /// Whether this kind reads the captured backdrop.
    ///
    /// Analytic kinds draw from their own parameters alone, so they need no
    /// intermediate texture and never force a frame capture.
    #[inline]
    pub const fn samples_backdrop(self) -> bool {
        !matches!(self, Self::LightEdge | Self::Gradient)
    }
}

/// Policy controlling dynamic fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum MaterialMotionPolicy {
    /// Permit configured Liquid motion.
    #[default]
    Full = 0,
    /// Disable distortion and animated highlight motion.
    Reduced = 1,
}

impl MaterialMotionPolicy {
    #[inline]
    fn from_wire(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Full),
            1 => Some(Self::Reduced),
            _ => None,
        }
    }
}

/// An optional rounded clip for a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialClip {
    /// `[x, y, width, height]` in local coordinates.
    pub rect: [f32; 4],
    /// `[top-left, top-right, bottom-right, bottom-left]` radii.
    pub corner_radii: [f32; 4],
}

/// A material request after it has crossed the canvas boundary.
///
/// This is intentionally a plain value model. GPU resources, offscreen
/// textures, and render-pass lifetimes belong to the stage that consumes this
/// request, not to the widget or canvas packages.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialRequest {
    /// Surface family.
    pub kind: MaterialKind,
    /// Local bounds `[x, y, width, height]`.
    pub bounds: [f32; 4],
    /// Column-major 3x3 transform.
    pub transform: [f32; 9],
    /// Optional clip.
    pub clip: Option<MaterialClip>,
    /// Stable ordering key.
    pub z_order: u32,
    /// Tint RGBA.
    pub tint: [f32; 4],
    /// Surface opacity.
    pub opacity: f32,
    /// Backdrop blur radius.
    pub blur_radius: f32,
    /// Backdrop saturation multiplier.
    pub saturation: f32,
    /// Backdrop brightness multiplier.
    pub brightness: f32,
    /// Backdrop contrast multiplier.
    pub contrast: f32,
    /// Border RGBA.
    pub border_color: [f32; 4],
    /// Border width.
    pub border_width: f32,
    /// Surface corner radii.
    pub corner_radii: [f32; 4],
    /// Shadow RGBA.
    pub shadow_color: [f32; 4],
    /// Shadow blur radius.
    pub shadow_blur: f32,
    /// Shadow/elevation offset.
    pub elevation: f32,
    /// Liquid distortion strength.
    pub distortion_strength: f32,
    /// Liquid edge-lighting strength.
    /// For [`MaterialKind::LightEdge`], opposing reflection relative to the primary highlight.
    pub edge_lighting: f32,
    /// Liquid specular strength.
    pub specular_highlight: f32,
    /// Liquid animation speed.
    pub animation_speed: f32,
    /// Injected animation time.
    pub animation_time: f32,
    /// Pointer/interaction influence.
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
    /// Color ramp of a [`MaterialKind::Gradient`]; empty for every other kind.
    pub gradient: MaterialGradient,
    /// Sampling quality in `1..=4`.
    pub quality: u8,
    /// Reduced-motion policy.
    pub motion_policy: MaterialMotionPolicy,
}

impl MaterialRequest {
    /// Creates a request with the same safe defaults as the canvas bridge.
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
            gradient: MaterialGradient {
                shape: GradientShape::Linear,
                geometry: [0.0; 4],
                stop_count: 0,
                offsets: [0.0; MAX_GRADIENT_STOPS],
                colors: [0; MAX_GRADIENT_STOPS],
            },
            quality: 2,
            motion_policy: MaterialMotionPolicy::Full,
        }
    }

    #[cfg(test)]
    pub(crate) fn encode_for_test(self) -> Vec<u8> {
        let request = self.normalized();
        let mut bytes = Vec::with_capacity(MATERIAL_GRADIENT_PACKET_LEN);
        bytes.extend_from_slice(b"AIMR");
        bytes.push(if request.kind == MaterialKind::Gradient {
            MATERIAL_GRADIENT_PACKET_VERSION
        } else {
            MATERIAL_PACKET_VERSION
        });
        bytes.push(request.kind as u8);
        bytes.push(request.motion_policy as u8);
        bytes.push(request.quality);
        bytes.extend_from_slice(&request.z_order.to_le_bytes());
        bytes.push(request.clip.is_some() as u8);
        bytes.extend_from_slice(&[0, 0, 0]);
        let clip = request.clip.unwrap_or(MaterialClip {
            rect: [0.0; 4],
            corner_radii: [0.0; 4],
        });
        for values in [
            request.bounds.as_slice(),
            request.transform.as_slice(),
            clip.rect.as_slice(),
            clip.corner_radii.as_slice(),
            request.tint.as_slice(),
            core::slice::from_ref(&request.opacity),
            core::slice::from_ref(&request.blur_radius),
            core::slice::from_ref(&request.saturation),
            core::slice::from_ref(&request.brightness),
            core::slice::from_ref(&request.contrast),
            request.border_color.as_slice(),
            core::slice::from_ref(&request.border_width),
            request.corner_radii.as_slice(),
            request.shadow_color.as_slice(),
            core::slice::from_ref(&request.shadow_blur),
            core::slice::from_ref(&request.elevation),
            core::slice::from_ref(&request.distortion_strength),
            core::slice::from_ref(&request.edge_lighting),
            core::slice::from_ref(&request.specular_highlight),
            core::slice::from_ref(&request.animation_speed),
            core::slice::from_ref(&request.animation_time),
            core::slice::from_ref(&request.interaction),
            core::slice::from_ref(&request.blob_amount),
            core::slice::from_ref(&request.blob_seed),
            core::slice::from_ref(&request.magnification),
            core::slice::from_ref(&request.tip_pull),
            core::slice::from_ref(&request.chromatic_aberration),
            core::slice::from_ref(&request.bevel_radius),
        ] {
            for value in values {
                bytes.extend_from_slice(&value.to_bits().to_le_bytes());
            }
        }
        bytes.extend_from_slice(&request.magnification_style.bits().to_le_bytes());
        bytes.extend_from_slice(&request.bulge_strength.to_le_bytes());
        bytes.extend_from_slice(&request.light_angle.to_le_bytes());
        debug_assert_eq!(bytes.len(), MATERIAL_PACKET_LEN);
        if request.kind == MaterialKind::Gradient {
            request.gradient.encode_extension(&mut bytes);
            debug_assert_eq!(bytes.len(), MATERIAL_GRADIENT_PACKET_LEN);
        }
        bytes
    }

    /// Normalizes finite/clamped values and applies reduced-motion policy.
    #[inline]
    pub fn normalized(mut self) -> Self {
        self.bounds = normalize_rect(self.bounds);
        self.transform = normalize_transform(self.transform);
        self.clip = self.clip.map(|clip| MaterialClip {
            rect: normalize_rect(clip.rect),
            corner_radii: normalize_radii(clip.corner_radii),
        });
        self.tint = normalize_color(self.tint);
        self.opacity = normalize(self.opacity, 0.72, 0.0, 1.0);
        self.blur_radius = normalize(self.blur_radius, 16.0, 0.0, 96.0);
        self.saturation = normalize(self.saturation, 1.0, 0.0, 3.0);
        self.brightness = normalize(self.brightness, 1.0, 0.0, 3.0);
        self.contrast = normalize(self.contrast, 1.0, 0.0, 3.0);
        self.border_color = normalize_color(self.border_color);
        self.border_width = normalize(self.border_width, 1.0, 0.0, 16.0);
        self.corner_radii = normalize_radii(self.corner_radii);
        self.shadow_color = normalize_color(self.shadow_color);
        self.shadow_blur = normalize(self.shadow_blur, 16.0, 0.0, 128.0);
        self.elevation = normalize(self.elevation, 4.0, 0.0, 64.0);
        self.distortion_strength = normalize(self.distortion_strength, 0.18, 0.0, 1.0);
        self.edge_lighting = normalize(self.edge_lighting, 0.35, 0.0, 1.0);
        self.specular_highlight = normalize(self.specular_highlight, 0.28, 0.0, 1.0);
        self.animation_speed = normalize(self.animation_speed, 0.5, 0.0, 4.0);
        self.animation_time = normalize(self.animation_time, 0.0, -1_000_000.0, 1_000_000.0);
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

    /// Decodes current and legacy version-1 and version-2 canvas packets.
    pub fn decode(bytes: &[u8]) -> Result<Self, MaterialDecodeError> {
        if bytes.len() < 5 {
            return Err(MaterialDecodeError::InvalidLength {
                expected: MATERIAL_PACKET_LEN,
                actual: bytes.len(),
            });
        }
        let mut reader = PacketReader::new(bytes);
        if reader.bytes::<4>()? != b"AIMR" {
            return Err(MaterialDecodeError::InvalidMagic);
        }
        let version = reader.u8()?;
        let expected = match version {
            1 => 244,
            2 => 252,
            MATERIAL_PACKET_VERSION => MATERIAL_PACKET_LEN,
            MATERIAL_GRADIENT_PACKET_VERSION => MATERIAL_GRADIENT_PACKET_LEN,
            _ => return Err(MaterialDecodeError::UnsupportedVersion),
        };
        if bytes.len() != expected {
            return Err(MaterialDecodeError::InvalidLength {
                expected,
                actual: bytes.len(),
            });
        }
        let kind =
            MaterialKind::from_wire(reader.u8()?).ok_or(MaterialDecodeError::UnsupportedKind)?;
        // The ramp rides only in version 4, and version 4 carries only a ramp.
        if (kind == MaterialKind::Gradient) != (version == MATERIAL_GRADIENT_PACKET_VERSION) {
            return Err(MaterialDecodeError::UnsupportedKind);
        }
        let motion_policy = MaterialMotionPolicy::from_wire(reader.u8()?)
            .ok_or(MaterialDecodeError::UnsupportedMotionPolicy)?;
        let quality = reader.u8()?;
        let z_order = reader.u32()?;
        let clip_present = reader.u8()?;
        let reserved = reader.bytes::<3>()?;
        if !reserved.iter().all(|value| *value == 0) {
            return Err(MaterialDecodeError::NonZeroReserved);
        }
        if clip_present > 1 {
            return Err(MaterialDecodeError::InvalidClipFlag);
        }

        let bounds = reader.f32s::<4>()?;
        let transform = reader.f32s::<9>()?;
        let clip_rect = reader.f32s::<4>()?;
        let clip_radii = reader.f32s::<4>()?;
        let tint = reader.f32s::<4>()?;
        let opacity = reader.f32()?;
        let blur_radius = reader.f32()?;
        let saturation = reader.f32()?;
        let brightness = reader.f32()?;
        let contrast = reader.f32()?;
        let border_color = reader.f32s::<4>()?;
        let border_width = reader.f32()?;
        let corner_radii = reader.f32s::<4>()?;
        let shadow_color = reader.f32s::<4>()?;
        let shadow_blur = reader.f32()?;
        let elevation = reader.f32()?;
        let distortion_strength = reader.f32()?;
        let edge_lighting = reader.f32()?;
        let specular_highlight = reader.f32()?;
        let animation_speed = reader.f32()?;
        let animation_time = reader.f32()?;
        let interaction = reader.f32()?;
        let blob_amount = reader.f32()?;
        let blob_seed = reader.f32()?;
        let magnification = reader.f32()?;
        let tip_pull = reader.f32()?;
        let chromatic_aberration = reader.f32()?;
        let bevel_radius = reader.f32()?;
        let (magnification_style, bulge_strength) = if version == 1 {
            if kind == MaterialKind::Magnification {
                (MagnificationStyle::DEFAULT, 0.25)
            } else {
                (MagnificationStyle::NONE, 0.0)
            }
        } else {
            let style = MagnificationStyle::from_bits(reader.u32()?)
                .ok_or(MaterialDecodeError::InvalidMagnificationStyle)?;
            (style, reader.f32()?)
        };
        let light_angle = if version >= 3 { reader.f32()? } else { 225.0 };
        let gradient = if kind == MaterialKind::Gradient {
            debug_assert_eq!(reader.position(), MATERIAL_PACKET_LEN);
            MaterialGradient::decode_extension(&bytes[MATERIAL_PACKET_LEN..])?
        } else {
            debug_assert_eq!(reader.position(), expected);
            MaterialGradient::default()
        };

        Ok(Self {
            kind,
            bounds,
            transform,
            clip: (clip_present == 1).then_some(MaterialClip {
                rect: clip_rect,
                corner_radii: clip_radii,
            }),
            z_order,
            tint,
            opacity,
            blur_radius,
            saturation,
            brightness,
            contrast,
            border_color,
            border_width,
            corner_radii,
            shadow_color,
            shadow_blur,
            elevation,
            distortion_strength,
            edge_lighting,
            specular_highlight,
            animation_speed,
            animation_time,
            interaction,
            blob_amount,
            blob_seed,
            magnification,
            tip_pull,
            chromatic_aberration,
            bevel_radius,
            magnification_style,
            bulge_strength,
            light_angle,
            gradient,
            quality,
            motion_policy,
        }
        .normalized())
    }

    /// Returns a deterministic phase for the Liquid highlight field.
    #[inline]
    pub fn effective_phase(self) -> f32 {
        let request = self.normalized();
        let phase = request.animation_time * request.animation_speed + request.interaction;
        phase.is_finite().then_some(phase).unwrap_or(0.0)
    }

    /// Creates the CPU-side fallback draw description.
    #[inline]
    pub fn fallback(self) -> FallbackDraw {
        let request = self.normalized();
        FallbackDraw {
            bounds: request.bounds,
            transform: request.transform,
            clip: request.clip,
            z_order: request.z_order,
            fill_color: multiply_alpha(request.tint, request.opacity),
            border_color: multiply_alpha(request.border_color, request.opacity),
            border_width: request.border_width,
            corner_radii: request.corner_radii,
            shadow_color: multiply_alpha(request.shadow_color, request.opacity),
            shadow_blur: request.shadow_blur,
            elevation: request.elevation,
        }
    }
}

/// Error returned when a canvas material packet cannot be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialDecodeError {
    /// Packet length was not the fixed schema length.
    InvalidLength { expected: usize, actual: usize },
    /// Packet did not begin with the W14 magic bytes.
    InvalidMagic,
    /// Packet version is not supported.
    UnsupportedVersion,
    /// Surface kind is not recognized.
    UnsupportedKind,
    /// Motion policy is not recognized.
    UnsupportedMotionPolicy,
    /// Reserved header bytes were non-zero.
    NonZeroReserved,
    /// Clip presence was neither zero nor one.
    InvalidClipFlag,
    /// Magnification flags contained an unknown bit.
    InvalidMagnificationStyle,
    /// A gradient declared more stops than a request can carry.
    InvalidGradient,
}

/// The stable draw values used by the translucent fallback.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FallbackDraw {
    /// Local bounds.
    pub bounds: [f32; 4],
    /// Transform to use for the fallback.
    pub transform: [f32; 9],
    /// Effective clip.
    pub clip: Option<MaterialClip>,
    /// Ordering key.
    pub z_order: u32,
    /// Tint with opacity applied.
    pub fill_color: [f32; 4],
    /// Border color with opacity applied.
    pub border_color: [f32; 4],
    /// Border width.
    pub border_width: f32,
    /// Corner radii.
    pub corner_radii: [f32; 4],
    /// Shadow color with opacity applied.
    pub shadow_color: [f32; 4],
    /// Shadow blur.
    pub shadow_blur: f32,
    /// Elevation offset.
    pub elevation: f32,
}

/// Capabilities and budgets available to the material stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaterialCapabilities {
    /// Whether the stage can sample a background representation.
    pub backdrop_sampling: bool,
    /// Whether the stage can run Liquid distortion sampling.
    pub distortion_sampling: bool,
    /// Per-surface dimension budget.
    pub max_surface_dimension: u32,
    /// Per-surface RGBA pixel budget.
    pub max_intermediate_pixels: u64,
    /// Maximum requested sample quality.
    pub max_samples: u8,
}

impl Default for MaterialCapabilities {
    fn default() -> Self {
        Self {
            backdrop_sampling: true,
            distortion_sampling: true,
            max_surface_dimension: MAX_INTERMEDIATE_DIMENSION,
            max_intermediate_pixels: MAX_INTERMEDIATE_PIXELS,
            max_samples: 4,
        }
    }
}

/// Bounded render route selected for one material request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialRenderPath {
    /// No pixels need to be emitted for a zero-sized surface.
    Skip,
    /// Use the Cupid material shader, with a bounded intermediate when needed.
    Gpu,
    /// Use the Cupid-owned translucent/border/shadow fallback.
    Fallback,
}

/// Explains why a non-empty material used the Cupid-owned fallback.
///
/// A fallback is an intentional render decision, not a silent loss of the
/// material request. Keeping the reason in the platform-neutral stage plan
/// lets a host report capability and budget decisions without exposing GPU or
/// native visual-effect handles to the widget layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MaterialFallbackReason {
    /// The render stage cannot sample the already-painted backdrop.
    BackdropSamplingUnavailable,
    /// The surface cannot fit within the configured intermediate budget.
    IntermediateBudgetExceeded,
    /// The requested quality is higher than the stage's sample budget.
    SampleQualityUnavailable,
    /// Liquid distortion was requested but the stage cannot sample it.
    DistortionSamplingUnavailable,
}

/// Bounded intermediate texture dimensions for one surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntermediateExtent {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA byte size.
    pub bytes: u64,
}

/// Describes the stage work without allocating GPU resources.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialStagePlan {
    /// Selected path.
    pub path: MaterialRenderPath,
    /// Explicit reason for [`MaterialRenderPath::Fallback`], if selected.
    pub fallback_reason: Option<MaterialFallbackReason>,
    /// Optional bounded intermediate.
    pub intermediate: Option<IntermediateExtent>,
    /// Effective dynamic phase.
    pub phase: f32,
    /// Effective distortion.
    pub distortion_strength: f32,
}

/// Selects a bounded GPU/fallback plan for a request.
pub fn plan_material(
    request: MaterialRequest,
    capabilities: MaterialCapabilities,
) -> MaterialStagePlan {
    let request = request.normalized();
    let inert_lens = request.kind == MaterialKind::Magnification
        && !(request
            .magnification_style
            .contains(MagnificationStyle::UNIFORM)
            && request.magnification != 1.0
            || request
                .magnification_style
                .contains(MagnificationStyle::BULGE)
                && request.bulge_strength > 0.0
            || request
                .magnification_style
                .contains(MagnificationStyle::REFRACTION)
                && request.bevel_radius > 0.0);
    let inert_edge = request.kind == MaterialKind::LightEdge
        && (request.border_width == 0.0
            || request.specular_highlight == 0.0
            || request.border_color[3] == 0.0
            || request.opacity == 0.0);
    let inert_gradient = request.kind == MaterialKind::Gradient
        && (request.gradient.stop_count == 0 || request.opacity == 0.0);
    if request.bounds[2] <= 0.0
        || request.bounds[3] <= 0.0
        || inert_lens
        || inert_edge
        || inert_gradient
    {
        return MaterialStagePlan {
            path: MaterialRenderPath::Skip,
            fallback_reason: None,
            intermediate: None,
            phase: 0.0,
            distortion_strength: 0.0,
        };
    }

    if !request.kind.samples_backdrop() {
        return MaterialStagePlan {
            path: MaterialRenderPath::Gpu,
            fallback_reason: None,
            intermediate: None,
            phase: 0.0,
            distortion_strength: 0.0,
        };
    }

    let intermediate = bounded_intermediate_extent(request, capabilities);
    let fallback_reason = if !capabilities.backdrop_sampling {
        Some(MaterialFallbackReason::BackdropSamplingUnavailable)
    } else if intermediate.is_none() {
        Some(MaterialFallbackReason::IntermediateBudgetExceeded)
    } else if request.quality > capabilities.max_samples {
        Some(MaterialFallbackReason::SampleQualityUnavailable)
    } else if request.kind == MaterialKind::Liquid
        && request.distortion_strength > 0.0
        && !capabilities.distortion_sampling
    {
        Some(MaterialFallbackReason::DistortionSamplingUnavailable)
    } else {
        None
    };
    let path = fallback_reason.map_or(MaterialRenderPath::Gpu, |_| MaterialRenderPath::Fallback);
    MaterialStagePlan {
        path,
        fallback_reason,
        intermediate: if path == MaterialRenderPath::Gpu {
            intermediate
        } else {
            None
        },
        phase: if path == MaterialRenderPath::Gpu {
            request.effective_phase()
        } else {
            0.0
        },
        distortion_strength: if path == MaterialRenderPath::Gpu {
            request.distortion_strength
        } else {
            0.0
        },
    }
}

/// Computes a bounded intermediate allocation.
/// BackdropFilter and Magnification use full-resolution textures and the
/// supplied hardware dimension limit. Glass and Liquid retain the
/// downsampled 2,048-pixel limit.
pub fn bounded_intermediate_extent(
    request: MaterialRequest,
    capabilities: MaterialCapabilities,
) -> Option<IntermediateExtent> {
    let request = request.normalized();
    if !request.kind.samples_backdrop() {
        return None;
    }
    let full_resolution = matches!(
        request.kind,
        MaterialKind::BackdropFilter | MaterialKind::Magnification
    );
    let max_dimension = if full_resolution {
        capabilities.max_surface_dimension
    } else {
        capabilities
            .max_surface_dimension
            .min(MAX_INTERMEDIATE_DIMENSION)
    };
    if max_dimension == 0 {
        return None;
    }
    let downsample = if full_resolution {
        1.0
    } else {
        match request.quality {
            1 => 4.0,
            2 => 2.0,
            _ => 1.0,
        }
    };
    let width = (request.bounds[2] / downsample).ceil().max(1.0);
    let height = (request.bounds[3] / downsample).ceil().max(1.0);
    if !width.is_finite()
        || !height.is_finite()
        || width > max_dimension as f32
        || height > max_dimension as f32
    {
        return None;
    }
    let width = width as u32;
    let height = height as u32;
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    if pixels
        > capabilities
            .max_intermediate_pixels
            .min(MAX_INTERMEDIATE_PIXELS)
    {
        return None;
    }
    Some(IntermediateExtent {
        width,
        height,
        bytes: pixels.checked_mul(4)?,
    })
}

/// Sorts requests by stable z-order while preserving equal-key order.
pub fn order_requests(requests: &mut [MaterialRequest]) {
    requests.sort_by_key(|request| request.z_order);
}

/// Source for the material shader used by Cupid's registered material stage.
pub struct MaterialShader;

impl MaterialShader {
    /// Returns the bundled shader source without exposing a GPU handle.
    #[inline]
    pub fn source() -> &'static str {
        crate::backend::BuiltinShader::Material.wgsl_str()
    }
}

struct PacketReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> PacketReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn bytes<const N: usize>(&mut self) -> Result<&'a [u8; N], MaterialDecodeError> {
        let end = self
            .position
            .checked_add(N)
            .ok_or(MaterialDecodeError::InvalidLength {
                expected: MATERIAL_PACKET_LEN,
                actual: self.bytes.len(),
            })?;
        let slice =
            self.bytes
                .get(self.position..end)
                .ok_or(MaterialDecodeError::InvalidLength {
                    expected: MATERIAL_PACKET_LEN,
                    actual: self.bytes.len(),
                })?;
        self.position = end;
        slice
            .try_into()
            .map_err(|_| MaterialDecodeError::InvalidLength {
                expected: MATERIAL_PACKET_LEN,
                actual: self.bytes.len(),
            })
    }

    fn u8(&mut self) -> Result<u8, MaterialDecodeError> {
        Ok(self.bytes::<1>()?[0])
    }

    fn u32(&mut self) -> Result<u32, MaterialDecodeError> {
        Ok(u32::from_le_bytes(*self.bytes::<4>()?))
    }

    fn f32(&mut self) -> Result<f32, MaterialDecodeError> {
        Ok(f32::from_bits(u32::from_le_bytes(*self.bytes::<4>()?)))
    }

    fn f32s<const N: usize>(&mut self) -> Result<[f32; N], MaterialDecodeError> {
        let mut values = [0.0; N];
        for value in &mut values {
            *value = self.f32()?;
        }
        Ok(values)
    }
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
        *radius = normalize(*radius, 0.0, 0.0, 1_024.0);
    }
    radii
}

#[inline]
fn multiply_alpha(mut color: [f32; 4], opacity: f32) -> [f32; 4] {
    color[3] = (color[3] * opacity).clamp(0.0, 1.0);
    color
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod gradient_tests;
