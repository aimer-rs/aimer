//! Fades drawn without an offscreen pass.
//!
//! A fade (an opacity gradient over a child) is drawn by rendering its alpha and
//! its child into two targets and compositing them, which is several render
//! passes per fade. When the child is only text and the alpha is a linear ramp,
//! the glyphs can take the ramp's alpha where they are drawn, and the fade is no
//! more than the text. This module decides when that is so, and holds what the
//! renderer needs to do it.
//!
//! It is conservative on purpose. Anything it does not recognise is left to the
//! offscreen path, which handles every case: a different kind of content, a ramp
//! that is not linear, text that could be a color glyph, a fade inside another.

use std::collections::{HashMap, HashSet};

use aimer_text_pipeline::AlphaRamp;

use crate::pipeline::material::{
    GradientShape, MATERIAL_PIPELINE_NAME, MAX_GRADIENT_STOPS, MaterialKind, MaterialRequest,
};
use crate::render_list::DrawCommand;

/// How a slice of commands is drawn, when its group is drawn as a ramped fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::renderer) enum SliceRole {
    /// An ordinary slice: drawn as it is.
    Normal,
    /// The fade's own item. It draws nothing: its gradient becomes the ramp the
    /// slices after it are drawn through, scaled by the group's `opacity`.
    RampAlpha { opacity: f32 },
    /// Text drawn through the ramp the last [`SliceRole::RampAlpha`] set.
    RampContent,
}

/// The fades of a frame that are drawn as ramps, and how their items are used.
///
/// Items are told apart by address: the plan's operations live as long as the
/// frame's drawing does, and nothing is added to them.
#[derive(Default)]
pub(in crate::renderer) struct RampedFades {
    /// The elements of the groups that are not made: their begin and end are
    /// skipped, and what is inside them is drawn with the group's parent.
    groups: HashSet<u64>,
    roles: HashMap<usize, SliceRole>,
}

impl RampedFades {
    pub(in crate::renderer) fn clear(&mut self) {
        self.groups.clear();
        self.roles.clear();
    }

    pub(in crate::renderer) fn is_empty(&self) -> bool {
        self.groups.is_empty()
    }

    pub(in crate::renderer) fn insert_group(&mut self, element: u64) {
        self.groups.insert(element);
    }

    pub(in crate::renderer) fn set_role(&mut self, operation: *const (), role: SliceRole) {
        self.roles.insert(operation as usize, role);
    }

    /// Whether the group of `element` is drawn as a ramp, not as a group.
    pub(in crate::renderer) fn skips_group(&self, element: u64) -> bool {
        self.groups.contains(&element)
    }

    pub(in crate::renderer) fn role_of(&self, operation: *const ()) -> SliceRole {
        self.roles
            .get(&(operation as usize))
            .copied()
            .unwrap_or(SliceRole::Normal)
    }
}

/// Whether `request` is a ramp the glyphs can take: a linear gradient with a few
/// stops over a plain rectangle. (A request has a border width by default, but a
/// gradient draws no border.)
///
/// Its own clip is not part of the question. A request that has one asks to be
/// cut to a shape of its own; the clip of the scene around a request is added by
/// the renderer when it resolves it, and is not a reason to refuse it: see
/// [`alpha_item_request`].
pub(super) fn is_plain_linear_ramp(request: &MaterialRequest) -> bool {
    let stops = usize::from(request.gradient.stop_count);
    request.kind == MaterialKind::Gradient
        && request.gradient.shape == GradientShape::Linear
        && (1..=MAX_GRADIENT_STOPS.min(aimer_text_pipeline::MAX_RAMP_STOPS)).contains(&stops)
        && request.corner_radii == [0.0; 4]
}

/// The ramp `request` makes, in target pixels.
///
/// `request` is the one the material pipeline would draw: already resolved to
/// the target, with its bounds and geometry in pixels and its opacity including
/// everything above it. The ramp is what that pipeline's shader evaluates, for
/// the alpha channel only.
pub(super) fn ramp_from_resolved(request: &MaterialRequest) -> Option<AlphaRamp> {
    let request = request.normalized();
    if !is_plain_linear_ramp(&request) {
        return None;
    }
    let [x, y, width, height] = request.bounds;
    let geometry = request.gradient.geometry;
    let stops: Vec<(f32, f32)> = (0..usize::from(request.gradient.stop_count))
        .map(|index| {
            // Packed as little-endian RGBA bytes, so alpha is the top byte.
            let alpha = (request.gradient.colors[index] >> 24) as f32 / 255.0;
            (request.gradient.offsets[index], alpha)
        })
        .collect();
    AlphaRamp::new(
        [x + geometry[0], y + geometry[1], x + geometry[2], y + geometry[3]],
        [x, y, width, height],
        &stops,
    )
    .map(|ramp| ramp.scaled(request.opacity))
}

/// Whether `text` could be drawn with a color glyph from the color atlas, which a
/// ramp does not reach.
///
/// An emoji is the only text that is. What is taken to be outline glyphs is Latin
/// and its relatives below U+2000, the dashes and quotes of General Punctuation,
/// the CJK blocks, Hangul and the full-width forms; anything else is taken to be
/// one.
pub(super) fn text_may_use_color_glyphs(text: &str) -> bool {
    !text.chars().all(|c| {
        let code = c as u32;
        code < 0x2000
            // Dashes, quotes, bullets and the ellipsis: common in titles, and not
            // the emoji the rest of General Punctuation holds.
            || (0x2010..=0x2027).contains(&code)
            || (0x3000..=0x9FFF).contains(&code)
            || (0xAC00..=0xD7AF).contains(&code)
            || (0xFF00..=0xFFEF).contains(&code)
    })
}

/// What a list of commands is, to decide whether a fade over it is a ramp.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ContentScan {
    /// Every command is one a ramped fade can draw.
    pub(super) drawable: bool,
    /// How many of them draw text.
    pub(super) texts: usize,
}

/// Scans the commands of one item of a fade's content: plain text, and the
/// state that goes with it.
pub(super) fn scan_content(commands: &[DrawCommand]) -> ContentScan {
    let mut scan = ContentScan {
        drawable: true,
        texts: 0,
    };
    for command in commands {
        match command {
            DrawCommand::PushTransform { .. }
            | DrawCommand::PopTransform
            | DrawCommand::SetTransform { .. }
            | DrawCommand::PushClip { .. }
            | DrawCommand::PopClip
            | DrawCommand::SetAlpha { .. }
            | DrawCommand::RestoreAlpha
            | DrawCommand::SetItalic { .. }
            | DrawCommand::SetTextLanguage { .. } => {}
            DrawCommand::DrawText {
                text,
                shadow,
                draw_glyphs,
                ..
            } => {
                // A shadow is drawn under the glyphs and would not be faded; text
                // that draws nothing needs no ramp but is not worth the thought.
                if shadow.is_some() || !*draw_glyphs || text_may_use_color_glyphs(text) {
                    scan.drawable = false;
                    return scan;
                }
                scan.texts += 1;
            }
            _ => {
                scan.drawable = false;
                return scan;
            }
        }
    }
    scan
}

/// The material request of a fade's own item, if the item is only a linear ramp.
///
/// The item holds the fade marker, which names no pipeline and draws nothing,
/// and the gradient the child is faded by, framed by the transforms that place it
/// and the state resets the lowering ends every item with. Anything else in it is
/// not a ramp.
pub(super) fn alpha_item_request(commands: &[DrawCommand]) -> Option<MaterialRequest> {
    let mut request = None;
    for command in commands {
        match command {
            // The state the lowering puts around every item, and the clip of the
            // scene around it, such as a scroll view's. That clip cuts the gradient
            // only where it cuts the child's text, which is cut by it the same,
            // so the ramp need not repeat it.
            DrawCommand::PushTransform { .. }
            | DrawCommand::PopTransform
            | DrawCommand::SetTransform { .. }
            | DrawCommand::PushClip { .. }
            | DrawCommand::PopClip
            | DrawCommand::SetAlpha { .. }
            | DrawCommand::RestoreAlpha
            | DrawCommand::SetItalic { .. }
            | DrawCommand::SetTextLanguage { .. } => {}
            DrawCommand::Custom {
                pipeline_name,
                data,
            } if pipeline_name == crate::draw_cmd::FADE_MASK_PIPELINE_NAME => {
                let _ = data;
            }
            DrawCommand::Custom {
                pipeline_name,
                data,
            } if pipeline_name == MATERIAL_PIPELINE_NAME => {
                if request.is_some() {
                    return None;
                }
                let bytes = data.downcast_ref::<Vec<u8>>()?;
                let decoded = MaterialRequest::decode(bytes).ok()?;
                // A payload that carries a clip of its own is cut to a shape the
                // ramp does not have.
                if decoded.clip.is_some() || !is_plain_linear_ramp(&decoded.normalized()) {
                    return None;
                }
                request = Some(decoded);
            }
            _ => return None,
        }
    }
    request
}

#[cfg(test)]
#[path = "ramped_fades_tests.rs"]
mod tests;
