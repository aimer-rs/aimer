//! A countdown timer whose remaining time is also a live activity.
//!
//! Pick a timeout with the **-1 min**, **-10 s**, **+10 s** and **+1 min**
//! buttons, then press **Start**. The page counts down once a second and keeps
//! one [`aimer::live_activity`] up to date with the time left, so the countdown
//! stays on the lock screen (iOS), in the status bar (Android 16) or in the
//! tray tooltip (macOS, Windows and Linux) after the app is put away.
//! **Stop** cancels the timer and ends the activity.
//!
//! The tick is a Venus task over [`aimer::venus::interval`], so it needs no
//! thread and works the same on every platform. Dropping the page does not stop
//! the timer: the countdown and its activity belong to the app, and the page
//! only mirrors them.
//!
//! The deadline is a [`Countdown`], read against the real clock on every tick.
//! A tick that arrives late therefore never makes the timer run long.
//!
//! What the activity says is decided by [`running_activity`] and
//! [`finished_activity`], plain functions, so it is tested without a live
//! activity service.

use std::cell::{Cell, RefCell};
use std::time::{Duration, SystemTime};

use aimer::input::button::Button;
use aimer::live_activity as island;
use aimer::live_activity::{
    LiveActivities, LiveActivity, LiveActivityError, LiveActivityId, Presentation,
};
use aimer::style::*;
use aimer::svg::SvgColor;
use aimer::venus::{self, TaskId, Venus};
use aimer::*;
use aimer_time::{Countdown, Instant};

/// The shortest timeout the buttons reach.
const MIN_TIMEOUT: u64 = 10;

/// The longest timeout the buttons reach: three hours.
const MAX_TIMEOUT: u64 = 3 * 60 * 60;

/// The timeout the page opens with.
const DEFAULT_TIMEOUT: u64 = 60;

/// How often the countdown is refreshed.
const TICK: Duration = Duration::from_secs(1);

/// The one activity this page runs; showing it again updates it.
const ACTIVITY: LiveActivityId = LiveActivityId(0x71_4D_52);

/// The width of the progress bar track.
const BAR_WIDTH: f32 = 296.0;

/// A timer that is counting down.
struct Run {
    countdown: Countdown,
    total: Duration,
    task: TaskId,
}

aimer_tls::thread_local! {
    /// The timeout the next run starts from, in seconds.
    static TIMEOUT: Cell<u64> = const { Cell::new(DEFAULT_TIMEOUT) };
    /// The running timer, if there is one.
    static RUN: RefCell<Option<Run>> = const { RefCell::new(None) };
    /// Whether the last run reached zero and its activity still says so.
    static FINISHED: Cell<bool> = const { Cell::new(false) };
    static STATUS: RefCell<String> = const { RefCell::new(String::new()) };
    /// Where the activity went, or why it could not.
    static SURFACE: RefCell<String> = const { RefCell::new(String::new()) };
    /// The connection to the live activity service, once there is one.
    static ACTIVITIES: RefCell<Option<LiveActivities>> = const { RefCell::new(None) };
    /// The page that is showing the timer, if one is.
    static PAGE: Cell<Option<StateUpdater<LiveActivityTimerExampleState>>> = const { Cell::new(None) };
}

/// `timeout` moved by `delta` seconds and kept within the reachable range.
fn adjusted(timeout: u64, delta: i64) -> u64 {
    timeout
        .saturating_add_signed(delta)
        .clamp(MIN_TIMEOUT, MAX_TIMEOUT)
}

/// Whole seconds in `time`, counting a started second as a whole one, so the
/// clock only reads zero once the timer is over.
fn whole_seconds(time: Duration) -> u64 {
    time.as_secs() + u64::from(time.subsec_nanos() > 0)
}

/// `secs` as `M:SS`, or `H:MM:SS` from an hour up.
fn clock_text(secs: u64) -> String {
    let (hours, minutes, seconds) = (secs / 3600, secs / 60 % 60, secs % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{}:{seconds:02}", secs / 60)
    }
}

/// The activity for a timer with `remaining` of `total` left, `progress` of it
/// elapsed, that runs out at `ends_at`.
///
/// The clock is the [countdown](LiveActivity::countdown), which the system
/// ticks by itself: this app is suspended once it is in the background, and
/// text pushed from here would freeze on the island and the lock screen. The
/// body therefore holds nothing that changes with the clock. `status` is the
/// clock as of this tick, for the tray, which cannot count on its own.
///
/// The views are described with `UIKitWidget`s and drawn by the SwiftUI of the
/// widget extension, on the Dynamic Island and the lock screen. Android and the
/// tray ignore them.
fn running_activity(
    total: Duration,
    remaining: Duration,
    progress: f32,
    ends_at: SystemTime,
) -> LiveActivity {
    let left = whole_seconds(remaining);
    let all = whole_seconds(total);
    LiveActivity::new(ACTIVITY)
        .title("Timer")
        .body(format!("{} timer", clock_text(all)))
        .status(clock_text(left))
        .progress(progress)
        .countdown(ends_at, total)
        .icon("timer")
        .state("remaining", left.to_string())
        .state("total", all.to_string())
        .view(
            island::Region::CompactLeading,
            island::Symbol::new("timer").tint(island::Tint::Orange),
        )
        .view(
            island::Region::CompactTrailing,
            island::Clock::new()
                .style(island::TextStyle::Caption)
                .tint(island::Tint::Orange),
        )
        .view(
            island::Region::Minimal,
            island::Symbol::new("timer").tint(island::Tint::Orange),
        )
        .view(
            island::Region::ExpandedLeading,
            island::Symbol::new("timer").tint(island::Tint::Orange),
        )
        .view(
            island::Region::ExpandedTrailing,
            island::Clock::new()
                .style(island::TextStyle::Title)
                .tint(island::Tint::Orange),
        )
        .view(
            island::Region::ExpandedCenter,
            island::Label::new("Timer").style(island::TextStyle::Headline),
        )
        .view(
            island::Region::ExpandedBottom,
            island::Bar::new().tint(island::Tint::Orange),
        )
        .view(
            island::Region::LockScreen,
            island::Column::new()
                .spacing(8.0)
                .child(
                    island::Row::new()
                        .spacing(6.0)
                        .child(island::Symbol::new("timer").tint(island::Tint::Orange))
                        .child(island::Label::new("Timer").style(island::TextStyle::Headline))
                        .child(island::Gap::new())
                        .child(
                            island::Clock::new()
                                .style(island::TextStyle::Title)
                                .tint(island::Tint::Orange),
                        ),
                )
                .child(
                    island::Label::new(format!("{} timer", clock_text(all)))
                        .tint(island::Tint::Secondary),
                )
                .child(island::Bar::new().tint(island::Tint::Orange)),
        )
}

/// The activity once the timer has run out.
fn finished_activity(total: Duration) -> LiveActivity {
    let all = whole_seconds(total);
    LiveActivity::new(ACTIVITY)
        .title("Time's up")
        .body(format!("The {} timer is over", clock_text(all)))
        .status("Done")
        .progress(1.0)
        .icon("timer")
        .state("remaining", "0")
        .state("total", all.to_string())
}

fn presentation_name(presentation: Presentation) -> &'static str {
    match presentation {
        Presentation::Native => "the system live activity",
        Presentation::Tray => "the tray tooltip",
    }
}

/// Redraws the page, if one is showing. A page that has gone away rejects the
/// update, which is the right outcome.
fn refresh_page() {
    if let Some(page) = PAGE.get() {
        let _ = page.try_set_state(|_| {});
    }
}

fn set_status(status: impl Into<String>) {
    STATUS.with_borrow_mut(|slot| *slot = status.into());
    refresh_page();
}

fn set_surface(surface: impl Into<String>) {
    SURFACE.with_borrow_mut(|slot| *slot = surface.into());
}

/// The icon the tray shows where the platform has no live activities.
fn tray_icon() -> Option<aimer::tray::TrayIcon> {
    aimer::lucide_tray_icon(Icons::ALARM_CLOCK, 64, SvgColor::rgba8(40, 120, 255, 255)).ok()
}

/// Runs `action` with the live activity service, connecting first if need be.
/// A failure is shown on the page.
fn with_activities(action: impl FnOnce(&LiveActivities) -> Result<(), LiveActivityError>) {
    if ACTIVITIES.with_borrow(Option::is_none) {
        let mut builder = LiveActivities::new();
        if let Some(icon) = tray_icon() {
            builder = builder.tray_icon(icon);
        }
        match builder.build() {
            Ok(connected) => {
                set_surface(format!(
                    "Showing on {}.",
                    presentation_name(connected.presentation())
                ));
                ACTIVITIES.with_borrow_mut(|slot| *slot = Some(connected));
            }
            Err(why) => {
                set_surface(format!("No live activity: {why}"));
                return;
            }
        }
    }
    let result = ACTIVITIES.with_borrow(|slot| {
        slot.as_ref()
            .map(action)
    });
    if let Some(Err(why)) = result {
        set_surface(format!("The live activity failed: {why}"));
    }
}

/// The wall clock, which a live activity needs for its deadline. The web has
/// no live activity, and `SystemTime::now` panics there.
fn wall_clock_now() -> SystemTime {
    if cfg!(target_arch = "wasm32") {
        SystemTime::UNIX_EPOCH
    } else {
        SystemTime::now()
    }
}

/// Puts the state of the running timer on the activity.
fn publish() {
    let activity = RUN.with_borrow(|run| {
        run.as_ref()
            .map(|run| {
                let now = Instant::now();
                let remaining = run
                    .countdown
                    .remaining_at(now);
                running_activity(
                    run.total,
                    remaining,
                    run.countdown
                        .progress_at(now),
                    wall_clock_now() + remaining,
                )
            })
    });
    if let Some(activity) = activity {
        with_activities(|activities| activities.show(&activity));
    }
}

/// One tick: updates the activity, or finishes the timer. Returns whether the
/// tick loop goes on.
fn on_tick() -> bool {
    let over = RUN.with_borrow(|run| {
        run.as_ref()
            .map(|run| {
                run.countdown
                    .is_expired_at(Instant::now())
            })
    });
    match over {
        None => false,
        Some(false) => {
            publish();
            refresh_page();
            true
        }
        Some(true) => {
            let finished = RUN.with_borrow_mut(Option::take);
            if let Some(run) = finished {
                FINISHED.set(true);
                let activity = finished_activity(run.total);
                with_activities(|activities| activities.show(&activity));
            }
            set_status("Time's up.");
            false
        }
    }
}

/// The task behind a run: ticks until the timer is over or stopped.
async fn tick_loop(mut ticks: venus::Interval) {
    while ticks
        .tick()
        .await
        .is_ok()
        && on_tick()
    {}
}

fn adjust_timeout(delta: i64) {
    if RUN.with_borrow(Option::is_some) {
        set_status("Stop the timer before changing the timeout.");
        return;
    }
    TIMEOUT.set(adjusted(TIMEOUT.get(), delta));
    FINISHED.set(false);
    set_status("");
}

fn minus_minute() {
    adjust_timeout(-60);
}

fn minus_ten() {
    adjust_timeout(-10);
}

fn plus_ten() {
    adjust_timeout(10);
}

fn plus_minute() {
    adjust_timeout(60);
}

fn start() {
    if RUN.with_borrow(Option::is_some) {
        set_status("The timer is already running.");
        return;
    }
    let Some(task) = venus::spawn_local(tick_loop(venus::interval(TICK))) else {
        set_status("There is no async runtime to tick on.");
        return;
    };
    let total = Duration::from_secs(TIMEOUT.get());
    FINISHED.set(false);
    RUN.with_borrow_mut(|run| {
        *run = Some(Run {
            countdown: Countdown::new(total),
            total,
            task,
        })
    });
    publish();
    set_status("Running.");
}

fn stop() {
    let stopped = RUN.with_borrow_mut(Option::take);
    let was_finished = FINISHED.replace(false);
    if stopped.is_none() && !was_finished {
        set_status("Nothing to stop.");
        return;
    }
    if let (Some(run), Some(venus)) = (stopped, Venus::current()) {
        venus.abort(run.task);
    }
    with_activities(|activities| activities.end(ACTIVITY));
    set_status("Stopped.");
}

/// The timer page.
#[derive(StatefulWidget)]
pub struct LiveActivityTimerExample;

impl LiveActivityTimerExample {
    /// Creates the page.
    #[inline]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LiveActivityTimerExample {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

/// The retained side of the page. The timer and its activity outlive the page,
/// so all it keeps is a way to ask for a redraw.
pub struct LiveActivityTimerExampleState {
    updater: StateUpdater<Self>,
}

impl StatefulWidget for LiveActivityTimerExample {
    type State = LiveActivityTimerExampleState;

    fn create_state(self) -> LiveActivityTimerExampleState {
        LiveActivityTimerExampleState {
            updater: StateUpdater::empty(),
        }
    }
}

impl LiveActivityTimerExampleState {
    /// A line of text in the page's body style.
    fn line(text: impl Into<String>, color: Color, size: u32) -> AnyWidget {
        Text::new(text.into())
            .text_style(
                TextStyle::new()
                    .font_size(size)
                    .color(color),
            )
            .boxed()
    }

    fn button(label: &'static str, app_theme: &ThemeData, on_press: fn()) -> AnyWidget {
        Button::new()
            .on_press(on_press)
            .decoration(
                BoxDecoration::new()
                    .background_color(app_theme.primary_color)
                    .border_radius((8, 8, 8, 8)),
            )
            .child(
                Container::new()
                    .width(Dimension::Px(140.0))
                    .height(Dimension::Px(40.0))
                    .child(
                        Text::new(label)
                            .text_align(TextAlign::MidCenter)
                            .text_style(
                                TextStyle::new()
                                    .font_size(14)
                                    .color(app_theme.on_primary_color),
                            ),
                    ),
            )
            .boxed()
    }

    fn row(buttons: Vec<AnyWidget>) -> AnyWidget {
        Row::new()
            .gaps(LayoutSpacing::new().right(16))
            .children(buttons)
            .boxed()
    }

    /// The bar: a track with a fill `progress` of the way across.
    fn bar(progress: f32, app_theme: &ThemeData) -> AnyWidget {
        Container::new()
            .width(Dimension::Px(BAR_WIDTH))
            .height(Dimension::Px(8.0))
            .color(crate::theme::muted_text(app_theme))
            .child(
                Container::new()
                    .width(Dimension::Px(BAR_WIDTH * progress.clamp(0.0, 1.0)))
                    .height(Dimension::Px(8.0))
                    .color(app_theme.primary_color)
                    .child(ZeroSizedBox),
            )
            .boxed()
    }
}

impl State<LiveActivityTimerExample> for LiveActivityTimerExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>)
    where
        Self: Sized,
    {
        self.updater = updater;
        PAGE.set(Some(updater));
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let muted = crate::theme::muted_text(&app_theme);
        let body = app_theme.on_background_color;

        let now = Instant::now();
        let (clock, progress) = RUN.with_borrow(|run| match run {
            Some(run) => (
                whole_seconds(
                    run.countdown
                        .remaining_at(now),
                ),
                run.countdown
                    .progress_at(now),
            ),
            None if FINISHED.get() => (0, 1.0),
            None => (TIMEOUT.get(), 0.0),
        });
        let running = RUN.with_borrow(Option::is_some);
        let timeout_note = if running { "Running" } else { "Timeout" };
        let status = STATUS.with_borrow(|status| {
            if status.is_empty() {
                "Set a timeout, then press Start.".to_owned()
            } else {
                status.clone()
            }
        });
        let surface = SURFACE.with_borrow(Clone::clone);

        Container::new()
            .color(app_theme.background_color)
            .padding(LayoutSpacing::all(Spacing::Px(12)))
            .child(
                Column::new()
                    .horizontal_alignment(BoxAlignment::Start)
                    .gaps(LayoutSpacing::new().bottom(12))
                    .children(vec![
                        Text::new("Live activity timer")
                            .text_style(TextStyle::new().font_size(18).font_weight(FontWeight::Bold).color(body))
                            .boxed(),
                        Self::line(timeout_note, muted, 14),
                        Text::new(clock_text(clock))
                            .text_style(TextStyle::new().font_size(56).font_weight(FontWeight::Bold).color(body))
                            .boxed(),
                        Self::bar(progress, &app_theme),
                        Self::line(status, body, 15),
                        Self::row(vec![
                            Self::button("-1 min", &app_theme, minus_minute),
                            Self::button("+1 min", &app_theme, plus_minute),
                        ]),
                        Self::row(vec![
                            Self::button("-10 s", &app_theme, minus_ten),
                            Self::button("+10 s", &app_theme, plus_ten),
                        ]),
                        Self::row(vec![
                            Self::button("Start", &app_theme, start),
                            Self::button("Stop", &app_theme, stop),
                        ]),
                        Self::line(surface, muted, 14),
                        Self::line(
                            "The countdown keeps running when you leave this page. iOS needs the widget extension \
                             and NSSupportsLiveActivities, Android 16 shows a Live Update, and desktop shows the \
                             time left in a tray icon's tooltip.",
                            muted,
                            14,
                        ),
                    ]),
            )
    }
}

#[cfg(test)]
mod tests {
    use aimer::quiver::aimer_app::AimerApp;

    use super::*;

    #[test]
    fn the_timeout_stays_within_its_range() {
        assert_eq!(adjusted(60, 10), 70);
        assert_eq!(adjusted(60, -10), 50);
        assert_eq!(adjusted(MIN_TIMEOUT, -60), MIN_TIMEOUT);
        assert_eq!(adjusted(15, -60), MIN_TIMEOUT);
        assert_eq!(adjusted(MAX_TIMEOUT, 60), MAX_TIMEOUT);
        assert_eq!(adjusted(u64::MAX, 60), MAX_TIMEOUT);
    }

    #[test]
    fn a_started_second_counts_as_a_whole_one() {
        assert_eq!(whole_seconds(Duration::ZERO), 0);
        assert_eq!(whole_seconds(Duration::from_millis(1)), 1);
        assert_eq!(whole_seconds(Duration::from_millis(1000)), 1);
        assert_eq!(whole_seconds(Duration::from_millis(1001)), 2);
    }

    #[test]
    fn the_clock_reads_minutes_then_hours() {
        assert_eq!(clock_text(0), "0:00");
        assert_eq!(clock_text(9), "0:09");
        assert_eq!(clock_text(60), "1:00");
        assert_eq!(clock_text(754), "12:34");
        assert_eq!(clock_text(3599), "59:59");
        assert_eq!(clock_text(3600), "1:00:00");
        assert_eq!(clock_text(MAX_TIMEOUT), "3:00:00");
    }

    #[test]
    fn the_running_activity_shows_the_time_left() {
        let ends = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_030);
        let activity = running_activity(
            Duration::from_secs(120),
            Duration::from_millis(29_500),
            0.75,
            ends,
        );
        assert_eq!(activity.id(), ACTIVITY);
        assert_eq!(activity.progress_percent(), Some(75));
        assert_eq!(
            activity,
            LiveActivity::new(ACTIVITY)
                .title("Timer")
                .body("2:00 timer")
                .status("0:30")
                .progress(0.75)
                .countdown(ends, Duration::from_secs(120))
                .icon("timer")
                .state("remaining", "30")
                .state("total", "120")
                .view(
                    island::Region::CompactLeading,
                    island::Symbol::new("timer").tint(island::Tint::Orange)
                )
                .view(
                    island::Region::CompactTrailing,
                    island::Clock::new()
                        .style(island::TextStyle::Caption)
                        .tint(island::Tint::Orange)
                )
                .view(
                    island::Region::Minimal,
                    island::Symbol::new("timer").tint(island::Tint::Orange)
                )
                .view(
                    island::Region::ExpandedLeading,
                    island::Symbol::new("timer").tint(island::Tint::Orange)
                )
                .view(
                    island::Region::ExpandedTrailing,
                    island::Clock::new()
                        .style(island::TextStyle::Title)
                        .tint(island::Tint::Orange)
                )
                .view(
                    island::Region::ExpandedCenter,
                    island::Label::new("Timer").style(island::TextStyle::Headline)
                )
                .view(
                    island::Region::ExpandedBottom,
                    island::Bar::new().tint(island::Tint::Orange)
                )
                .view(
                    island::Region::LockScreen,
                    island::Column::new()
                        .spacing(8.0)
                        .child(
                            island::Row::new()
                                .spacing(6.0)
                                .child(island::Symbol::new("timer").tint(island::Tint::Orange))
                                .child(
                                    island::Label::new("Timer").style(island::TextStyle::Headline)
                                )
                                .child(island::Gap::new())
                                .child(
                                    island::Clock::new()
                                        .style(island::TextStyle::Title)
                                        .tint(island::Tint::Orange)
                                ),
                        )
                        .child(island::Label::new("2:00 timer").tint(island::Tint::Secondary))
                        .child(island::Bar::new().tint(island::Tint::Orange)),
                )
        );
    }

    #[test]
    fn the_finished_activity_is_full_and_says_so() {
        let activity = finished_activity(Duration::from_secs(60));
        assert_eq!(activity.progress_percent(), Some(100));
        assert_eq!(
            activity,
            LiveActivity::new(ACTIVITY)
                .title("Time's up")
                .body("The 1:00 timer is over")
                .status("Done")
                .progress(1.0)
                .icon("timer")
                .state("remaining", "0")
                .state("total", "60")
        );
    }

    #[test]
    fn the_buttons_move_the_timeout_while_idle() {
        RUN.with_borrow_mut(|run| *run = None);
        TIMEOUT.set(DEFAULT_TIMEOUT);
        plus_minute();
        plus_ten();
        assert_eq!(TIMEOUT.get(), 130);
        minus_minute();
        minus_ten();
        assert_eq!(TIMEOUT.get(), DEFAULT_TIMEOUT);
    }

    #[test]
    fn stopping_an_idle_timer_changes_nothing() {
        RUN.with_borrow_mut(|run| *run = None);
        FINISHED.set(false);
        stop();
        assert_eq!(STATUS.with_borrow(Clone::clone), "Nothing to stop.");
        assert!(
            ACTIVITIES.with_borrow(Option::is_none),
            "no activity service is touched"
        );
    }

    #[test]
    fn the_page_builds_and_renders() {
        RUN.with_borrow_mut(|run| *run = None);
        TIMEOUT.set(DEFAULT_TIMEOUT);
        let mut app = AimerApp::start_headless(crate::theme::provide(
            LiveActivityTimerExample::new().boxed(),
        ));
        app.render_frame();
        assert!(
            PAGE.get().is_some(),
            "the page registered itself for redraws"
        );
    }
}
