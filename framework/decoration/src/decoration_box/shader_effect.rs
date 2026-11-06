//! [`ShaderEffectDecoration`], a decoration that draws an application-defined shader.

use std::any::TypeId;
use std::cell::{Cell, RefCell};

use aimer_attribute::{ResolvedSize, Vec2d};
use aimer_canvas::Canvas;
use aimer_cupid::effects::{self, EffectCommand, ShaderEffect};
use aimer_cupid::utilities::Rect;

use super::{Decoration, declared_corner_radii};

/// Draws a [`ShaderEffect`] over the decorated box.
///
/// This puts an effect where decorations already go: behind a child in a
/// [`DecorationBox`](crate::DecorationBox), over it in a
/// [`ForegroundDecorationBox`](crate::ForegroundDecorationBox), or anywhere in
/// a stack next to shadows, borders and gradients. The shader fills the box;
/// the decoration changes neither layout, child clipping nor paint bounds.
///
/// [`new`](Self::new) registers the effect's shader the first time this effect
/// type is seen on the thread, so there is no separate registration call. A
/// shader that breaks the [`ShaderEffect`] contract is reported once on
/// standard error, and the decoration then paints nothing.
///
/// The box has square corners unless a [`BorderRadius`](crate::BorderRadius) or
/// [`BoxDecoration`](crate::BoxDecoration) radius precedes the effect in the
/// same stack, in which case the effect is clipped to those per-corner radii,
/// bounded to half the shorter side. A radius that follows it does not reach
/// back.
///
/// # Animation
///
/// The decoration keeps the effect value it was built with and paints it as it
/// is. To animate, build a new decoration from an animation builder each frame
/// so the effect carries the new time. The retained paint list is reused while
/// [`revision`](Decoration::revision) is unchanged, so an effect that mutates
/// behind a decoration that is *not* rebuilt must change it with
/// [`with_revision`](Self::with_revision) and request a redraw itself.
///
/// An effect that samples the backdrop sees what is painted before it. A
/// foreground effect therefore also sees the child.
///
/// # Examples
///
/// ```
/// use aimer_container::SizedBox;
/// use aimer_decoration::{BorderRadius, DecorationBox, ShaderEffectDecoration, Decoration};
/// use aimer_cupid::effects::{ParamWriter, ShaderEffect};
///
/// struct Tint([f32; 4]);
/// impl ShaderEffect for Tint {
///     const NAME: &'static str = "doc_tint";
///     const SOURCE: &'static str = include_str!("../../tests/shaders/tint.aimer");
///     fn write_params(&self, params: &mut ParamWriter) {
///         params.vec4(self.0);
///     }
/// }
///
/// let card = DecorationBox::new()
///     .decoration(vec![
///         BorderRadius::from(16).notype(),
///         ShaderEffectDecoration::new(Tint([0.2, 0.4, 0.9, 1.0])).notype(),
///     ])
///     .child(SizedBox::new().width(240).height(160));
/// ```
pub struct ShaderEffectDecoration<E: ShaderEffect> {
    effect: E,
    revision: u64,
    /// Set after the first failed encode so a broken effect logs once, not per frame.
    reported: Cell<bool>,
}

impl<E: ShaderEffect> ShaderEffectDecoration<E> {
    /// Creates a decoration that draws `effect` over the whole box.
    ///
    /// Registers `E`'s shader on first use; see the type documentation.
    #[inline]
    pub fn new(effect: E) -> Self {
        register_once::<E>();
        Self {
            effect,
            revision: 0,
            reported: Cell::new(false),
        }
    }

    /// Sets the value [`Decoration::revision`] reports. The default is `0`.
    ///
    /// Change it whenever the effect's parameters change without the
    /// decoration being rebuilt, and request a redraw.
    #[inline]
    pub fn with_revision(mut self, revision: u64) -> Self {
        self.revision = revision;
        self
    }

    /// Returns the effect this decoration draws.
    #[inline]
    pub fn effect(&self) -> &E {
        &self.effect
    }
}

impl<E: ShaderEffect> Decoration for ShaderEffectDecoration<E> {
    fn paint(&self, canvas: &Canvas, origin: Vec2d, size: ResolvedSize, scale: f32) {
        // `!(x > 0.0)` is false for NaN, so this also rejects nonfinite sizes.
        if !(size.width > 0.0 && size.height > 0.0 && size.width.is_finite() && size.height.is_finite())
            || !(scale > 0.0 && scale.is_finite())
        {
            return;
        }
        // The rectangle is in the element's logical space; the renderer maps it
        // through the element transform, so `scale` is not applied here.
        let rect = [origin.x, origin.y, size.width, size.height];
        let command = match EffectCommand::new(&self.effect, rect) {
            Ok(command) => command,
            Err(error) => {
                if !self.reported.replace(true) {
                    eprintln!("shader effect decoration: {error}");
                }
                return;
            }
        };
        // Effects clip to the active clip, not to the declared shape, so a
        // rounded box needs an explicit one. Half the shorter side is as round
        // as a box can get.
        let radii = declared_corner_radii(canvas, size.width.min(size.height) / 2.0);
        let rounded = radii.iter().any(|&radius| radius > 0.0);
        if rounded {
            canvas.push_clip(Rect::new(origin.x, origin.y, size.width, size.height), radii);
        }
        canvas.draw_custom(command.name, command.payload);
        if rounded {
            canvas.pop_clip();
        }
    }

    #[inline]
    fn revision(&self) -> u64 {
        self.revision
    }
}

thread_local! {
    /// Effect types this thread has already tried to register. A decoration is
    /// rebuilt every frame while animating, and `register` takes a process-wide
    /// lock, so the common case must not reach it.
    static REGISTERED: RefCell<Vec<TypeId>> = const { RefCell::new(Vec::new()) };
}

fn register_once<E: ShaderEffect>() {
    let id = TypeId::of::<E>();
    let first = REGISTERED.with(|seen| {
        let mut seen = seen.borrow_mut();
        if seen.contains(&id) {
            false
        } else {
            seen.push(id);
            true
        }
    });
    if first {
        // `register` is idempotent for the same shader, so another thread
        // doing the same is harmless.
        if let Err(error) = effects::register::<E>() {
            eprintln!("shader effect decoration: {error}");
        }
    }
}
