//! The painted mark of a control: a checkbox box, a radio circle, a switch
//! track and thumb, or a select chevron.
//!
//! Each mark is one leaf element that records a handful of rounded rectangles
//! in its own coordinates. Check marks, dashes, and chevrons are thin rounded
//! rectangles under a rotation, so no path is tessellated.

use aimer_attribute::size::Size;
use aimer_canvas::Canvas;
use aimer_cupid::utilities::{Color as V2Color, Mat3, Rect};
use aimer_style::ThemeTokens;
use aimer_widget::base::{BuildContext, Color};
use aimer_widget::{
    AnyElement, Drawable, Element, EventElement, EventTreeRole, LayoutElement, PortableWidget, Rebuildable,
    VisitorElement, Widget,
};

use crate::interaction::Flags;
use crate::theme::with_state_layer;

/// The side length of the square mark before density scaling.
const MARK_SIDE: f32 = 20.0;

/// What a mark is painted as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mark {
    /// A rounded box, optionally holding a check or a dash.
    Box { check: Glyph },
    /// A circle, optionally holding a dot.
    Radio { selected: bool },
    /// A track with a sliding thumb.
    Switch { on: bool },
    /// A down chevron, or an up chevron while open.
    Chevron { open: bool },
    /// A bare check mark with no box, marking the selected row of a list.
    Tick,
}

/// The glyph inside a checkbox box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    None,
    Check,
    Dash,
}

/// A control's mark, ready to be placed in a row.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Indicator {
    mark: Mark,
    side: f32,
    fill: Color,
    border: Color,
    glyph: Color,
    ring: Option<(Color, f32)>,
    corner: f32,
}

impl Indicator {
    /// Resolves the colors of `mark` for the given interaction state.
    pub(crate) fn new(
        tokens: &ThemeTokens,
        mark: Mark,
        flags: Flags,
        disabled: bool,
        errored: bool,
    ) -> Self {
        let colors = &tokens.colors;
        let active = match mark {
            Mark::Box { check } => check != Glyph::None,
            Mark::Radio { selected } => selected,
            Mark::Switch { on } => on,
            Mark::Chevron { .. } | Mark::Tick => false,
        };
        let filled = active && !matches!(mark, Mark::Radio { .. });
        let accent = if errored { colors.error } else { colors.primary };
        let fill = if filled {
            with_state_layer(tokens, accent, flags, disabled)
        } else {
            with_state_layer(tokens, colors.surface, flags, disabled)
        };
        let border = match (disabled, active, errored) {
            (_, _, true) => colors.error,
            (_, true, _) => accent,
            _ => colors.outline,
        };
        let border = with_state_layer(tokens, border, Flags::default(), disabled);
        let glyph = match mark {
            Mark::Box { .. } => colors.on_primary,
            Mark::Radio { .. } => accent,
            Mark::Switch { on: true } => colors.on_primary,
            Mark::Switch { on: false } => colors.outline,
            Mark::Chevron { .. } => colors.on_surface,
            Mark::Tick => colors.primary,
        };
        let glyph = with_state_layer(tokens, glyph, Flags::default(), disabled);
        let ring = flags
            .focus_visible()
            .then_some((tokens.focus.ring_color, tokens.focus.ring_width));
        let corner = match mark {
            Mark::Box { .. } => tokens.shape.small,
            Mark::Radio { .. } | Mark::Switch { .. } | Mark::Chevron { .. } | Mark::Tick => {
                tokens.shape.pill
            }
        };
        Self {
            mark,
            // Keep the fixed target size on the leaf so layout and paint
            // resolve the same dimensions under any parent.
            side: tokens.density.target_size(MARK_SIDE),
            fill,
            border,
            glyph,
            ring,
            corner,
        }
    }

    /// Overrides the side length, for marks that sit inside a row of their own
    /// height rather than being a control's touch target.
    #[inline]
    pub(crate) fn sized(mut self, side: f32) -> Self {
        self.side = side;
        self
    }

    /// The width of the mark; a switch track is wider than it is tall.
    #[inline]
    fn width(&self) -> f32 {
        match self.mark {
            Mark::Switch { .. } => self.side * 1.8,
            _ => self.side,
        }
    }
}

impl PortableWidget for Indicator {}

impl Widget for Indicator {
    fn to_element(self, _ctx: &BuildContext) -> AnyElement {
        RawIndicator(self).boxed()
    }

    fn debug_name(&self) -> &'static str {
        "SelectionIndicator"
    }
}

struct RawIndicator(Indicator);

impl VisitorElement for RawIndicator {
    fn debug_name(&self) -> &'static str {
        "SelectionIndicator"
    }
}

impl EventElement for RawIndicator {
    fn event_tree_role(&self) -> EventTreeRole {
        EventTreeRole::Transparent
    }
}

impl Rebuildable for RawIndicator {}

impl LayoutElement for RawIndicator {
    fn size(&self) -> Option<Size> {
        Some(Size::new(self.0.width(), self.0.side))
    }

    fn is_layout_stable(&self) -> bool {
        true
    }
}

fn v2(color: Color) -> V2Color {
    let (red, green, blue, alpha) = color.to_rgba();
    V2Color::rgba8(red, green, blue, alpha)
}

/// Records a round-capped segment from `from` to `to`.
fn stroke_segment(canvas: &Canvas, from: (f32, f32), to: (f32, f32), thickness: f32, color: V2Color) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt();
    canvas.push_transform(Mat3::translate(from.0, from.1).mul(&Mat3::rotate(dy.atan2(dx))));
    canvas.fill_rect_styled(
        Rect::new(-thickness / 2.0, -thickness / 2.0, length + thickness, thickness),
        color,
        [thickness / 2.0; 4],
        [0.0; 4],
        V2Color::transparent(),
        [0.0; 4],
        V2Color::transparent(),
    );
    canvas.pop_transform();
}

/// Records a filled rounded rectangle, with an optional border and focus ring.
#[allow(clippy::too_many_arguments)]
fn rounded(
    canvas: &Canvas,
    rect: Rect,
    fill: V2Color,
    radius: f32,
    border: Option<(V2Color, f32)>,
    ring: Option<(V2Color, f32)>,
) {
    let (border_color, border_width) = border.unwrap_or((V2Color::transparent(), 0.0));
    let (ring_color, ring_width) = ring.unwrap_or((V2Color::transparent(), 0.0));
    canvas.fill_rect_styled(
        rect,
        fill,
        [radius; 4],
        [border_width; 4],
        border_color,
        [ring_width; 4],
        ring_color,
    );
}

impl Drawable for RawIndicator {
    fn can_paint_local_v2(&self, ctx: &BuildContext) -> bool {
        let indicator = &self.0;
        ctx.scale.is_finite()
            && ctx.scale > 0.0
            && indicator.side.is_finite()
            && indicator.side > 0.0
            && indicator.side <= 1_000_000.0
            && indicator.corner.is_finite()
            && indicator.corner >= 0.0
    }

    fn paint_local_v2(&self, ctx: &BuildContext) {
        let canvas = Canvas::of(ctx);
        let Indicator {
            mark,
            side,
            fill,
            border,
            glyph,
            ring,
            corner,
        } = self.0;
        let width = self.0.width();
        let glyph = v2(glyph);
        let ring = ring.map(|(color, width)| (v2(color), width));
        let body = Rect::new(0.0, 0.0, width, side);
        // A corner radius of half the shorter side is a full pill or circle.
        let radius = corner.min(side / 2.0);
        match mark {
            Mark::Box { check } => {
                rounded(&canvas, body, v2(fill), radius, Some((v2(border), 2.0)), ring);
                let thickness = side * 0.12;
                match check {
                    Glyph::Check => {
                        let knee = (side * 0.43, side * 0.70);
                        stroke_segment(&canvas, (side * 0.22, side * 0.50), knee, thickness, glyph);
                        stroke_segment(&canvas, knee, (side * 0.78, side * 0.30), thickness, glyph);
                    }
                    Glyph::Dash => stroke_segment(
                        &canvas,
                        (side * 0.28, side * 0.5),
                        (side * 0.72, side * 0.5),
                        thickness,
                        glyph,
                    ),
                    Glyph::None => {}
                }
            }
            Mark::Radio { selected } => {
                rounded(&canvas, body, v2(fill), radius, Some((v2(border), 2.0)), ring);
                if selected {
                    let dot = side * 0.5;
                    let inset = (side - dot) / 2.0;
                    rounded(
                        &canvas,
                        Rect::new(inset, inset, dot, dot),
                        glyph,
                        dot / 2.0,
                        None,
                        None,
                    );
                }
            }
            Mark::Switch { on } => {
                rounded(&canvas, body, v2(fill), radius, Some((v2(border), 2.0)), ring);
                let thumb = side * if on { 0.6 } else { 0.5 };
                let inset = (side - thumb) / 2.0;
                let x = if on { width - inset - thumb } else { inset };
                rounded(
                    &canvas,
                    Rect::new(x, inset, thumb, thumb),
                    glyph,
                    thumb / 2.0,
                    None,
                    None,
                );
            }
            Mark::Tick => {
                let thickness = side * 0.12;
                let knee = (side * 0.43, side * 0.70);
                stroke_segment(&canvas, (side * 0.22, side * 0.50), knee, thickness, glyph);
                stroke_segment(&canvas, knee, (side * 0.78, side * 0.30), thickness, glyph);
            }
            Mark::Chevron { open } => {
                let (top, bottom) = if open { (0.62, 0.38) } else { (0.38, 0.62) };
                let tip = (side * 0.5, side * bottom);
                let thickness = side * 0.1;
                stroke_segment(&canvas, (side * 0.3, side * top), tip, thickness, glyph);
                stroke_segment(&canvas, tip, (side * 0.7, side * top), thickness, glyph);
            }
        }
        canvas.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_switch_track_is_wider_than_it_is_tall() {
        let tokens = ThemeTokens::light();
        let switch = Indicator::new(&tokens, Mark::Switch { on: true }, Flags::default(), false, false);
        let checkbox = Indicator::new(
            &tokens,
            Mark::Box { check: Glyph::None },
            Flags::default(),
            false,
            false,
        );
        assert!(switch.width() > switch.side);
        assert_eq!(checkbox.width(), checkbox.side);
    }

    #[test]
    fn the_focus_ring_follows_keyboard_focus_only() {
        let tokens = ThemeTokens::light();
        let mark = Mark::Radio { selected: false };
        let pointer_focus = Flags {
            focused: true,
            ..Flags::default()
        };
        let keyboard_focus = Flags {
            focused: true,
            keyboard: true,
            ..Flags::default()
        };
        assert!(Indicator::new(&tokens, mark, pointer_focus, false, false).ring.is_none());
        assert!(Indicator::new(&tokens, mark, keyboard_focus, false, false).ring.is_some());
    }

    #[test]
    fn an_error_recolors_the_border_of_an_unselected_mark() {
        let tokens = ThemeTokens::light();
        let mark = Mark::Box { check: Glyph::None };
        let plain = Indicator::new(&tokens, mark, Flags::default(), false, false);
        let errored = Indicator::new(&tokens, mark, Flags::default(), false, true);
        assert_ne!(plain.border, errored.border);
    }

    #[test]
    fn rotation_is_applied_before_translation() {
        // The segment helper relies on `translate * rotate` rotating about the
        // segment's start point.
        let matrix = Mat3::translate(10.0, 20.0).mul(&Mat3::rotate(std::f32::consts::FRAC_PI_2));
        let (x, y) = matrix.transform_point(1.0, 0.0);
        assert!((x - 10.0).abs() < 1e-4 && (y - 21.0).abs() < 1e-4, "got ({x}, {y})");
    }
}
