//! Diagnostic probe for redundant `sync_paint_geometry` walks (`frame-stats` only).
//!
//! Wrapper elements forward `sync_paint_geometry` to their child, and every
//! element also syncs its own paint state, which starts the same walk again one
//! level down. This probe records, per element, whether it synced itself and how
//! many times a wrapper above it forwarded the walk to it, so a frame can be
//! summarized as "how much of the forwarding repeated work the element did
//! anyway".
//!
//! The state lives in a plain thread-local rather than in
//! [`FrameWorkStats`](crate::frame_work_stats::FrameWorkStats): the frame drawer
//! takes those counters at the end of every draw, so a caller could never read
//! them afterwards. Here the caller decides when a window ends, by calling
//! [`take_geometry_walk_report`]. Without the `frame-stats` feature every
//! function compiles to nothing and the report is empty.

#[cfg(feature = "frame-stats")]
use std::cell::RefCell;
#[cfg(feature = "frame-stats")]
use std::collections::HashMap;

/// How many leading buckets of [`GeometryWalkReport::forwards_per_self_synced`]
/// count exact values; the last bucket counts that many or more.
pub const FORWARD_BUCKETS: usize = 6;

/// One window of geometry-walk activity, as returned by
/// [`take_geometry_walk_report`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GeometryWalkReport {
    /// Elements that synced their own paint state (each starts its own walk).
    pub own_syncs: u64,
    /// Visits a wrapper forwarded to a child's `sync_paint_geometry`.
    pub forwarded_visits: u64,
    /// Distinct elements that received at least one forwarded visit.
    pub distinct_forwarded: u64,
    /// Elements that were forwarded to but never synced themselves in the
    /// window, for which the forward is the only geometry sync they got.
    pub forward_only_elements: u64,
    /// Forwarded visits to elements that also synced themselves in the window.
    /// These are the candidates for removal.
    pub redundant_visits: u64,
    /// Self-synced elements bucketed by how many forwarded visits they also
    /// received: index `n` holds those forwarded to `n` times, the last index
    /// `FORWARD_BUCKETS - 1` or more.
    pub forwards_per_self_synced: [u64; FORWARD_BUCKETS],
    /// The most forwarded visits any single element received.
    pub max_forwards_on_one_element: u64,
    /// Forwards a wrapper skipped because the child was about to sync itself.
    pub suppressed_forwards: u64,
    /// Elements that were spared a forward and then never synced themselves in
    /// the window. They got no geometry sync at all, so this must stay zero.
    pub unpaid_suppressions: u64,
    /// Element types receiving the most forwarded visits, descending.
    pub top_types: Vec<(&'static str, u64)>,
}

impl GeometryWalkReport {
    /// Adds `other` into this report, for totalling several per-frame reports.
    ///
    /// Counts and buckets add, the worst single element is kept, and the type
    /// lists are combined by name and cut back to the eight busiest.
    pub fn merge(&mut self, other: &Self) {
        self.own_syncs += other.own_syncs;
        self.forwarded_visits += other.forwarded_visits;
        self.distinct_forwarded += other.distinct_forwarded;
        self.forward_only_elements += other.forward_only_elements;
        self.redundant_visits += other.redundant_visits;
        for (mine, theirs) in self
            .forwards_per_self_synced
            .iter_mut()
            .zip(other.forwards_per_self_synced)
        {
            *mine += theirs;
        }
        self.max_forwards_on_one_element = self
            .max_forwards_on_one_element
            .max(other.max_forwards_on_one_element);
        self.suppressed_forwards += other.suppressed_forwards;
        self.unpaid_suppressions += other.unpaid_suppressions;
        for &(name, visits) in &other.top_types {
            match self.top_types.iter_mut().find(|(known, _)| *known == name) {
                Some((_, known)) => *known += visits,
                None => self.top_types.push((name, visits)),
            }
        }
        self.top_types
            .sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        self.top_types.truncate(8);
    }
}

#[cfg(feature = "frame-stats")]
#[derive(Default)]
struct Entry {
    own: u32,
    forwards: u64,
    suppressed: u64,
}

#[cfg(feature = "frame-stats")]
#[derive(Default)]
struct Probe {
    elements: HashMap<u64, Entry>,
    own_syncs: u64,
    types: HashMap<&'static str, u64>,
}

#[cfg(feature = "frame-stats")]
std::thread_local! {
    static PROBE: RefCell<Probe> = RefCell::new(Probe::default());
}

/// Records that the element `id` synchronized its own paint state.
#[doc(hidden)]
#[inline]
pub fn record_geometry_own_sync(id: u64) {
    #[cfg(feature = "frame-stats")]
    PROBE.with(|probe| {
        let mut probe = probe.borrow_mut();
        probe.own_syncs += 1;
        probe.elements.entry(id).or_default().own += 1;
    });
    #[cfg(not(feature = "frame-stats"))]
    let _ = id;
}

/// Records that a wrapper forwarded `sync_paint_geometry` to the element `id`
/// of type `type_name`.
#[doc(hidden)]
#[inline]
pub fn record_geometry_forward(id: u64, type_name: &'static str) {
    #[cfg(feature = "frame-stats")]
    PROBE.with(|probe| {
        let mut probe = probe.borrow_mut();
        probe.elements.entry(id).or_default().forwards += 1;
        *probe.types.entry(type_name).or_default() += 1;
    });
    #[cfg(not(feature = "frame-stats"))]
    let _ = (id, type_name);
}

/// Records that a wrapper skipped forwarding to the element `id` because that
/// element was about to sync its own geometry.
#[doc(hidden)]
#[inline]
pub fn record_geometry_suppressed(id: u64) {
    #[cfg(feature = "frame-stats")]
    PROBE.with(|probe| {
        probe.borrow_mut().elements.entry(id).or_default().suppressed += 1;
    });
    #[cfg(not(feature = "frame-stats"))]
    let _ = id;
}

/// Summarizes everything recorded since the previous call and starts a new
/// window.
#[doc(hidden)]
pub fn take_geometry_walk_report() -> GeometryWalkReport {
    #[cfg(feature = "frame-stats")]
    return PROBE.with(|probe| {
        let probe = probe.take();
        let mut report = GeometryWalkReport {
            own_syncs: probe.own_syncs,
            ..GeometryWalkReport::default()
        };
        for entry in probe.elements.values() {
            report.forwarded_visits += entry.forwards;
            report.suppressed_forwards += entry.suppressed;
            if entry.suppressed > 0 && entry.own == 0 {
                report.unpaid_suppressions += 1;
            }
            report.max_forwards_on_one_element =
                report.max_forwards_on_one_element.max(entry.forwards);
            if entry.forwards > 0 {
                report.distinct_forwarded += 1;
            }
            if entry.own > 0 {
                report.redundant_visits += entry.forwards;
                let bucket = (entry.forwards as usize).min(FORWARD_BUCKETS - 1);
                report.forwards_per_self_synced[bucket] += 1;
            } else if entry.forwards > 0 {
                report.forward_only_elements += 1;
            }
        }
        let mut types: Vec<_> = probe.types.into_iter().collect();
        types.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        types.truncate(8);
        report.top_types = types;
        report
    });

    #[cfg(not(feature = "frame-stats"))]
    GeometryWalkReport::default()
}

#[cfg(all(test, feature = "frame-stats"))]
mod tests {
    use super::*;

    /// Element 1 syncs itself and is also forwarded to twice, element 2 syncs
    /// itself and is never forwarded to, element 3 is only ever forwarded to.
    #[test]
    fn report_separates_self_synced_from_forward_only_elements() {
        let _ = take_geometry_walk_report();
        record_geometry_own_sync(1);
        record_geometry_own_sync(2);
        record_geometry_forward(1, "Container");
        record_geometry_forward(1, "Container");
        record_geometry_forward(3, "MouseRegion");

        let report = take_geometry_walk_report();
        assert_eq!(report.own_syncs, 2);
        assert_eq!(report.forwarded_visits, 3);
        assert_eq!(report.distinct_forwarded, 2);
        assert_eq!(report.forward_only_elements, 1);
        // Forwards that re-do work a self-synced element already did itself.
        assert_eq!(report.redundant_visits, 2);
        // Self-synced elements bucketed by how often they were also forwarded to.
        assert_eq!(report.forwards_per_self_synced, [1, 0, 1, 0, 0, 0]);
        assert_eq!(report.max_forwards_on_one_element, 2);
        assert_eq!(report.top_types[0], ("Container", 2));
        assert_eq!(report.top_types[1], ("MouseRegion", 1));

        // Taking the report starts a fresh window.
        assert_eq!(take_geometry_walk_report().forwarded_visits, 0);
    }

    /// A forward that was skipped is paid by the child's own sync; one the child
    /// never pays is reported, because that child got no geometry at all.
    #[test]
    fn a_skipped_forward_is_unpaid_until_the_child_syncs_itself() {
        let _ = take_geometry_walk_report();
        record_geometry_suppressed(4);
        record_geometry_own_sync(4);
        record_geometry_suppressed(5);
        record_geometry_suppressed(5);

        let report = take_geometry_walk_report();
        assert_eq!(report.suppressed_forwards, 3);
        assert_eq!(report.unpaid_suppressions, 1);
    }

    #[test]
    fn merging_reports_adds_frames_and_keeps_the_worst_element() {
        let mut total = GeometryWalkReport {
            own_syncs: 2,
            forwarded_visits: 3,
            distinct_forwarded: 2,
            forward_only_elements: 1,
            redundant_visits: 2,
            forwards_per_self_synced: [1, 0, 1, 0, 0, 0],
            max_forwards_on_one_element: 2,
            suppressed_forwards: 5,
            unpaid_suppressions: 1,
            top_types: vec![("Container", 2), ("MouseRegion", 1)],
        };
        let other = GeometryWalkReport {
            own_syncs: 1,
            forwarded_visits: 4,
            distinct_forwarded: 1,
            forward_only_elements: 0,
            redundant_visits: 4,
            forwards_per_self_synced: [0, 0, 0, 0, 1, 0],
            max_forwards_on_one_element: 4,
            suppressed_forwards: 2,
            unpaid_suppressions: 0,
            top_types: vec![("MouseRegion", 4)],
        };
        total.merge(&other);
        assert_eq!(total.own_syncs, 3);
        assert_eq!(total.forwarded_visits, 7);
        assert_eq!(total.distinct_forwarded, 3);
        assert_eq!(total.forward_only_elements, 1);
        assert_eq!(total.redundant_visits, 6);
        assert_eq!(total.forwards_per_self_synced, [1, 0, 1, 0, 1, 0]);
        assert_eq!(total.max_forwards_on_one_element, 4);
        assert_eq!(total.suppressed_forwards, 7);
        assert_eq!(total.unpaid_suppressions, 1);
        assert_eq!(total.top_types, vec![("MouseRegion", 5), ("Container", 2)]);
    }
}
