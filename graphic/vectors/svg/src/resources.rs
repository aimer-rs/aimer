use std::sync::Arc;

use super::{SvgColor, SvgGradient, SvgTransform};

/// Coordinate interpretation shared by SVG definitions that use a bounding box.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgResourceUnits {
    /// Coordinates are relative to the referencing element's bounding box.
    ObjectBoundingBox,
    /// Coordinates use the referencing element's current user space.
    UserSpaceOnUse,
}

/// A local SVG clip path and the retained geometry that defines it.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgClipPath {
    pub id: Arc<str>,
    pub units: SvgResourceUnits,
    pub transform: SvgTransform,
    pub nodes: Arc<[u32]>,
}

/// Whether a mask uses alpha or luminance to compute coverage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgMaskType {
    Alpha,
    Luminance,
}

/// A local SVG mask definition.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgMask {
    pub id: Arc<str>,
    pub units: SvgResourceUnits,
    pub content_units: SvgResourceUnits,
    pub mask_type: SvgMaskType,
    /// `x`, `y`, `width`, and `height` in the coordinate space selected by `units`.
    pub region: [f32; 4],
    pub nodes: Arc<[u32]>,
}

/// A local SVG pattern definition.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgPattern {
    pub id: Arc<str>,
    pub units: SvgResourceUnits,
    pub content_units: SvgResourceUnits,
    /// `x`, `y`, `width`, and `height` in the coordinate space selected by `units`.
    pub tile: [f32; 4],
    pub transform: SvgTransform,
    pub nodes: Arc<[u32]>,
}

/// Inputs named by an SVG filter primitive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SvgFilterInput {
    SourceGraphic,
    SourceAlpha,
    Previous,
    Named(Arc<str>),
}

/// The composite operator used by `feComposite`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgCompositeOperator {
    Over,
    In,
    Out,
    Atop,
    Xor,
    Lighter,
    Arithmetic,
}

/// The blend mode used by `feBlend`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SvgBlendMode {
    Normal,
    Multiply,
    Screen,
    Darken,
    Lighten,
}

/// Color-matrix operation used by `feColorMatrix`.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgColorMatrix {
    Matrix(Arc<[f32]>),
    Saturate(f32),
    HueRotate(f32),
    LuminanceToAlpha,
}

/// One retained SVG filter operation.
#[derive(Clone, Debug, PartialEq)]
pub enum SvgFilterPrimitive {
    GaussianBlur {
        input: SvgFilterInput,
        deviation: [f32; 2],
    },
    Offset {
        input: SvgFilterInput,
        dx: f32,
        dy: f32,
    },
    Flood {
        color: SvgColor,
    },
    Composite {
        input: SvgFilterInput,
        input2: SvgFilterInput,
        operator: SvgCompositeOperator,
        coefficients: [f32; 4],
    },
    Blend {
        input: SvgFilterInput,
        input2: SvgFilterInput,
        mode: SvgBlendMode,
    },
    Merge {
        inputs: Arc<[SvgFilterInput]>,
    },
    ColorMatrix {
        input: SvgFilterInput,
        matrix: SvgColorMatrix,
    },
    /// A syntactically valid primitive outside the currently implemented set.
    Unsupported(Arc<str>),
}

/// A local SVG filter graph and its primitive sequence.
#[derive(Clone, Debug, PartialEq)]
pub struct SvgFilter {
    pub id: Arc<str>,
    pub units: SvgResourceUnits,
    pub primitive_units: SvgResourceUnits,
    /// `x`, `y`, `width`, and `height` in the coordinate space selected by `units`.
    pub region: [f32; 4],
    pub primitives: Arc<[SvgFilterPrimitive]>,
}

/// All local definitions retained for an SVG scene.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SvgResourceGraph {
    /// Root `viewBox` mapping already composed into retained definition nodes.
    pub root_transform: SvgTransform,
    pub gradients: Arc<[SvgGradient]>,
    pub clip_paths: Arc<[SvgClipPath]>,
    pub masks: Arc<[SvgMask]>,
    pub filters: Arc<[SvgFilter]>,
    pub patterns: Arc<[SvgPattern]>,
}

impl SvgResourceGraph {
    /// Creates an empty graph for programmatically constructed scenes.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the first clip path with the given local fragment id.
    pub fn clip_path(&self, id: &str) -> Option<&SvgClipPath> {
        self.clip_paths
            .iter()
            .find(|resource| resource.id.as_ref() == id)
    }

    /// Returns the first mask with the given local fragment id.
    pub fn mask(&self, id: &str) -> Option<&SvgMask> {
        self.masks
            .iter()
            .find(|resource| resource.id.as_ref() == id)
    }

    /// Returns the first filter with the given local fragment id.
    pub fn filter(&self, id: &str) -> Option<&SvgFilter> {
        self.filters
            .iter()
            .find(|resource| resource.id.as_ref() == id)
    }

    /// Returns the first pattern with the given local fragment id.
    pub fn pattern(&self, id: &str) -> Option<&SvgPattern> {
        self.patterns
            .iter()
            .find(|resource| resource.id.as_ref() == id)
    }
}

impl SvgResourceUnits {
    pub(crate) fn from_attribute(value: Option<&str>, default: Self) -> Option<Self> {
        match value.unwrap_or(match default {
            Self::ObjectBoundingBox => "objectBoundingBox",
            Self::UserSpaceOnUse => "userSpaceOnUse",
        }) {
            "objectBoundingBox" => Some(Self::ObjectBoundingBox),
            "userSpaceOnUse" => Some(Self::UserSpaceOnUse),
            _ => None,
        }
    }
}
