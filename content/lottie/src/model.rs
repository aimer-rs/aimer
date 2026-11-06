//! The typed Lottie document: a composition of layers, each with a transform
//! and, for shape layers, a tree of shapes. Every animatable value is a
//! [`Property`].
//!
//! Frame numbers are in the composition's own frames (`Composition::frame_rate`
//! per second). Nothing here evaluates an animation; this is the parsed form.

/// A 2D vector or point, `[x, y]`.
pub type Vec2 = [f32; 2];
/// A color with straight (non-premultiplied) alpha, each channel in `0..=1`.
pub type Rgba = [f32; 4];

/// A parsed Lottie animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    /// Canvas width in pixels.
    pub width: f32,
    /// Canvas height in pixels.
    pub height: f32,
    /// Frames per second. Always finite and positive.
    pub frame_rate: f32,
    /// First frame that is played.
    pub in_frame: f32,
    /// Frame at which playback ends.
    pub out_frame: f32,
    /// Layers in file order. Lottie draws the first layer on top.
    pub layers: Vec<Layer>,
    /// Everything in the file that was skipped or replaced by a default.
    pub diagnostics: Vec<Diagnostic>,
}

impl Composition {
    /// Playback length in seconds.
    #[inline]
    pub fn duration_secs(&self) -> f32 {
        (self.out_frame - self.in_frame) / self.frame_rate
    }
}

/// One layer of a composition.
#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    /// The layer's `ind`, the number other layers use to name it as a parent.
    pub index: Option<i32>,
    /// `ind` of the parent layer, whose transform this layer inherits.
    pub parent: Option<i32>,
    pub name: String,
    /// First frame the layer is visible.
    pub in_frame: f32,
    /// Frame at which the layer stops being visible.
    pub out_frame: f32,
    /// Frame offset applied to the layer's own timeline.
    pub start_frame: f32,
    /// A layer the author hid. Kept so that parent links stay valid.
    pub hidden: bool,
    pub transform: Transform,
    pub kind: LayerKind,
    /// How the layer is cut by its track matte, when it has one that exists.
    pub matte: Option<MatteMode>,
    /// The layer exists to be another layer's matte (`td`) and is not drawn
    /// itself.
    pub is_matte: bool,
    /// Position in [`Composition::layers`] of the parent layer, resolved once
    /// at load time. Never part of a cycle.
    pub(crate) parent_slot: Option<usize>,
    /// Position in [`Composition::layers`] of this layer's matte, resolved once
    /// at load time; `Some` exactly when [`Layer::matte`] is.
    pub(crate) matte_slot: Option<usize>,
    /// The `tp` of the file: `ind` of the matte layer, when it names one.
    pub(crate) matte_index: Option<i32>,
}

/// How a layer is cut by its track matte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatteMode {
    /// The layer shows where the matte has coverage.
    Alpha,
    /// The layer shows where the matte has none.
    AlphaInverted,
    /// The layer shows where the matte is bright.
    Luma,
    /// The layer shows where the matte is dark.
    LumaInverted,
}

/// What a layer contains.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LayerKind {
    /// Draws nothing; exists to be a parent.
    Null,
    /// Vector shapes, in file order.
    Shape(Vec<Shape>),
    /// A layer type outside the supported subset (precomp, image, text, ...),
    /// kept with its Lottie `ty` so that parent links stay valid.
    Unsupported(i64),
}

/// Position, either as one 2D property or split into independent axes.
#[derive(Debug, Clone, PartialEq)]
pub enum Position {
    Joint(Property<Vec2>),
    Split { x: Property<f32>, y: Property<f32> },
}

/// A layer or group transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Transform {
    /// Point the scale and rotation pivot around, in the layer's own space.
    pub anchor: Property<Vec2>,
    pub position: Position,
    /// Percent; `[100, 100]` is unscaled.
    pub scale: Property<Vec2>,
    /// Degrees, clockwise.
    pub rotation: Property<f32>,
    /// Percent, `0..=100`.
    pub opacity: Property<f32>,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            anchor: Property::Static([0.0, 0.0]),
            position: Position::Joint(Property::Static([0.0, 0.0])),
            scale: Property::Static([100.0, 100.0]),
            rotation: Property::Static(0.0),
            opacity: Property::Static(100.0),
        }
    }
}

/// A value that is either fixed or keyframed over time.
#[derive(Debug, Clone, PartialEq)]
pub enum Property<T> {
    Static(T),
    /// Keyframes in file order; never empty.
    Animated(Vec<Keyframe<T>>),
}

/// One keyframe of an animated [`Property`].
#[derive(Debug, Clone, PartialEq)]
pub struct Keyframe<T> {
    /// Frame at which the property has `value`.
    pub time: f32,
    pub value: T,
    /// How the property moves from this keyframe to the next one.
    pub interpolation: Interpolation,
}

/// Easing of the segment that starts at a keyframe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Interpolation {
    Linear,
    /// The value stays until the next keyframe, then jumps.
    Hold,
    /// A cubic bezier easing curve from `(0, 0)` to `(1, 1)` whose control
    /// points are `ease_out` and `ease_in`, in Lottie's `o` and `i` order.
    Bezier { ease_out: Vec2, ease_in: Vec2 },
}

/// A shape-layer item, in file order.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Shape {
    Group(Group),
    Rect(Rect),
    Ellipse(Ellipse),
    Path(PathShape),
    Fill(Fill),
    Stroke(Stroke),
    Trim(Trim),
}

/// A list of shapes drawn together under one transform.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub items: Vec<Shape>,
    pub transform: Transform,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rect {
    /// Center of the rectangle.
    pub position: Property<Vec2>,
    pub size: Property<Vec2>,
    /// Corner radius.
    pub roundness: Property<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ellipse {
    /// Center of the ellipse.
    pub position: Property<Vec2>,
    pub size: Property<Vec2>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PathShape {
    pub path: Property<Bezier>,
}

/// A cubic bezier path. Tangents are stored as in the file: relative to their
/// vertex.
#[derive(Debug, Clone, PartialEq)]
pub struct Bezier {
    pub closed: bool,
    pub vertices: Vec<Vec2>,
    pub in_tangents: Vec<Vec2>,
    pub out_tangents: Vec<Vec2>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    NonZero,
    EvenOdd,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Fill {
    pub color: Property<Rgba>,
    /// Percent, `0..=100`.
    pub opacity: Property<f32>,
    pub rule: FillRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stroke {
    pub color: Property<Rgba>,
    /// Percent, `0..=100`.
    pub opacity: Property<f32>,
    pub width: Property<f32>,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
}

/// Draws only part of the paths above it.
#[derive(Debug, Clone, PartialEq)]
pub struct Trim {
    /// Percent of the path length where the visible part starts.
    pub start: Property<f32>,
    /// Percent of the path length where the visible part ends.
    pub end: Property<f32>,
    /// Degrees; `360` shifts the visible part once around a closed path.
    pub offset: Property<f32>,
    /// `true` trims every path by its own length, as After Effects does by
    /// default; `false` treats all the paths as one long path, in order.
    pub simultaneous: bool,
}

/// Something in the file that Aimer skipped or replaced by a default, instead
/// of failing the whole document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Diagnostic {
    /// A layer type, shape type or feature outside the supported subset.
    Unsupported { layer: String, feature: String },
    /// An After Effects expression, replaced by the property's plain value.
    Expression { layer: String },
    /// A value that could not be read, replaced by a default.
    Malformed { layer: String, what: &'static str },
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { layer, feature } => {
                write!(f, "layer `{layer}`: unsupported {feature}")
            }
            Self::Expression { layer } => {
                write!(f, "layer `{layer}`: expression ignored, using the plain value")
            }
            Self::Malformed { layer, what } => {
                write!(f, "layer `{layer}`: malformed {what}, using a default")
            }
        }
    }
}
