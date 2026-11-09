//! What a debug build shows while a tool drives it: a badge saying the app is
//! being driven, and a marker where each injected mouse or finger is.
//!
//! An injected event moves no real cursor and touches no real screen, so without
//! this a screenshot of a driven app shows nothing happening. The overlay is
//! two retained nodes painted above the application and its modals — the badge,
//! which changes when someone attaches or leaves, and the pointers, which change
//! with every injected event and animate while a press ripples out. Like the
//! performance overlay it allocates nothing and draws nothing while nobody is
//! attached, and a debug build is the only one that has it.
//!
//! The mouse is drawn as the cursor icon in the repository's `.assets` folder, a
//! fifth bigger than the file draws it and at 90% opacity, with its tip on the
//! position the driver gave. A finger is a circle, and a press sends a ripple out
//! from either. A mouse that has rested for a moment idles: it sways a few degrees
//! either side, pivoting on its tip, so a driven app that is waiting still looks
//! alive.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use aimer_attribute::position::Vec2d;
use aimer_cupid::draw_cmd::{Color, DrawCommand, Rect, RenderNodeId, RenderPaintSource, RenderTree, RenderTreeError};
use aimer_cupid::svg::{SvgNodeStyleOverride, SvgTransform};
use aimer_platform::attachment::{self, MarkerPhase, SyntheticPointer};
use aimer_utils::AnimInstant;
use aimer_widget::SafeAreaInsets;

/// Gap between the badge and the window's (safe) edge.
const BADGE_MARGIN: f32 = 8.0;
/// The badge is a pill: wider than tall, with fully rounded ends, and holds only
/// an icon.
const BADGE_WIDTH: f32 = 48.0;
const BADGE_HEIGHT: f32 = 24.0;
/// The icon's size in the pill, in logical pixels.
const BADGE_ICON: f32 = 16.0;
/// The pill's fill: a solid indigo, so it reads over a dark page and a light one.
const BADGE_COLOR: (u8, u8, u8) = (73, 75, 196);

/// How much bigger than the icon's own 24 units the pointer is drawn.
const ICON_SCALE: f32 = 0.95;
/// The icon's viewBox is 24 units square. Its arrow has a rounded tip: the path
/// itself reaches (3.75, 3.66), and the white outline around it adds about half
/// a unit more, so the visible tip — the cursor's hotspot — is at about (3.3, 3.3)
/// of that box. Retune it if the outline of the file changes.
const ICON_VIEWBOX: f32 = 24.0;
const ICON_TIP_UNITS: f32 = 3.3;
/// The pointer icon's size, and how far its tip is from the icon's top-left
/// corner, in logical pixels.
const ICON_SIZE: f32 = ICON_VIEWBOX * ICON_SCALE;
pub(crate) const ICON_TIP: f32 = ICON_TIP_UNITS * ICON_SCALE;
/// How opaque the pointer is.
const POINTER_OPACITY: f32 = 1.0;

/// How long a mouse rests before it idles, and how long one sway takes, to one
/// side and back to the other and home.
const IDLE_AFTER: Duration = Duration::from_millis(700);
const IDLE_PERIOD: Duration = Duration::from_millis(2800);
/// How often to repaint an idle pointer: enough for a slow sway, not a frame loop.
const IDLE_FRAME: Duration = Duration::from_millis(33);
/// How far the idle pointer leans to either side, in degrees: slight.
const SWAY_DEGREES: f32 = 4.0;
/// How far from a marker's position anything of it is drawn, which is the half
/// width of the node that holds it. Covers the icon at the far end of a sway and
/// the widest ripple.
const FOOTPRINT: f32 = 40.0;

const FINGER_RADIUS: f32 = 14.0;
const HALO_RADIUS: f32 = 10.0;
const RIPPLE_FROM: f32 = 10.0;
const RIPPLE_TO: f32 = 28.0;
/// How long a press ripples out.
const RIPPLE: Duration = Duration::from_millis(450);
/// How long a lifted finger takes to fade.
const FADE: Duration = Duration::from_millis(250);
/// How often to repaint while something animates.
const FRAME: Duration = Duration::from_millis(8);
/// How long the badge and the pointers take to fade in when a driver attaches
/// and to fade out when it leaves.
pub(crate) const PRESENCE_FADE: Duration = Duration::from_millis(300);

const ACCENT: (u8, u8, u8) = (59, 130, 246);

/// Where the badge goes: the top-right corner, inside the safe area.
fn badge_rect(width: f32, insets: SafeAreaInsets) -> Rect {
    // A window too narrow for it keeps the badge's left edge visible.
    let x = (width - insets.right - BADGE_MARGIN - BADGE_WIDTH).max(0.0);
    Rect::new(x, insets.top + BADGE_MARGIN, BADGE_WIDTH, BADGE_HEIGHT)
}

/// One injected mouse or finger.
#[derive(Clone, Copy, Debug)]
struct Marker {
    touch: bool,
    id: u64,
    pos: Vec2d,
    down: bool,
    pressed_at: Option<AnimInstant>,
    released_at: Option<AnimInstant>,
    /// When it last moved, pressed or released: what it has been idle since.
    active_at: AnimInstant,
}

impl Marker {
    /// How far through its ripple a press is, or `None` when none is playing.
    fn ripple(&self, now: AnimInstant) -> Option<f32> {
        let elapsed = now.duration_since(self.pressed_at?);
        (elapsed < RIPPLE).then(|| elapsed.as_secs_f32() / RIPPLE.as_secs_f32())
    }

    /// How opaque a lifted finger still is, or `None` once it has faded.
    fn fade(&self, now: AnimInstant) -> Option<f32> {
        let elapsed = now.duration_since(self.released_at?);
        (elapsed < FADE).then(|| 1.0 - elapsed.as_secs_f32() / FADE.as_secs_f32())
    }

    fn animating(&self, now: AnimInstant) -> bool {
        self.ripple(now).is_some() || (self.touch && !self.down && self.fade(now).is_some())
    }

    /// How far through its sway an idle mouse is, from 0 to 1, or `None` when it
    /// is not idling: a finger on the glass and a held button are not at rest,
    /// and a mouse that has only just stopped has not yet begun.
    fn idle_phase(&self, now: AnimInstant) -> Option<f32> {
        if self.touch || self.down {
            return None;
        }
        let idle = now.duration_since(self.active_at).checked_sub(IDLE_AFTER)?;
        Some((idle.as_secs_f32() / IDLE_PERIOD.as_secs_f32()).fract())
    }

    /// When this marker next needs a frame, if ever.
    fn wake_in(&self, now: AnimInstant) -> Option<Duration> {
        if self.animating(now) {
            return Some(FRAME);
        }
        if self.touch || self.down {
            return None;
        }
        let rested = now.duration_since(self.active_at);
        // Once, for when idling is due to begin; then at the idle pace.
        Some(if rested >= IDLE_AFTER { IDLE_FRAME } else { IDLE_AFTER - rested })
    }
}

/// How visible a layer of the overlay is: it fades toward shown or hidden over
/// [`PRESENCE_FADE`], starting from however visible it was when the target
/// changed, so a driver that leaves and returns mid-fade does not make it jump.
#[derive(Clone, Copy, Debug, Default)]
struct Presence {
    shown: bool,
    /// How visible it was when `shown` last changed.
    from: f32,
    changed_at: Option<AnimInstant>,
}

impl Presence {
    /// Aims at shown or hidden. Aiming where it already aims changes nothing.
    fn set(&mut self, shown: bool, now: AnimInstant) {
        if shown != self.shown {
            self.from = self.alpha(now);
            self.shown = shown;
            self.changed_at = Some(now);
        }
    }

    /// How opaque it is, from 0 to 1, eased at both ends.
    fn alpha(&self, now: AnimInstant) -> f32 {
        let target = if self.shown { 1.0 } else { 0.0 };
        let Some(since) = self.changed_at else {
            return target;
        };
        let t = (now.duration_since(since).as_secs_f32() / PRESENCE_FADE.as_secs_f32()).min(1.0);
        let eased = t * t * (3.0 - 2.0 * t);
        self.from + (target - self.from) * eased
    }

    /// Whether it is still on its way to where it aims.
    fn settling(&self, now: AnimInstant) -> bool {
        self.changed_at.is_some_and(|since| now.duration_since(since) < PRESENCE_FADE)
    }
}

/// A retained render node and whether it currently shows anything.
#[derive(Default)]
struct Retained {
    node: Option<RenderNodeId>,
    painted: bool,
}

impl Retained {
    /// The node, created or recreated when a tree synchronization removed it.
    fn node(&mut self, tree: &RenderTree, bounds: Rect, window: Rect) -> Result<RenderNodeId, RenderTreeError> {
        let exists = self.node.is_some_and(|node| tree.element_bounds(node).is_ok());
        let node = match self.node.filter(|_| exists) {
            Some(node) => node,
            None => {
                let node = tree.add_root(bounds)?;
                tree.set_paint_source(node, RenderPaintSource::LocalV2)?;
                self.node = Some(node);
                node
            }
        };
        tree.set_bounds(node, bounds)?;
        tree.set_clip(node, Some(window))?;
        Ok(node)
    }

    /// Empties the node and damages its footprint once, restoring the
    /// application's pixels underneath.
    fn release(&mut self, tree: &RenderTree) -> Result<(), RenderTreeError> {
        if self.painted
            && let Some(node) = self.node
            && let Ok(context) = tree.context(node)
        {
            context.begin_recording()?.commit(Vec::new())?;
            tree.set_bounds(node, Rect::new(0.0, 0.0, 0.0, 0.0))?;
        }
        self.painted = false;
        Ok(())
    }
}

/// The attachment badge and the injected pointers, owned by the UI thread.
#[derive(Default)]
pub(crate) struct AttachOverlay {
    names: Vec<String>,
    /// The platform's generation when `names` was read.
    generation: Option<u64>,
    markers: Vec<Marker>,
    badge: Retained,
    pointers: Retained,
    badge_dirty: bool,
    pointers_dirty: bool,
    badge_presence: Presence,
    pointer_presence: Presence,
    /// How opaque each layer was when it was last recorded, as bits: a layer is
    /// repainted when this is no longer how opaque it should be.
    badge_alpha: Option<u32>,
    pointers_alpha: Option<u32>,
    /// Nobody is attached any more, and the pointers they left are fading out.
    /// Events that arrive late are ignored, and the pointers are forgotten once
    /// they are gone.
    detaching: bool,
    /// Width, height, scale and safe area the recorded commands were laid out for.
    geometry: Option<(u32, u32, u32, [u32; 4])>,
}

impl AttachOverlay {
    /// Reads who is attached from the platform, and reports whether that changed.
    pub(crate) fn refresh_names(&mut self) -> bool {
        let generation = attachment::generation();
        if self.generation == Some(generation) {
            return false;
        }
        self.generation = Some(generation);
        let before = self.names.len();
        let changed = {
            let names = attachment::agents();
            let changed = names != self.names;
            self.set_names(names);
            changed
        };
        changed || before != self.names.len()
    }

    fn set_names(&mut self, names: Vec<String>) {
        if names.is_empty() {
            // Whoever drove the pointers is gone, and the pointers go with them: they
            // fade out, and are forgotten once they have.
            if !self.names.is_empty() || !self.markers.is_empty() {
                self.detaching = true;
            }
            self.pointers_dirty = true;
        } else {
            self.detaching = false;
        }
        if names != self.names {
            self.names = names;
            self.badge_dirty = true;
        }
    }

    /// Records where an injected pointer is and what it just did.
    pub(crate) fn note_pointer(&mut self, pointer: SyntheticPointer, now: AnimInstant) {
        if self.detaching {
            return;
        }
        self.pointers_dirty = true;
        if pointer.phase == MarkerPhase::Left {
            // A mouse leaves on its own; a cancelled touch takes every finger.
            self.markers.retain(|m| m.touch != pointer.touch);
            self.pointer_presence.set(!self.markers.is_empty(), now);
            return;
        }
        let index = match self.markers.iter().position(|m| m.touch == pointer.touch && m.id == pointer.id) {
            Some(index) => index,
            None => {
                self.markers.push(Marker {
                    touch: pointer.touch,
                    id: pointer.id,
                    pos: pointer.pos,
                    down: false,
                    pressed_at: None,
                    released_at: None,
                    active_at: now,
                });
                self.markers.len() - 1
            }
        };
        // The first pointer fades in; one that is already there is not disturbed.
        self.pointer_presence.set(true, now);
        let marker = &mut self.markers[index];
        marker.pos = pointer.pos;
        marker.active_at = now;
        match pointer.phase {
            MarkerPhase::Pressed => {
                marker.down = true;
                marker.pressed_at = Some(now);
                marker.released_at = None;
            }
            MarkerPhase::Released => {
                marker.down = false;
                marker.released_at = Some(now);
            }
            MarkerPhase::Moved | MarkerPhase::Left => {}
        }
    }

    /// When to wake an idle app to keep a ripple, a fading finger or an idling
    /// mouse moving, or `None` when nothing does. A mouse that has just stopped
    /// asks once, for when its idle animation is due to begin; one that is idling
    /// asks at a gentle 30 Hz; a held button or a finger on the glass asks for
    /// nothing.
    pub(crate) fn wake_in(&self, now: AnimInstant) -> Option<Duration> {
        let markers = self.markers.iter().filter_map(|m| m.wake_in(now)).min();
        if self.badge_presence.settling(now) || self.pointer_presence.settling(now) {
            // Fading in or out: a frame at the animation's pace.
            return Some(markers.map_or(FRAME, |wake| wake.min(FRAME)));
        }
        markers
    }

    /// Records what is to be shown into `tree`, after the application and its
    /// modals have been synchronized into it.
    ///
    /// A surface with a zero size or a non-finite or non-positive `scale` is
    /// skipped without creating a node.
    ///
    /// # Errors
    ///
    /// Returns the render tree's error when a retained node cannot be updated.
    pub(crate) fn prepare(
        &mut self,
        tree: &RenderTree,
        width: u32,
        height: u32,
        scale: f32,
        insets: SafeAreaInsets,
        now: AnimInstant,
    ) -> Result<(), RenderTreeError> {
        if !scale.is_finite() || scale <= 0.0 || width == 0 || height == 0 {
            return Ok(());
        }
        let (logical_width, logical_height) = (width as f32 / scale, height as f32 / scale);
        if !logical_width.is_finite() || !logical_height.is_finite() {
            return Ok(());
        }
        // A finger that has lifted and faded is forgotten.
        self.markers.retain(|m| !(m.touch && !m.down && m.released_at.is_some() && m.fade(now).is_none()));

        let geometry = (
            width,
            height,
            scale.to_bits(),
            [insets.left.to_bits(), insets.top.to_bits(), insets.right.to_bits(), insets.bottom.to_bits()],
        );
        if self.geometry != Some(geometry) {
            self.geometry = Some(geometry);
            self.badge_dirty = true;
            self.pointers_dirty = true;
        }
        let window = Rect::new(0.0, 0.0, logical_width, logical_height);
        self.badge_presence.set(!self.names.is_empty(), now);
        self.pointer_presence.set(!self.markers.is_empty() && !self.detaching, now);
        // Pointers whose driver has gone are forgotten once they have faded out.
        if self.detaching && !self.markers.is_empty() && self.pointer_presence.alpha(now) <= 0.0 {
            self.markers.clear();
        }
        self.prepare_badge(tree, window, insets, now)?;
        self.prepare_pointers(tree, window, now)
    }

    fn prepare_badge(&mut self, tree: &RenderTree, window: Rect, insets: SafeAreaInsets, now: AnimInstant) -> Result<(), RenderTreeError> {
        let alpha = self.badge_presence.alpha(now);
        // Nobody attached and nothing left of the fade: take the badge away.
        if self.names.is_empty() && alpha <= 0.0 {
            self.badge_alpha = None;
            return self.badge.release(tree);
        }
        let missing = !self.badge.node.is_some_and(|node| tree.element_bounds(node).is_ok());
        if !self.badge_dirty && !missing && self.badge_alpha == Some(alpha.to_bits()) {
            return Ok(());
        }
        let rect = badge_rect(window.width, insets);
        // Commands are local to their node's origin, and these are in window
        // coordinates, so the node starts at the window's origin like the
        // performance overlay's. A node hugging the badge would draw it offset
        // by its own position, off the window.
        let node = self.badge.node(tree, window, window)?;
        let (r, g, b) = BADGE_COLOR;
        let commands = vec![
            circle_or_pill(rect, rect.height / 2.0, Color::rgba8(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8), None),
            DrawCommand::Svg {
                scene: badge_scene().clone(),
                destination: Rect::new(
                    rect.x + (rect.width - BADGE_ICON) / 2.0,
                    rect.y + (rect.height - BADGE_ICON) / 2.0,
                    BADGE_ICON,
                    BADGE_ICON,
                ),
                overrides: badge_overrides(alpha),
            },
        ];
        tree.context(node)?.begin_recording()?.commit(commands)?;
        self.badge.painted = true;
        self.badge_dirty = false;
        self.badge_alpha = Some(alpha.to_bits());
        Ok(())
    }

    fn prepare_pointers(&mut self, tree: &RenderTree, window: Rect, now: AnimInstant) -> Result<(), RenderTreeError> {
        // Every marker's footprint, kept inside the window: the node holds just
        // them, so what repaints when a pointer moves or breathes is a small
        // square and not the window.
        let bounds = self
            .markers
            .iter()
            .filter_map(|m| footprint(m.pos, window))
            .reduce(Rect::union);
        let Some(bounds) = bounds else {
            self.pointers_dirty = false;
            self.pointers_alpha = None;
            return self.pointers.release(tree);
        };
        let alpha = self.pointer_presence.alpha(now);
        let animating = self.markers.iter().any(|m| m.animating(now) || m.idle_phase(now).is_some())
            || self.pointers_alpha != Some(alpha.to_bits());
        let missing = !self.pointers.node.is_some_and(|node| tree.element_bounds(node).is_ok());
        if !self.pointers_dirty && !animating && !missing {
            return Ok(());
        }
        let node = self.pointers.node(tree, bounds, window)?;
        // Commands are local to the node, so everything is drawn relative to its
        // top-left corner; a marker that moves with the node records the same
        // commands, and only the node's bounds say it moved.
        let origin = Vec2d { x: bounds.x, y: bounds.y };
        let mut commands = Vec::with_capacity(self.markers.len() * 4);
        for marker in &self.markers {
            marker.record(&mut commands, now, origin, alpha);
        }
        tree.context(node)?.begin_recording()?.commit(commands)?;
        self.pointers.painted = true;
        self.pointers_dirty = false;
        self.pointers_alpha = Some(alpha.to_bits());
        Ok(())
    }

    #[cfg(test)]
    fn markers(&self) -> &[Marker] {
        &self.markers
    }

    #[cfg(test)]
    fn badge_node(&self) -> Option<RenderNodeId> {
        self.badge.node
    }

    #[cfg(test)]
    fn pointer_node(&self) -> Option<RenderNodeId> {
        self.pointers.node
    }
}

impl Marker {
    /// Appends what draws this marker, relative to `origin` (the corner of the
    /// node that holds it): its halo or body, any ripple, and for the mouse the
    /// cursor icon on top.
    ///
    /// `fade` is how visible the whole layer is, from 0 to 1, as a driver attaches
    /// or leaves: everything drawn is that much more transparent.
    fn record(&self, commands: &mut Vec<DrawCommand>, now: AnimInstant, origin: Vec2d, fade: f32) {
        let (r, g, b) = ACCENT;
        let accent = |alpha: f32| Color::rgba8(r, g, b, (alpha.clamp(0.0, 1.0) * fade.clamp(0.0, 1.0) * 255.0) as u8);
        let at = Vec2d { x: self.pos.x - origin.x, y: self.pos.y - origin.y };
        if let Some(t) = self.ripple(now) {
            let radius = RIPPLE_FROM + (RIPPLE_TO - RIPPLE_FROM) * t;
            commands.push(disc(at, radius, Color::transparent(), Some((2.0, accent(0.7 * (1.0 - t))))));
        }
        if self.touch {
            let alpha = if self.down { 1.0 } else { self.fade(now).unwrap_or(0.0) };
            commands.push(disc(at, FINGER_RADIUS, accent(0.3 * alpha), Some((2.0, accent(0.9 * alpha)))));
            return;
        }
        if self.down {
            commands.push(disc(at, HALO_RADIUS, accent(0.35), None));
        }
        // Idling: the arrow leans to one side and then the other, pivoting on its
        // tip so the tip stays on the position. It starts and ends upright, so the
        // loop has no seam.
        let overrides = match (self.idle_phase(now), fade < 1.0) {
            (Some(phase), _) => swayed_overrides(SWAY_DEGREES.to_radians() * (2.0 * std::f32::consts::PI * phase).sin(), fade),
            // Fully visible and upright, the common case: the overrides are built once.
            (None, false) => pointer_overrides().clone(),
            (None, true) => overrides_with(None, POINTER_OPACITY * fade),
        };
        commands.push(DrawCommand::Svg {
            scene: pointer_scene().clone(),
            destination: Rect::new(at.x - ICON_TIP, at.y - ICON_TIP, ICON_SIZE, ICON_SIZE),
            overrides,
        });
    }
}

/// Where a marker at `pos` is drawn, kept inside `window`, or `None` when none of
/// it is in the window.
fn footprint(pos: Vec2d, window: Rect) -> Option<Rect> {
    let (left, top) = ((pos.x - FOOTPRINT).max(0.0), (pos.y - FOOTPRINT).max(0.0));
    let (right, bottom) = ((pos.x + FOOTPRINT).min(window.width), (pos.y + FOOTPRINT).min(window.height));
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// A circle of `radius` around `center`, optionally with a ring of `(width, color)`.
fn disc(center: Vec2d, radius: f32, fill: Color, ring: Option<(f32, Color)>) -> DrawCommand {
    let rect = Rect::new(center.x - radius, center.y - radius, radius * 2.0, radius * 2.0);
    circle_or_pill(rect, radius, fill, ring)
}

/// A rounded rectangle with one radius on every corner.
fn circle_or_pill(rect: Rect, radius: f32, fill: Color, ring: Option<(f32, Color)>) -> DrawCommand {
    let (width, color) = ring.unwrap_or((0.0, Color::transparent()));
    DrawCommand::FillRect {
        rect,
        color: fill,
        border_radius: [radius; 4],
        border_width: [width; 4],
        border_color: color,
        outline_width: [0.0; 4],
        outline_color: Color::transparent(),
    }
}

/// The cursor icon, from the repository's `.assets` folder.
///
/// The file opens with a DOCTYPE, which the SVG parser rejects, so it is parsed
/// from its `<svg` element on; the file itself is left as it was given.
const POINTER_SVG: &str = {
    let full = include_str!("../../../.assets/cursor-alt-svgrepo-com.svg");
    // `str::find` is not const, so the DOCTYPE is skipped by hand.
    let bytes = full.as_bytes();
    let mut at = 0;
    while at + 4 <= bytes.len() && !(bytes[at] == b'<' && bytes[at + 1] == b's' && bytes[at + 2] == b'v' && bytes[at + 3] == b'g') {
        at += 1;
    }
    // SAFETY-free: `at` is the index of an ASCII `<`, so it is a char boundary.
    full.split_at(at).1
};

/// The badge's icon: Lucide's `square-mouse-pointer`.
///
/// Lucide draws in `currentColor`, which a retained scene has no text colour to
/// take, so the badge's white is put in its place.
const BADGE_SVG: &str = include_str!("../../../content/lucide/icons/square-mouse-pointer.svg");

/// The icon's markup with its strokes in white.
fn badge_markup() -> String {
    BADGE_SVG.replace("currentColor", "#ffffff")
}

/// The retained scene of the badge's icon, parsed on first use.
///
/// Parsed from the icon's file and not through `aimer_lucide`, which would pull
/// an HTTP client into every build of this crate.
pub(crate) fn badge_scene() -> &'static Arc<aimer_cupid::svg::SvgScene> {
    static SCENE: OnceLock<Arc<aimer_cupid::svg::SvgScene>> = OnceLock::new();
    SCENE.get_or_init(|| {
        Arc::new(aimer_cupid::svg::parse_svg(&badge_markup()).expect("the bundled badge icon parses"))
    })
}

/// The retained scene of [`POINTER_SVG`], parsed on first use.
fn pointer_scene() -> &'static Arc<aimer_cupid::svg::SvgScene> {
    static SCENE: OnceLock<Arc<aimer_cupid::svg::SvgScene>> = OnceLock::new();
    SCENE.get_or_init(|| {
        Arc::new(aimer_cupid::svg::parse_svg(POINTER_SVG).expect("the bundled cursor icon parses"))
    })
}

/// What is overridden on each part of the arrow that is drawn: only its opacity,
/// because the icon has its own colours — a dark body with a white outline, which
/// reads over a dark page and a light one alike.
fn pointer_overrides() -> &'static Arc<[SvgNodeStyleOverride]> {
    static OVERRIDES: OnceLock<Arc<[SvgNodeStyleOverride]>> = OnceLock::new();
    OVERRIDES.get_or_init(|| overrides_with(None, POINTER_OPACITY))
}

/// The same, with the arrow turned by `angle` radians about its tip.
fn swayed_overrides(angle: f32, fade: f32) -> Arc<[SvgNodeStyleOverride]> {
    // The tip, in the units of the scene's viewport: the icon's own 24-unit box is
    // scaled to the width the file declares.
    let pivot = ICON_TIP_UNITS * pointer_scene().viewport.width / ICON_VIEWBOX;
    let (sin, cos) = angle.sin_cos();
    overrides_with(
        Some(SvgTransform {
            sx: cos,
            ky: sin,
            kx: -sin,
            sy: cos,
            // `turn(p) + t = p` for `p` at the pivot, so the pivot does not move.
            tx: pivot - cos * pivot + sin * pivot,
            ty: pivot - sin * pivot - cos * pivot,
        }),
        POINTER_OPACITY * fade,
    )
}

fn overrides_with(turn: Option<SvgTransform>, opacity: f32) -> Arc<[SvgNodeStyleOverride]> {
    pointer_scene()
        .nodes
        .iter()
        .filter(|node| node.geometry.is_some() && !node.is_definition)
        .map(|node| SvgNodeStyleOverride {
            node_id: node.node_id,
            fill: None,
            stroke: None,
            opacity: Some(opacity),
            // A node's own transform carries the viewBox scaling, so the turn is
            // applied after it, in the viewport's units.
            transform: turn.map(|turn| turn.mul(node.transform)),
        })
        .collect()
}

/// The badge icon's overrides at `alpha`: none at all when it is fully visible,
/// so the settled badge costs nothing extra.
fn badge_overrides(alpha: f32) -> Arc<[SvgNodeStyleOverride]> {
    if alpha >= 1.0 {
        return Arc::from(Vec::new());
    }
    badge_scene()
        .nodes
        .iter()
        .filter(|node| node.geometry.is_some() && !node.is_definition)
        .map(|node| SvgNodeStyleOverride {
            node_id: node.node_id,
            fill: None,
            stroke: None,
            opacity: Some(alpha.clamp(0.0, 1.0)),
            transform: None,
        })
        .collect()
}

#[cfg(test)]
#[path = "attach_overlay_tests.rs"]
mod tests;
