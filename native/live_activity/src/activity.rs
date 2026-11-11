//! What a live activity says.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::view::{Region, UIKitWidget, ViewNode};
use crate::LiveActivityError;

/// Names a live activity, so it can be updated and ended. Showing an activity
/// with the id of one already running updates it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LiveActivityId(pub u64);

/// How far along the activity is.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Progress {
    /// No bar at all.
    #[default]
    None,
    /// A bar that is busy but cannot say how far it is.
    Indeterminate,
    /// A share of the work done, from `0.0` to `1.0`. Values outside the range
    /// are clamped and `NaN` counts as `0.0`.
    Fraction(f32),
}

impl Progress {
    /// The sanitized fraction, or `None` when the bar is absent or
    /// indeterminate.
    #[inline]
    pub(crate) fn fraction(self) -> Option<f32> {
        match self {
            Self::Fraction(value) => Some(sanitize(value)),
            Self::None | Self::Indeterminate => None,
        }
    }

    /// The progress rounded to a whole percent, or `None` when the bar is
    /// absent or indeterminate.
    #[inline]
    pub fn percent(self) -> Option<u8> {
        self.fraction().map(to_percent)
    }
}

#[inline]
fn sanitize(value: f32) -> f32 {
    if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) }
}

#[inline]
fn to_percent(fraction: f32) -> u8 {
    // `fraction` is within 0..=1, so the product is within 0..=100.
    (fraction * 100.0).round() as u8
}

/// A countdown the system ticks by itself, as wall-clock milliseconds since
/// the Unix epoch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Countdown {
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
}

/// `time` as milliseconds since the Unix epoch; a time before it counts as the
/// epoch itself.
fn epoch_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

/// A named point along the progress bar, such as "Picked up" at `0.5`.
#[derive(Clone, Debug, PartialEq)]
pub struct Milestone {
    pub(crate) label: String,
    pub(crate) at: f32,
}

impl Milestone {
    /// A milestone called `label` at the fraction `at` of the bar. `at` is
    /// clamped to `0.0..=1.0`.
    #[inline]
    pub fn new(label: impl Into<String>, at: f32) -> Self {
        Self { label: label.into(), at: sanitize(at) }
    }

    /// The text of the milestone.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Where the milestone sits along the bar, from `0.0` to `1.0`.
    #[inline]
    pub fn at(&self) -> f32 {
        self.at
    }

    /// Where the milestone sits, as a whole percent.
    #[inline]
    pub fn position(&self) -> u8 {
        to_percent(self.at)
    }
}

/// The content of one live activity.
///
/// Every platform shows the same few things, in the template it owns: Android's
/// Live Update is a fixed system layout, and on Apple platforms the layout is
/// the SwiftUI view of the application's widget extension. [`LiveActivity::state`]
/// is the escape hatch for the latter: free-form pairs that only a custom
/// SwiftUI view reads, and every other platform ignores.
///
/// # Examples
///
/// ```
/// use aimer_live_activity::{LiveActivity, LiveActivityId, Milestone};
///
/// let ride = LiveActivity::new(LiveActivityId(7))
///     .title("Ride to the airport")
///     .body("Driver is 4 minutes away")
///     .status("4 min")
///     .progress(0.6)
///     .milestone(Milestone::new("Picked up", 0.5))
///     .state("driver", "Ada");
/// assert_eq!(ride.progress_percent(), Some(60));
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct LiveActivity {
    pub(crate) id: LiveActivityId,
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) status: String,
    pub(crate) progress: Progress,
    pub(crate) milestones: Vec<Milestone>,
    pub(crate) icon: String,
    pub(crate) state: Vec<(String, String)>,
    pub(crate) countdown: Option<Countdown>,
    pub(crate) views: Vec<(Region, ViewNode)>,
}

impl LiveActivity {
    /// Starts describing the activity named `id`, with no content yet.
    #[inline]
    pub fn new(id: LiveActivityId) -> Self {
        Self {
            id,
            title: String::new(),
            body: String::new(),
            status: String::new(),
            progress: Progress::None,
            milestones: Vec::new(),
            icon: String::new(),
            state: Vec::new(),
            countdown: None,
            views: Vec::new(),
        }
    }

    /// The headline.
    #[inline]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// The line under the headline.
    #[inline]
    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    /// A few words that stay visible when space is tight: Android's status chip
    /// (about seven characters fit), the compact Dynamic Island on iOS.
    #[inline]
    pub fn status(mut self, status: impl Into<String>) -> Self {
        self.status = status.into();
        self
    }

    /// A determinate bar at `fraction` of `0.0..=1.0`.
    #[inline]
    pub fn progress(mut self, fraction: f32) -> Self {
        self.progress = Progress::Fraction(fraction);
        self
    }

    /// A bar that cannot say how far along it is.
    #[inline]
    pub fn indeterminate(mut self) -> Self {
        self.progress = Progress::Indeterminate;
        self
    }

    /// A countdown that ends at `ends_at` and started `total` earlier.
    ///
    /// The text and the bar of an activity only change when the application
    /// shows it again, and an application that is in the background cannot do
    /// that: iOS suspends it, and Android may too. A countdown is the part the
    /// system keeps ticking on its own, with no help from the application: the
    /// clock and bar of the Dynamic Island and lock screen (iOS) and the
    /// chronometer of the notification and its chip (Android). Show the
    /// activity again with a new `ends_at` to move the deadline, and without a
    /// countdown once it is over. The tray ignores it.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::{Duration, SystemTime};
    /// use aimer_live_activity::{LiveActivity, LiveActivityId};
    ///
    /// let ends = SystemTime::now() + Duration::from_secs(90);
    /// let timer = LiveActivity::new(LiveActivityId(1))
    ///     .title("Tea")
    ///     .countdown(ends, Duration::from_secs(180));
    /// ```
    #[inline]
    pub fn countdown(mut self, ends_at: SystemTime, total: Duration) -> Self {
        let end_ms = epoch_millis(ends_at);
        let total_ms = u64::try_from(total.as_millis()).unwrap_or(u64::MAX);
        self.countdown = Some(Countdown { start_ms: end_ms.saturating_sub(total_ms), end_ms });
        self
    }

    /// Adds a named point along the bar.
    #[inline]
    pub fn milestone(mut self, milestone: Milestone) -> Self {
        self.milestones.push(milestone);
        self
    }

    /// The icon, named in the platform's own vocabulary: a drawable resource
    /// name on Android, an SF Symbol name on Apple platforms. Unknown or empty
    /// names fall back to the application's own icon.
    #[inline]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = icon.into();
        self
    }

    /// Adds a free-form pair for a custom SwiftUI view. A repeated `key`
    /// replaces its earlier value. Android and the tray ignore it.
    pub fn state(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = value.into();
        match self.state.iter_mut().find(|(existing, _)| *existing == key) {
            Some(pair) => pair.1 = value,
            None => self.state.push((key, value)),
        }
        self
    }

    /// Fills `region` of the Dynamic Island or the lock screen with `widget`,
    /// replacing the template's content there and any view set for the same
    /// region before.
    ///
    /// The widget is described, not drawn: the tree travels with the activity
    /// and a SwiftUI interpreter in the widget extension draws it. Android and
    /// the tray ignore it. The whole payload of an activity may not exceed about
    /// 4 KB, so keep the trees small.
    ///
    /// # Examples
    ///
    /// ```
    /// use aimer_live_activity::{Clock, Label, LiveActivity, LiveActivityId, Region, Row, Symbol};
    ///
    /// let timer = LiveActivity::new(LiveActivityId(1))
    ///     .title("Tea")
    ///     .view(Region::CompactLeading, Symbol::new("cup.and.saucer.fill"))
    ///     .view(Region::CompactTrailing, Clock::new())
    ///     .view(Region::LockScreen, Row::new().child(Label::new("Tea")).child(Clock::new()));
    /// ```
    pub fn view(mut self, region: Region, widget: impl UIKitWidget) -> Self {
        let node = widget.to_node();
        match self.views.iter_mut().find(|(existing, _)| *existing == region) {
            Some(slot) => slot.1 = node,
            None => self.views.push((region, node)),
        }
        self
    }

    /// The id the activity is named by.
    #[inline]
    pub fn id(&self) -> LiveActivityId {
        self.id
    }

    /// The progress, rounded to a whole percent; `None` for no bar or an
    /// indeterminate one.
    #[inline]
    pub fn progress_percent(&self) -> Option<u8> {
        self.progress.percent()
    }

    /// Fails when there is nothing to show.
    pub(crate) fn validate(&self) -> Result<(), LiveActivityError> {
        if self.title.is_empty() && self.body.is_empty() {
            Err(LiveActivityError::Empty)
        } else {
            Ok(())
        }
    }

    /// One line for the places that have room for no more, such as a tray
    /// tooltip: the non-empty parts of title, body, status and percent.
    pub(crate) fn summary(&self) -> String {
        let percent = self.progress_percent().map(|percent| format!("{percent}%"));
        let parts = [
            Some(self.title.as_str()),
            Some(self.body.as_str()),
            Some(self.status.as_str()),
            percent.as_deref(),
        ];
        let mut line = String::new();
        for part in parts.into_iter().flatten().filter(|part| !part.is_empty()) {
            if !line.is_empty() {
                line.push_str(" \u{b7} ");
            }
            line.push_str(part);
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_rounds_and_clamps() {
        assert_eq!(Progress::Fraction(0.0).percent(), Some(0));
        assert_eq!(Progress::Fraction(0.424).percent(), Some(42));
        assert_eq!(Progress::Fraction(0.425).percent(), Some(43));
        assert_eq!(Progress::Fraction(1.0).percent(), Some(100));
        assert_eq!(Progress::Fraction(7.5).percent(), Some(100));
        assert_eq!(Progress::Fraction(-3.0).percent(), Some(0));
        assert_eq!(Progress::Fraction(f32::NAN).percent(), Some(0));
        assert_eq!(Progress::Fraction(f32::INFINITY).percent(), Some(100));
    }

    #[test]
    fn absent_and_indeterminate_have_no_percent() {
        assert_eq!(Progress::None.percent(), None);
        assert_eq!(Progress::Indeterminate.percent(), None);
        assert_eq!(Progress::default(), Progress::None);
    }

    #[test]
    fn milestone_position_is_clamped() {
        assert_eq!(Milestone::new("a", 0.5).position(), 50);
        assert_eq!(Milestone::new("a", 2.0).position(), 100);
        assert_eq!(Milestone::new("a", -1.0).position(), 0);
        assert_eq!(Milestone::new("a", f32::NAN).at(), 0.0);
    }

    #[test]
    fn empty_activity_is_invalid() {
        let id = LiveActivityId(1);
        assert_eq!(LiveActivity::new(id).validate(), Err(LiveActivityError::Empty));
        assert_eq!(LiveActivity::new(id).status("x").validate(), Err(LiveActivityError::Empty));
        assert!(LiveActivity::new(id).title("x").validate().is_ok());
        assert!(LiveActivity::new(id).body("x").validate().is_ok());
    }

    #[test]
    fn countdown_spans_total_up_to_the_deadline() {
        let ends = UNIX_EPOCH + Duration::from_millis(1_700_000_090_500);
        let activity = LiveActivity::new(LiveActivityId(1))
            .title("x")
            .countdown(ends, Duration::from_secs(90));
        assert_eq!(
            activity.countdown,
            Some(Countdown { start_ms: 1_700_000_000_500, end_ms: 1_700_000_090_500 })
        );
    }

    #[test]
    fn countdown_never_starts_before_the_epoch() {
        let ends = UNIX_EPOCH + Duration::from_secs(5);
        let activity = LiveActivity::new(LiveActivityId(1)).countdown(ends, Duration::from_secs(60));
        assert_eq!(activity.countdown, Some(Countdown { start_ms: 0, end_ms: 5000 }));
        let before = LiveActivity::new(LiveActivityId(1)).countdown(UNIX_EPOCH - Duration::from_secs(1), Duration::ZERO);
        assert_eq!(before.countdown, Some(Countdown { start_ms: 0, end_ms: 0 }));
    }

    #[test]
    fn a_view_replaces_the_one_in_the_same_region() {
        use crate::{Clock, Label};
        let activity = LiveActivity::new(LiveActivityId(1))
            .view(Region::Minimal, Label::new("a"))
            .view(Region::LockScreen, Clock::new())
            .view(Region::Minimal, Label::new("b"));
        assert_eq!(activity.views.len(), 2);
        assert_eq!(activity.views[0], (Region::Minimal, Label::new("b").to_node()));
    }

    #[test]
    fn state_replaces_a_repeated_key() {
        let activity = LiveActivity::new(LiveActivityId(1)).state("a", "1").state("b", "2").state("a", "3");
        assert_eq!(activity.state, vec![("a".into(), "3".into()), ("b".into(), "2".into())]);
    }

    #[test]
    fn summary_joins_the_non_empty_parts() {
        let id = LiveActivityId(1);
        assert_eq!(LiveActivity::new(id).title("Ride").summary(), "Ride");
        let full = LiveActivity::new(id).title("Ride").body("Near").status("4 min").progress(0.6);
        assert_eq!(full.summary(), "Ride \u{b7} Near \u{b7} 4 min \u{b7} 60%");
        let bar_only = LiveActivity::new(id).body("Near").indeterminate();
        assert_eq!(bar_only.summary(), "Near");
    }
}
