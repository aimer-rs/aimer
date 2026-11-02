//! Platform monotonic clock.
//!
//! Native targets use [`std::time::Instant`] directly. `std::time::Instant`
//! panics on `wasm32-unknown-unknown`, so there the clock reads the host's
//! `performance.now()` and stores the result as the time since the time origin.

#[cfg(not(target_arch = "wasm32"))]
pub(crate) use std::time::Instant;

#[cfg(target_arch = "wasm32")]
pub(crate) use browser::Instant;

#[cfg(target_arch = "wasm32")]
mod browser {
    use std::ops::{Add, AddAssign, Sub, SubAssign};
    use std::time::Duration;

    #[wasm_bindgen::prelude::wasm_bindgen]
    unsafe extern "C" {
        /// `performance.now()`, available on the window and in workers.
        #[wasm_bindgen::prelude::wasm_bindgen(js_namespace = performance, js_name = now)]
        safe fn performance_now() -> f64;
    }

    /// Time since the host's time origin; mirrors the `std::time::Instant` API
    /// that [`crate::Instant`] relies on.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub(crate) struct Instant(Duration);

    impl Instant {
        #[inline]
        pub(crate) fn now() -> Self {
            let millis = performance_now();
            // `performance.now()` is never negative, but never trust the host with a panic.
            Self(Duration::from_secs_f64(if millis > 0.0 { millis / 1000.0 } else { 0.0 }))
        }

        #[inline]
        pub(crate) fn saturating_duration_since(self, earlier: Self) -> Duration {
            self.0.saturating_sub(earlier.0)
        }

        #[inline]
        pub(crate) fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
            self.0.checked_sub(earlier.0)
        }

        #[inline]
        pub(crate) fn checked_add(self, duration: Duration) -> Option<Self> {
            self.0.checked_add(duration).map(Self)
        }

        #[inline]
        pub(crate) fn checked_sub(self, duration: Duration) -> Option<Self> {
            self.0.checked_sub(duration).map(Self)
        }
    }

    impl Add<Duration> for Instant {
        type Output = Instant;

        #[inline]
        fn add(self, rhs: Duration) -> Instant {
            Instant(self.0 + rhs)
        }
    }

    impl AddAssign<Duration> for Instant {
        #[inline]
        fn add_assign(&mut self, rhs: Duration) {
            self.0 += rhs;
        }
    }

    impl Sub<Duration> for Instant {
        type Output = Instant;

        #[inline]
        fn sub(self, rhs: Duration) -> Instant {
            Instant(self.0 - rhs)
        }
    }

    impl SubAssign<Duration> for Instant {
        #[inline]
        fn sub_assign(&mut self, rhs: Duration) {
            self.0 -= rhs;
        }
    }
}
