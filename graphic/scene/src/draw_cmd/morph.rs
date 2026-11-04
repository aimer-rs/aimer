//! The morph marker: a shader transition between a node's two child subtrees.

/// The custom-pipeline name of the morph marker.
///
/// A node whose local list begins with a `DrawCustom` of this name and whose
/// payload decodes as [`MorphParams`] declares that its first child subtree is
/// the *outgoing* picture and its second the *incoming* one. The render plan
/// wraps the node in a morph [`RenderOp::BeginOpacityGroup`], each child in a
/// source group of its own, and the renderer draws the two sources into
/// offscreen targets and blends them with a shader instead of painting them on
/// top of each other.
///
/// A marked node that does not have exactly two children paints as an ordinary
/// node; the marker draws nothing itself, so a renderer that has no morph
/// support simply shows both children.
///
/// [`RenderOp::BeginOpacityGroup`]: super::RenderOp::BeginOpacityGroup
pub const MORPH_PIPELINE_NAME: &str = "aimer.morph";

/// How the outgoing picture turns into the incoming one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MorphStyle {
    /// Pixels switch from outgoing to incoming as a noise threshold sweeps
    /// past them, with a soft edge of [`MorphParams::softness`].
    Dissolve = 0,
    /// A soft edge travels along [`MorphParams::direction`], revealing the
    /// incoming picture behind it.
    Wipe = 1,
    /// Both pictures are displaced along a flowing field while they blend, so
    /// shapes appear to melt into each other.
    Liquid = 2,
    /// The two content boxes are interpolated in position and size, and both
    /// pictures are stretched into the moving box while they cross-fade.
    Bounds = 3,
    /// Both pictures are blurred while they cross-fade: the outgoing one
    /// smudges away as the incoming one comes into focus. Each picture stays
    /// in place, so unlike [`Bounds`](Self::Bounds) nothing is stretched.
    Blur = 4,
    /// The outgoing picture is pushed out along
    /// [`MorphParams::direction`] while the incoming one enters from the
    /// opposite side, both opaque and clipped to the morph, like a page turning
    /// in a navigation bar. Neither fades unless a blur is asked for.
    Slide = 5,
    /// The two pictures' silhouettes melt into one another like drops of
    /// liquid: shapes that grow bud out of their neighbours, join through a
    /// neck, and part when the neck thins, as one pill divides into a pill and a
    /// circle. The colors inside them blur and cross-fade. The reach of the
    /// melting is [`MorphParams::blur`], at its widest mid-way, and
    /// [`MorphParams::softness`] is how soft the edge of the shape is.
    Goo = 6,
}

impl MorphStyle {
    #[inline]
    fn from_wire(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Dissolve),
            1 => Some(Self::Wipe),
            2 => Some(Self::Liquid),
            3 => Some(Self::Bounds),
            4 => Some(Self::Blur),
            5 => Some(Self::Slide),
            6 => Some(Self::Goo),
            _ => None,
        }
    }
}

/// The parameters of one morph frame, carried by the marker's payload.
///
/// Only `progress` normally changes while a morph runs; the rest describe the
/// look of the effect and stay put.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MorphParams {
    /// Which effect blends the two pictures.
    pub style: MorphStyle,
    /// `0.0` shows only the outgoing picture, `1.0` only the incoming one.
    pub progress: f32,
    /// Travel direction of a [`MorphStyle::Wipe`] edge or a [`MorphStyle::Slide`]
    /// push, in screen space (y grows downward). Need not be normalized; a zero
    /// vector means rightward.
    pub direction: [f32; 2],
    /// Width of the soft edge, as a fraction of the morph, in `0.0..=1.0`.
    pub softness: f32,
    /// How far [`MorphStyle::Liquid`] displaces the pictures, as a fraction of
    /// the morph's smaller side.
    pub amplitude: f32,
    /// Spatial frequency of the noise behind [`MorphStyle::Dissolve`] and
    /// [`MorphStyle::Liquid`], in cells across the morph.
    pub frequency: f32,
    /// Offsets the noise so two morphs of the same size need not look alike.
    pub seed: f32,
    /// The blur, in logical pixels, an outgoing picture reaches as it is gone
    /// and an incoming one starts from. Any style honors it; [`MorphStyle::Blur`]
    /// is the one that is about it. `0.0` is sharp.
    pub blur: f32,
}

impl MorphParams {
    /// The encoded payload length in bytes.
    pub const ENCODED_LEN: usize = 36;

    /// The largest blur a payload may ask for, in logical pixels.
    pub const MAX_BLUR: f32 = 128.0;

    /// Creates parameters with the default look of `style`.
    #[inline]
    pub fn new(style: MorphStyle, progress: f32) -> Self {
        Self {
            style,
            progress,
            direction: [1.0, 0.0],
            softness: 0.25,
            amplitude: 0.08,
            frequency: 14.0,
            seed: 0.0,
            blur: 0.0,
        }
    }

    /// Encodes the parameters as the marker payload.
    pub fn encode(&self) -> [u8; Self::ENCODED_LEN] {
        let words = [
            f32::from_bits(self.style as u32),
            self.progress,
            self.direction[0],
            self.direction[1],
            self.softness,
            self.amplitude,
            self.frequency,
            self.seed,
            self.blur,
        ];
        let mut bytes = [0u8; Self::ENCODED_LEN];
        for (chunk, word) in bytes.chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        bytes
    }

    /// Decodes a marker payload, rejecting anything the renderer cannot draw.
    ///
    /// Returns `None` for a payload of the wrong length, an unknown style, a
    /// non-finite field, a progress or softness outside `0.0..=1.0`, or a blur
    /// outside `0.0..=`[`MAX_BLUR`](Self::MAX_BLUR).
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != Self::ENCODED_LEN {
            return None;
        }
        let mut words = [0u32; 9];
        for (word, chunk) in words.iter_mut().zip(bytes.chunks_exact(4)) {
            *word = u32::from_le_bytes(chunk.try_into().ok()?);
        }
        let float = |index: usize| f32::from_bits(words[index]);
        let params = Self {
            style: MorphStyle::from_wire(words[0])?,
            progress: float(1),
            direction: [float(2), float(3)],
            softness: float(4),
            amplitude: float(5),
            frequency: float(6),
            seed: float(7),
            blur: float(8),
        };
        let finite = [
            params.progress,
            params.direction[0],
            params.direction[1],
            params.softness,
            params.amplitude,
            params.frequency,
            params.seed,
            params.blur,
        ]
        .into_iter()
        .all(f32::is_finite);
        (finite
            && (0.0..=1.0).contains(&params.progress)
            && (0.0..=1.0).contains(&params.softness)
            && (0.0..=Self::MAX_BLUR).contains(&params.blur))
        .then_some(params)
    }
}

/// What a group in the plan is for in a morph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MorphGroup {
    /// The morph itself: it owns no pixels, only the two sources inside it.
    Host(MorphParams),
    /// The first child's subtree, drawn into the outgoing picture.
    From,
    /// The second child's subtree, drawn into the incoming picture.
    To,
}
