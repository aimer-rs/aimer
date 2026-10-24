//! Playback controls shared by the animations that own an
//! [`AnimationController`](crate::AnimationController).

/// Expands to the playback methods of a type with a `controller` field.
///
/// The controller is a handle: clones of the animation share one playback
/// state, so a widget and its owner can drive the same animation. Every method
/// forwards to the controller, whose documentation describes the exact timing
/// rules.
macro_rules! controller_playback {
    () => {
        /// Returns the controller that drives this animation.
        ///
        /// Clones of the controller share playback state with this animation.
        pub fn controller(&self) -> &$crate::AnimationController {
            &self.controller
        }

        /// Starts playing forward from the current progress.
        pub fn forward(&self) {
            self.controller.forward();
        }

        /// Arms forward playback, starting the clock on the first tick so
        /// widget construction does not consume part of the duration.
        pub fn forward_from_first_tick(&self) {
            self.controller.forward_from_first_tick();
        }

        /// Starts playing in reverse from the current progress.
        pub fn reverse(&self) {
            self.controller.reverse();
        }

        /// Returns to the start and stops.
        pub fn reset(&self) {
            self.controller.reset();
        }

        /// Returns `true` while the animation is playing in either direction.
        pub fn is_animating(&self) -> bool {
            self.controller.is_animating()
        }

        /// Returns the playback status.
        pub fn status(&self) -> $crate::AnimationStatus {
            self.controller.status()
        }

        /// Sets whether playback restarts when it reaches an end.
        pub fn set_repeat(&self, repeat: bool) {
            self.controller.set_repeat(repeat);
        }

        /// Sets whether repeated playback alternates direction instead of
        /// jumping back to the start. It only matters while repeating.
        pub fn set_auto_reverse(&self, auto_reverse: bool) {
            self.controller.set_auto_reverse(auto_reverse);
        }

        /// Builder form of [`set_repeat`](Self::set_repeat).
        pub fn repeat(self, repeat: bool) -> Self {
            self.controller.set_repeat(repeat);
            self
        }

        /// Builder form of [`set_auto_reverse`](Self::set_auto_reverse).
        pub fn auto_reverse(self, auto_reverse: bool) -> Self {
            self.controller.set_auto_reverse(auto_reverse);
            self
        }

        /// Registers a listener for status changes.
        pub fn add_status_listener(&self, listener: ::std::rc::Rc<dyn $crate::StatusListener>) {
            self.controller.add_status_listener(listener);
        }
    };
}

pub(crate) use controller_playback;
