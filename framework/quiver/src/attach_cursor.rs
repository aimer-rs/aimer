//! What the virtual mouse looks like: the cursor icon the app asked for.
//!
//! A driven app's real cursor never moves, so the attach overlay draws one of its
//! own. A person's cursor changes shape with what it hovers — a hand over a link,
//! an I-beam over text, a double arrow on a splitter — and the app already says
//! so by asking its window for a [`CursorIcon`]. The overlay follows that request
//! ([`aimer_utils::cursor::requested_icon`]) and draws the [`Look`] closest to
//! it, so a screenshot of a driven app shows what a person would be seeing.
//!
//! Every look is a [`Glyph`]: an SVG scene, the point of it that is the hotspot,
//! and how far it is turned. The two cursor files in `curosrs/` keep their own
//! drawing — the arrow its own colours, the resize arrows recoloured to match
//! them — and the rest are Lucide icons, each stroked twice so a white halo
//! surrounds the dark line and it reads over a dark page as well as a light one.
//! Diagonal and horizontal resize cursors are the vertical one turned.

use std::sync::{Arc, OnceLock};

use aimer_cupid::svg::{SvgColor, SvgNodeStyleOverride, SvgScene, SvgTransform};
use aimer_utils::cursor::CursorIcon;

use crate::attach_overlay::{ICON_TIP_UNITS, ICON_VIEWBOX, POINTER_OPACITY};

/// The drawing of the arrow, the one the mouse is without a better reason.
pub(crate) const ARROW_SVG: &str = svg_element(include_str!("../curosrs/cursor-alt-svgrepo-com.svg"));
/// Two triangles pointing up and down, drawn with a black outline and no fill.
const RESIZE_SVG: &str = svg_element(include_str!("../curosrs/resize-up-down-svgrepo-com.svg"));

const TEXT_CURSOR: &str = include_str!("../../../content/lucide/icons/text-cursor.svg");
const POINTER: &str = include_str!("../../../content/lucide/icons/pointer.svg");
const HAND: &str = include_str!("../../../content/lucide/icons/hand.svg");
const MOVE: &str = include_str!("../../../content/lucide/icons/move.svg");
const CROSSHAIR: &str = include_str!("../../../content/lucide/icons/crosshair.svg");
const LOADER: &str = include_str!("../../../content/lucide/icons/loader.svg");
const BAN: &str = include_str!("../../../content/lucide/icons/ban.svg");
const ZOOM_IN: &str = include_str!("../../../content/lucide/icons/zoom-in.svg");
const ZOOM_OUT: &str = include_str!("../../../content/lucide/icons/zoom-out.svg");

/// The dark of the arrow's body, which the other glyphs use for their line.
const INK: &str = "#1d1c1c";
const HALO: &str = "#ffffff";
/// How wide the halo is against the 2-unit line it surrounds.
const HALO_WIDTH: &str = "4.6";

/// What a file's markup is from its `<svg` element on.
///
/// The cursor files open with a DOCTYPE or an XML declaration, which the SVG
/// parser rejects; they are parsed from the element on and the files are left as
/// they were given. `str::find` is not const, so the element is found by hand.
const fn svg_element(full: &'static str) -> &'static str {
    let bytes = full.as_bytes();
    let mut at = 0;
    while at + 4 <= bytes.len() && !(bytes[at] == b'<' && bytes[at + 1] == b's' && bytes[at + 2] == b'v' && bytes[at + 3] == b'g') {
        at += 1;
    }
    // `at` is the index of an ASCII `<`, so it is a char boundary.
    full.split_at(at).1
}

/// One way the virtual mouse can be drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Look {
    #[default]
    Arrow,
    IBeam,
    PointingHand,
    OpenHand,
    ResizeVertical,
    ResizeHorizontal,
    /// Along the line from the top right to the bottom left.
    ResizeRising,
    /// Along the line from the top left to the bottom right.
    ResizeFalling,
    Move,
    Crosshair,
    Busy,
    Forbidden,
    ZoomIn,
    ZoomOut,
}

const LOOKS: usize = 14;

impl Look {
    /// The look closest to a cursor `icon`. An icon without a drawing of its own —
    /// the context menu, a cell, a drag-and-drop alias — is the arrow.
    pub(crate) fn of(icon: CursorIcon) -> Self {
        use CursorIcon as C;
        match icon {
            C::Pointer => Self::PointingHand,
            C::Text | C::VerticalText => Self::IBeam,
            C::Grab | C::Grabbing => Self::OpenHand,
            C::NsResize | C::NResize | C::SResize | C::RowResize => Self::ResizeVertical,
            C::EwResize | C::EResize | C::WResize | C::ColResize => Self::ResizeHorizontal,
            C::NeswResize | C::NeResize | C::SwResize => Self::ResizeRising,
            C::NwseResize | C::NwResize | C::SeResize => Self::ResizeFalling,
            C::Move | C::AllScroll | C::AllResize => Self::Move,
            C::Crosshair => Self::Crosshair,
            C::Wait | C::Progress => Self::Busy,
            C::NotAllowed | C::NoDrop => Self::Forbidden,
            C::ZoomIn => Self::ZoomIn,
            C::ZoomOut => Self::ZoomOut,
            _ => Self::Arrow,
        }
    }

    /// The drawing of this look, built on first use.
    pub(crate) fn glyph(self) -> &'static Glyph {
        static GLYPHS: [OnceLock<Glyph>; LOOKS] = [const { OnceLock::new() }; LOOKS];
        GLYPHS[self as usize].get_or_init(|| Glyph::build(self))
    }
}

/// A look's drawing: what to paint, where its hotspot is and how it is turned.
pub(crate) struct Glyph {
    pub(crate) scene: Arc<SvgScene>,
    /// The hotspot, in the icon's own 24-unit box.
    pub(crate) hotspot: (f32, f32),
    /// Turned this many radians about the hotspot, clockwise.
    pub(crate) turn: f32,
    /// A fill and a stroke to paint over the file's own, for a drawing that has
    /// none that reads on any page.
    recolor: Option<(SvgColor, SvgColor)>,
    /// The overrides when the glyph is fully visible and not swaying, the common
    /// case, built once.
    settled: Arc<[SvgNodeStyleOverride]>,
}

impl Glyph {
    fn build(look: Look) -> Self {
        let centre = (ICON_VIEWBOX / 2.0, ICON_VIEWBOX / 2.0);
        let quarter = std::f32::consts::FRAC_PI_2;
        let ink = SvgColor { r: 0.114, g: 0.110, b: 0.110, a: 1.0 };
        let white = SvgColor { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
        let (markup, hotspot, turn, recolor) = match look {
            // The arrow's tip is its hotspot. Retune it if the file changes.
            Look::Arrow => (ARROW_SVG.to_owned(), (ICON_TIP_UNITS, ICON_TIP_UNITS), 0.0, None),
            Look::IBeam => (haloed(TEXT_CURSOR), centre, 0.0, None),
            // The tip of the pointing finger.
            Look::PointingHand => (haloed(POINTER), (8.0, 2.5), 0.0, None),
            Look::OpenHand => (haloed(HAND), centre, 0.0, None),
            Look::ResizeVertical => (RESIZE_SVG.to_owned(), centre, 0.0, Some((ink, white))),
            Look::ResizeHorizontal => (RESIZE_SVG.to_owned(), centre, quarter, Some((ink, white))),
            Look::ResizeRising => (RESIZE_SVG.to_owned(), centre, quarter / 2.0, Some((ink, white))),
            Look::ResizeFalling => (RESIZE_SVG.to_owned(), centre, -quarter / 2.0, Some((ink, white))),
            Look::Move => (haloed(MOVE), centre, 0.0, None),
            Look::Crosshair => (haloed(CROSSHAIR), centre, 0.0, None),
            Look::Busy => (haloed(LOADER), centre, 0.0, None),
            Look::Forbidden => (haloed(BAN), centre, 0.0, None),
            // The middle of the lens, not of the icon, which is the handle too.
            Look::ZoomIn => (haloed(ZOOM_IN), (11.0, 11.0), 0.0, None),
            Look::ZoomOut => (haloed(ZOOM_OUT), (11.0, 11.0), 0.0, None),
        };
        let scene = Arc::new(aimer_cupid::svg::parse_svg(&markup).expect("a bundled cursor icon parses"));
        let mut glyph = Self { scene, hotspot, turn, recolor, settled: Arc::from(Vec::new()) };
        glyph.settled = glyph.styled(0.0, POINTER_OPACITY);
        glyph
    }

    /// What is overridden on each drawn part, for a glyph leaning `sway` radians
    /// off upright (`None` for upright) and `fade` visible, from 0 to 1.
    pub(crate) fn overrides(&self, sway: Option<f32>, fade: f32) -> Arc<[SvgNodeStyleOverride]> {
        if sway.is_none() && fade >= 1.0 {
            return self.settled.clone();
        }
        self.styled(sway.unwrap_or(0.0), POINTER_OPACITY * fade)
    }

    fn styled(&self, sway: f32, opacity: f32) -> Arc<[SvgNodeStyleOverride]> {
        let angle = self.turn + sway;
        let turn = (angle != 0.0).then(|| {
            // The hotspot, in the units of the scene's viewport: the icon's own
            // 24-unit box is scaled to the width the file declares.
            let unit = self.scene.viewport.width / ICON_VIEWBOX;
            rotation_about(angle, self.hotspot.0 * unit, self.hotspot.1 * unit)
        });
        self.scene
            .nodes
            .iter()
            .filter(|node| node.geometry.is_some() && !node.is_definition)
            .map(|node| SvgNodeStyleOverride {
                node_id: node.node_id,
                fill: self.recolor.map(|(fill, _)| Some(fill)),
                stroke: self.recolor.map(|(_, stroke)| Some(stroke)),
                opacity: Some(opacity),
                // A node's own transform carries the viewBox scaling, so the turn is
                // applied after it, in the viewport's units.
                transform: turn.map(|turn| turn.mul(node.transform)),
            })
            .collect()
    }
}

/// A turn of `angle` radians (clockwise on screen) that leaves `(px, py)` where it is.
fn rotation_about(angle: f32, px: f32, py: f32) -> SvgTransform {
    let (sin, cos) = angle.sin_cos();
    SvgTransform { sx: cos, ky: sin, kx: -sin, sy: cos, tx: px - cos * px + sin * py, ty: py - sin * px - cos * py }
}

/// A Lucide icon's markup with its line in [`INK`] and a wider white line under it.
///
/// Lucide draws in `currentColor`, which a retained scene has no text colour to
/// take. The icon's shapes are put in twice, the first stroked in white and wider,
/// which is a halo; the root's own attributes (no fill, round caps) apply to both.
fn haloed(svg: &str) -> String {
    let svg = svg.replace("currentColor", INK);
    let open_end = svg.find('>').expect("an svg element") + 1;
    let close = svg.rfind("</svg>").expect("a closed svg element");
    let (open, body) = (&svg[..open_end], &svg[open_end..close]);
    format!("{open}<g stroke=\"{HALO}\" stroke-width=\"{HALO_WIDTH}\">{body}</g>{body}</svg>")
}

#[cfg(test)]
#[path = "attach_cursor_tests.rs"]
mod tests;
