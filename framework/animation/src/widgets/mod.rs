pub mod animated;
pub mod animated_builder;
pub mod animated_paint;
pub mod animated_switcher;
pub mod container_transform;
pub mod expand_transform;
pub mod implicit_animation;
pub mod morph_transition;
pub mod shared_element;
pub mod transition;

pub use animated::{Animated, AnimationEffect};
pub use animated_builder::AnimatedBuilder;
pub use animated_paint::{AnimatedPaint, PaintPresentation, PresentationClip};
pub use animated_switcher::AnimatedSwitcher;
pub use container_transform::{ContainerTransformFrame, ContainerTransformSpec, ElementBounds, StageSpans};
pub use expand_transform::ExpandSpec;
pub use implicit_animation::ImplicitAnimatedBuilder;
pub use morph_transition::{MorphStyle, MorphTransition};
pub use shared_element::{
    SharedContent, SharedElement, SharedElementSnapshot, last_shared_element, last_shared_element_bounds,
    shared_element_bounds,
};
pub use transition::{FadeTransition, RotationTransition, ScaleTransition, SlideTransition};

#[cfg(test)]
pub(crate) mod test_frame_requester {
    use std::cell::Cell;

    thread_local! {
        static REQUESTS: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn install() {
        static INSTALL: std::sync::Once = std::sync::Once::new();
        INSTALL.call_once(|| {
            aimer_events::window::set_redraw_requester(|| {
                REQUESTS.with(|requests| requests.set(requests.get() + 1));
            });
        });
    }

    pub(crate) fn reset() {
        REQUESTS.with(|requests| requests.set(0));
    }

    pub(crate) fn count() -> usize {
        REQUESTS.with(Cell::get)
    }
}

#[cfg(all(test, not(target_arch = "wasm32"), not(feature = "portable-guest")))]
mod builder_tests;
