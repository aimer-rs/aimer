//! Debug-only counters for the retained rebuild walk.
//!
//! These counters answer a different question from draw traversal counts:
//! drawing may visit every retained element while the rebuild walk only visits
//! the paths affected by invalidation. They are thread-local because the
//! widget tree is rebuilt on the UI/render thread and the counters are sampled
//! at the end of that same frame.

#[cfg(feature = "frame-stats")]
use std::cell::Cell;

/// Work observed during one retained-tree rebuild walk.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RebuildStats {
    /// Retained `ElementNode` boundaries entered by the rebuild walk.
    pub visits: u64,
    /// Retained boundaries that returned early because their dirty path was
    /// known not to contain work.
    pub pruned: u64,
    /// Stateful elements whose dirty state was checked.
    pub stateful_checks: u64,
    /// Stateless elements whose dirty state was checked.
    pub stateless_checks: u64,
    /// Stateful elements whose `build` callback actually ran.
    pub stateful_builds: u64,
    /// Stateless elements whose `build` callback actually ran.
    pub stateless_builds: u64,
}

#[cfg(feature = "frame-stats")]
thread_local! {
    static STATS: Cell<RebuildStats> = const { Cell::new(RebuildStats {
        visits: 0,
        pruned: 0,
        stateful_checks: 0,
        stateless_checks: 0,
        stateful_builds: 0,
        stateless_builds: 0,
    }) };
}

#[cfg(feature = "frame-stats")]
#[inline]
fn update(update: impl FnOnce(&mut RebuildStats)) {
    STATS.with(|stats| {
        let mut current = stats.get();
        update(&mut current);
        stats.set(current);
    });
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_visit() {
    update(|stats| stats.visits += 1);
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_pruned() {
    update(|stats| stats.pruned += 1);
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_stateful_check() {
    update(|stats| stats.stateful_checks += 1);
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_stateless_check() {
    update(|stats| stats.stateless_checks += 1);
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_stateful_build() {
    update(|stats| stats.stateful_builds += 1);
}

#[cfg(feature = "frame-stats")]
#[inline]
pub(crate) fn record_stateless_build() {
    update(|stats| stats.stateless_builds += 1);
}

/// Clears the counters for the next frame.
#[cfg(feature = "frame-stats")]
#[inline]
pub fn reset() {
    STATS.with(|stats| stats.set(RebuildStats::default()));
}

/// Takes the counters for the completed frame and clears them.
#[cfg(feature = "frame-stats")]
#[inline]
pub fn take() -> RebuildStats {
    STATS.with(|stats| stats.replace(RebuildStats::default()))
}
