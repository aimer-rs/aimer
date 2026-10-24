//! A themed, interactive showcase for Aimer's feedback and overlay contracts.
//!
//! The page keeps the clock, queue, tooltip controller, and overlay host in
//! one retained state value. Buttons therefore demonstrate the same explicit
//! host seam an application would use without relying on a process-global
//! popup registry.

use std::time::Duration;

use aimer::feedback::{
    Announcement, Announcer, DismissReason, ManualClock, MotionPolicy, OverlayHost, OverlayId,
    OverlayRequest, ProgressIndicator, Spinner, StatusBanner, StatusKind, Toast, ToastAction,
    ToastKind, ToastQueue, ToastQueueEvent, Tooltip, TooltipController, TooltipEvent,
};
use aimer::input::button::Button;
use aimer::macros::widget;
use aimer::style::{
    AnimatedTheme, BoxDecoration, FontWeight, LayoutSpacing, Spacing, TextAlign, TextStyle, Theme,
    ThemeData,
};
use aimer::{
    AimerApp, AnyWidget, BuildContext, Column, Container, Dimension, Row, ScrollAxis, Scrollable,
    State, StateUpdater, StatefulWidget, Text, Widget,
};

/// Builds the feedback showcase without starting an application.
pub fn feedback_example() -> impl Widget {
    FeedbackExample::new()
}

/// Starts the standalone feedback showcase with Jaime's application theme.
pub fn start_feedback_example() {
    AimerApp::start(local_theme_provider(feedback_example()));
}

/// A live page exercising status slots, progress indicators, and explicit
/// tooltip/toast host lifecycles.
#[widget(Stateful)]
pub struct FeedbackExample {}

impl FeedbackExample {
    /// Creates an empty page configuration; retained demo state is initialized
    /// when the widget is mounted.
    #[inline]
    pub const fn new() -> Self {
        Self {}
    }
}

impl Default for FeedbackExample {
    fn default() -> Self {
        Self::new()
    }
}

/// Retained state for [`FeedbackExample`].
pub struct FeedbackExampleState {
    progress_value: f32,
    motion_policy: MotionPolicy,
    spinner: Spinner,
    clock: ManualClock,
    queue: ToastQueue<ManualClock>,
    tooltip: TooltipController<ManualClock>,
    host: DemoOverlayHost,
    announcer: DemoAnnouncer,
    last_toast_event: ToastQueueEvent,
    last_tooltip_event: TooltipEvent,
    updater: StateUpdater<Self>,
}

impl FeedbackExampleState {
    fn initial() -> Self {
        let clock = ManualClock::new();
        let mut host = DemoOverlayHost::default();
        let mut announcer = DemoAnnouncer::default();
        let mut queue = ToastQueue::new(clock.clone());

        queue.enqueue(
            Toast::new("Saved successfully")
                .kind(ToastKind::Success)
                .replacement_key("save")
                .action(ToastAction::new("Undo", "undo")),
        );
        queue.enqueue(
            Toast::new("Queued notification")
                .kind(ToastKind::Info)
                .timeout(Duration::from_secs(4)),
        );
        let last_toast_event = queue.pump(&mut host, Some(&mut announcer));

        let mut tooltip = TooltipController::new(
            Tooltip::new("Tooltips use the same explicit host seam")
                .delay(Duration::from_millis(350)),
            clock.clone(),
        );
        tooltip.set_anchor(aimer::feedback::Rect::new(0.0, 0.0, 160.0, 36.0));

        let mut spinner = Spinner::new();
        spinner.set_motion_policy(MotionPolicy::Reduced);

        Self {
            progress_value: 0.65,
            motion_policy: MotionPolicy::Reduced,
            spinner,
            clock,
            queue,
            tooltip,
            host,
            announcer,
            last_toast_event,
            last_tooltip_event: TooltipEvent::Idle,
            updater: StateUpdater::empty(),
        }
    }

    fn advance_progress(&mut self) {
        self.progress_value = (self.progress_value + 0.1).min(1.0);
    }

    fn reset_progress(&mut self) {
        self.progress_value = 0.0;
    }

    fn toggle_motion(&mut self) {
        self.motion_policy = match self.motion_policy {
            MotionPolicy::Full => MotionPolicy::Reduced,
            MotionPolicy::Reduced => MotionPolicy::Full,
        };
        self.spinner
            .set_motion_policy(self.motion_policy);
    }

    fn advance_spinner(&mut self) {
        self.spinner
            .advance(Duration::from_millis(250));
    }

    fn queue_toast(&mut self) {
        self.queue.enqueue(
            Toast::new("A queued toast is waiting")
                .kind(ToastKind::Info)
                .timeout(Duration::from_secs(4)),
        );
        self.last_toast_event = self
            .queue
            .pump(&mut self.host, Some(&mut self.announcer));
    }

    fn advance_toast_clock(&mut self) {
        self.clock
            .advance(Duration::from_secs(4));
        let event = self
            .queue
            .pump(&mut self.host, Some(&mut self.announcer));
        self.last_toast_event = event;
        // `ToastQueue::pump` reports a timeout before presenting the next
        // queued item. Pump once more so the control feels like a single
        // deterministic "advance" action while retaining the useful
        // timeout transition in the summary.
        if matches!(event, ToastQueueEvent::Dismissed { .. }) {
            let _ = self
                .queue
                .pump(&mut self.host, Some(&mut self.announcer));
        }
    }

    fn dismiss_toast(&mut self) {
        if self
            .queue
            .dismiss_active(&mut self.host, DismissReason::Programmatic)
        {
            self.last_toast_event = self
                .queue
                .pump(&mut self.host, Some(&mut self.announcer));
        }
    }

    fn activate_toast_action(&mut self) {
        if self
            .queue
            .activate_action(&mut self.host, "undo")
            .is_some()
        {
            self.last_toast_event = self
                .queue
                .pump(&mut self.host, Some(&mut self.announcer));
        }
    }

    fn show_tooltip(&mut self) {
        self.tooltip
            .set_focused(true);
        self.clock
            .advance(Duration::from_millis(350));
        self.last_tooltip_event = self
            .tooltip
            .pump_with_announcer(&mut self.host, Some(&mut self.announcer));
    }

    fn hide_tooltip(&mut self) {
        self.tooltip
            .set_focused(false);
        self.last_tooltip_event = self
            .tooltip
            .pump(&mut self.host);
    }
}

impl StatefulWidget for FeedbackExample {
    type State = FeedbackExampleState;

    fn create_state(self) -> Self::State {
        FeedbackExampleState::initial()
    }
}

impl State<FeedbackExample> for FeedbackExampleState {
    fn init_state(&mut self, updater: StateUpdater<Self>) {
        self.updater = updater;
    }

    fn build(&self, ctx: &BuildContext) -> impl Widget {
        let app_theme = ThemeData::copied(ctx);
        let mut progress = ProgressIndicator::determinate(self.progress_value)
            .expect("the showcase progress value is always bounded");
        progress.set_motion_policy(self.motion_policy);
        let progress = progress
            .width(420.0)
            .height(10.0)
            .track_color(recessed_surface(&app_theme))
            .progress_color(app_theme.primary_color);
        let mut indeterminate = ProgressIndicator::indeterminate();
        indeterminate.set_motion_policy(self.motion_policy);
        let indeterminate = indeterminate
            .width(420.0)
            .height(10.0)
            .track_color(recessed_surface(&app_theme))
            .progress_color(
                app_theme
                    .primary_color
                    .with_alpha(0.62),
            );
        let spinner = self
            .spinner
            .size(32.0)
            .color(app_theme.primary_color);

        let progress_updater = self.updater;
        let advance_progress = action_button("+10%", app_theme, true, move || {
            progress_updater.set_state(|state| state.advance_progress())
        });
        let reset_updater = self.updater;
        let reset_progress = action_button("Reset", app_theme, false, move || {
            reset_updater.set_state(|state| state.reset_progress())
        });
        let motion_updater = self.updater;
        let toggle_motion = action_button(
            if self.motion_policy == MotionPolicy::Reduced {
                "Enable motion"
            } else {
                "Reduce motion"
            },
            app_theme,
            false,
            move || motion_updater.set_state(|state| state.toggle_motion()),
        );
        let spinner_updater = self.updater;
        let advance_spinner = action_button("Advance 250ms", app_theme, false, move || {
            spinner_updater.set_state(|state| state.advance_spinner())
        });

        let queue_updater = self.updater;
        let queue_toast = action_button("Queue toast", app_theme, true, move || {
            queue_updater.set_state(|state| state.queue_toast())
        });
        let advance_toast_updater = self.updater;
        let advance_toast = action_button("Advance 4s", app_theme, false, move || {
            advance_toast_updater.set_state(|state| state.advance_toast_clock())
        });
        let dismiss_updater = self.updater;
        let dismiss_toast = action_button("Dismiss", app_theme, false, move || {
            dismiss_updater.set_state(|state| state.dismiss_toast())
        });
        let action_updater = self.updater;
        let undo_toast = action_button("Undo action", app_theme, false, move || {
            action_updater.set_state(|state| state.activate_toast_action())
        });

        let show_updater = self.updater;
        let show_tooltip = action_button("Show tooltip", app_theme, true, move || {
            show_updater.set_state(|state| state.show_tooltip())
        });
        let hide_updater = self.updater;
        let hide_tooltip = action_button("Hide tooltip", app_theme, false, move || {
            hide_updater.set_state(|state| state.hide_tooltip())
        });

        let toast_summary = match self.queue.active() {
            Some(toast) => format!(
                "Visible: {} · pending: {} · remaining: {:?}",
                toast.message(),
                self.queue
                    .pending_len(),
                self.queue
                    .remaining()
            ),
            None => format!(
                "No toast visible · pending: {}",
                self.queue
                    .pending_len()
            ),
        };
        let tooltip_summary = format!(
            "{} · last event: {:?} · overlays hosted: {}",
            if self
                .tooltip
                .is_visible()
            {
                "Visible"
            } else {
                "Hidden"
            },
            self.last_tooltip_event,
            self.host
                .overlays
                .len()
        );
        let announcement_summary = self
            .announcer
            .last
            .as_deref()
            .map_or_else(
                || "No announcements yet".to_owned(),
                |text| format!("Last: {text}"),
            );

        Container::new()
            .width(Dimension::Percent(100.0))
            .height(Dimension::Percent(100.0))
            .child(
                Scrollable::new()
                    .axis(ScrollAxis::Vertical)
                    .vertical_scroll_bar(None)
                    .child(
                        Column::new()
                            .gaps(LayoutSpacing::all(Spacing::Px(14)))
                            .children([
                                Text::new("A compact, deterministic feedback lab")
                                    .text_style(
                                        TextStyle::new()
                                            .font_size(18)
                                            .font_weight(FontWeight::Bold)
                                            .color(app_theme.on_surface_color),
                                    )
                                    .boxed(),
                                Text::new(
                                    "Use the controls to drive retained state. Every popup is \
                                     owned by the host supplied to the controller.",
                                )
                                .wrapped()
                                .text_style(
                                    TextStyle::new()
                                        .font_size(14)
                                        .color(muted_text(&app_theme)),
                                )
                                .boxed(),
                                feedback_card(
                                    "Progress and motion",
                                    "Determinate and indeterminate indicators keep their visual state while motion preferences change.",
                                    Column::new()
                                        .gaps(LayoutSpacing::all(Spacing::Px(10)))
                                        .children([
                                            Row::new()
                                                .vertical_alignment(aimer::BoxAlignment::Center)
                                                .gaps(LayoutSpacing::all(Spacing::Px(18)))
                                                .children([
                                                    Column::new()
                                                        .gaps(LayoutSpacing::all(Spacing::Px(6)))
                                                        .children([
                                                            Text::new(format!(
                                                                "Determinate · {:.0}%",
                                                                self.progress_value * 100.0
                                                            ))
                                                            .text_style(
                                                                TextStyle::new()
                                                                    .font_size(13)
                                                                    .color(app_theme.on_surface_color),
                                                            )
                                                            .boxed(),
                                                            progress.boxed(),
                                                            Text::new("Indeterminate")
                                                                .text_style(
                                                                    TextStyle::new()
                                                                        .font_size(13)
                                                                        .color(muted_text(&app_theme)),
                                                                )
                                                                .boxed(),
                                                            indeterminate.boxed(),
                                                        ]),
                                                    Column::new()
                                                        .horizontal_alignment(aimer::BoxAlignment::Center)
                                                        .gaps(LayoutSpacing::all(Spacing::Px(4)))
                                                        .children([
                                                            spinner.boxed(),
                                                            Text::new(format!(
                                                                "Phase {:.2} · {:?}",
                                                                self.spinner.phase(),
                                                                self.motion_policy
                                                            ))
                                                            .text_style(
                                                                TextStyle::new()
                                                                    .font_size(12)
                                                                    .color(muted_text(&app_theme)),
                                                            )
                                                            .boxed(),
                                                        ]),
                                                ])
                                                .boxed(),
                                            Row::new()
                                                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                                .children([
                                                    advance_progress,
                                                    reset_progress,
                                                    toggle_motion,
                                                    advance_spinner,
                                                ])
                                                .boxed(),
                                        ]),
                                    app_theme,
                                ),
                                feedback_card(
                                    "Toasts and tooltips",
                                    "Queue, replace, timeout, action, and dismiss transient feedback through one explicit overlay host.",
                                    Row::new()
                                        .gaps(LayoutSpacing::all(Spacing::Px(14)))
                                        .children([
                                            Container::new()
                                                .width(Dimension::Percent(50.0))
                                                .child(
                                                    Column::new()
                                                        .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                                        .children([
                                                            status_banner(
                                                                self.queue.active().map_or(
                                                                    StatusKind::Info,
                                                                    |toast| toast_status_kind(toast.kind_value()),
                                                                ),
                                                                toast_summary,
                                                                app_theme,
                                                            ),
                                                            Text::new(format!(
                                                                "Last transition: {:?}",
                                                                self.last_toast_event
                                                            ))
                                                            .wrapped()
                                                            .text_style(
                                                                TextStyle::new()
                                                                    .font_size(12)
                                                                    .color(muted_text(&app_theme)),
                                                            )
                                                            .boxed(),
                                                            Row::new()
                                                                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                                                .children([
                                                                    queue_toast,
                                                                    advance_toast,
                                                                    dismiss_toast,
                                                                    undo_toast,
                                                                ])
                                                                .boxed(),
                                                        ]),
                                                )
                                                .boxed(),
                                            Container::new()
                                                .width(Dimension::Percent(50.0))
                                                .child(
                                                    Column::new()
                                                        .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                                        .children([
                                                            status_banner(
                                                                StatusKind::Info,
                                                                tooltip_summary,
                                                                app_theme,
                                                            ),
                                                            Row::new()
                                                                .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                                                .children([show_tooltip, hide_tooltip])
                                                                .boxed(),
                                                            Text::new(announcement_summary)
                                                                .wrapped()
                                                                .text_style(
                                                                    TextStyle::new()
                                                                        .font_size(12)
                                                                        .color(muted_text(&app_theme)),
                                                                )
                                                                .boxed(),
                                                        ]),
                                                )
                                                .boxed(),
                                        ]),
                                    app_theme,
                                ),
                                feedback_card(
                                    "Status presentation slots",
                                    "The same slot supports loading, success, warning, and error tones with assertive errors.",
                                    Column::new()
                                        .gaps(LayoutSpacing::all(Spacing::Px(8)))
                                        .children([
                                            status_banner(
                                                StatusKind::Loading,
                                                "A request is in progress",
                                                app_theme,
                                            ),
                                            status_banner(
                                                StatusKind::Success,
                                                "Saved successfully",
                                                app_theme,
                                            ),
                                            status_banner(
                                                StatusKind::Warning,
                                                "Review the queued notification",
                                                app_theme,
                                            ),
                                            status_banner(
                                                StatusKind::Error,
                                                "Errors are announced assertively",
                                                app_theme,
                                            ),
                                        ]),
                                    app_theme,
                                ),
                            ]),
                    ),
            )
    }
}

fn local_theme_provider<W: Widget + 'static>(child: W) -> impl Widget {
    let theme = ThemeData::dark()
        .primary_color(aimer::Color::Rgb(255, 107, 82))
        .on_primary_color(aimer::Color::WHITE)
        .background_color(aimer::Color::Rgb(25, 17, 16))
        .on_background_color(aimer::Color::Rgb(237, 226, 222))
        .surface_color(aimer::Color::Rgb(42, 28, 26))
        .on_surface_color(aimer::Color::Rgb(237, 226, 222));
    AnimatedTheme::new()
        .data(theme)
        .child(child)
}

#[inline]
fn muted_text(app_theme: &ThemeData) -> aimer::Color {
    app_theme
        .on_background_color
        .with_alpha(0.72)
}

#[inline]
fn raised_surface(app_theme: &ThemeData) -> aimer::Color {
    app_theme
        .surface_color
        .lighten(0.08)
}

#[inline]
fn recessed_surface(app_theme: &ThemeData) -> aimer::Color {
    app_theme
        .background_color
        .lighten(0.06)
}

fn status_banner(kind: StatusKind, message: impl Into<String>, app_theme: ThemeData) -> AnyWidget {
    StatusBanner::new(message)
        .kind(kind)
        .background_color(status_background(kind, app_theme))
        .foreground_color(app_theme.on_surface_color)
        .padding(
            LayoutSpacing::new()
                .top(10)
                .bottom(10)
                .left(12)
                .right(12),
        )
        .boxed()
}

fn status_background(kind: StatusKind, app_theme: ThemeData) -> aimer::Color {
    let (color, alpha) = match kind {
        StatusKind::Info => (app_theme.on_surface_color, 0.08),
        StatusKind::Loading => (app_theme.primary_color, 0.20),
        StatusKind::Success => (aimer::Color::Rgb(76, 175, 80), 0.20),
        StatusKind::Warning => (aimer::Color::Rgb(255, 179, 0), 0.20),
        StatusKind::Error => (aimer::Color::Rgb(244, 67, 54), 0.20),
    };
    color.with_alpha(alpha)
}

fn toast_status_kind(kind: ToastKind) -> StatusKind {
    match kind {
        ToastKind::Info => StatusKind::Info,
        ToastKind::Success => StatusKind::Success,
        ToastKind::Warning => StatusKind::Warning,
        ToastKind::Error => StatusKind::Error,
    }
}

fn feedback_card(
    title: &'static str,
    description: &'static str,
    body: impl Widget + 'static,
    app_theme: ThemeData,
) -> AnyWidget {
    Container::new()
        .width(Dimension::Percent(100.0))
        .padding(LayoutSpacing::all(Spacing::Px(16)))
        .box_decoration(
            BoxDecoration::new()
                .background_color(app_theme.surface_color)
                .border_radius(14),
        )
        .child(
            Column::new()
                .gaps(LayoutSpacing::all(Spacing::Px(9)))
                .children([
                    Text::new(title)
                        .text_style(
                            TextStyle::new()
                                .font_size(16)
                                .font_weight(FontWeight::Bold)
                                .color(app_theme.on_surface_color),
                        )
                        .boxed(),
                    Text::new(description)
                        .wrapped()
                        .text_style(
                            TextStyle::new()
                                .font_size(13)
                                .color(muted_text(&app_theme)),
                        )
                        .boxed(),
                    body.boxed(),
                ]),
        )
        .boxed()
}

fn action_button(
    label: &'static str,
    app_theme: ThemeData,
    primary: bool,
    on_press: impl Fn() + 'static,
) -> AnyWidget {
    let background = if primary {
        app_theme.primary_color
    } else {
        aimer::Color::Transparent
    };
    let foreground = if primary {
        app_theme.on_primary_color
    } else {
        app_theme.primary_color
    };
    Button::new()
        .on_press(on_press)
        .decoration(
            BoxDecoration::new()
                .background_color(background)
                .border_radius(8),
        )
        .hover_decoration(
            BoxDecoration::new()
                .background_color(if primary {
                    app_theme
                        .primary_color
                        .lighten(0.08)
                } else {
                    raised_surface(&app_theme)
                })
                .border_radius(8),
        )
        .press_decoration(
            BoxDecoration::new()
                .background_color(if primary {
                    app_theme
                        .primary_color
                        .darken(0.08)
                } else {
                    recessed_surface(&app_theme)
                })
                .border_radius(8),
        )
        .child(
            Container::new()
                .height(Dimension::Px(32.0))
                .padding(
                    LayoutSpacing::new()
                        .left(11)
                        .right(11),
                )
                .child(
                    Text::new(label)
                        .text_align(TextAlign::MidCenter)
                        .text_style(
                            TextStyle::new()
                                .font_size(12)
                                .font_weight(FontWeight::Bold)
                                .color(foreground),
                        ),
                ),
        )
        .boxed()
}

#[derive(Default)]
struct DemoOverlayHost {
    next_id: u64,
    overlays: Vec<(OverlayId, OverlayRequest)>,
}

impl OverlayHost for DemoOverlayHost {
    fn present(&mut self, request: OverlayRequest) -> OverlayId {
        self.next_id = self
            .next_id
            .saturating_add(1)
            .max(1);
        let id = OverlayId::new(self.next_id);
        self.overlays
            .push((id, request));
        id
    }

    fn update(&mut self, id: OverlayId, request: OverlayRequest) -> bool {
        let Some((_, current)) = self
            .overlays
            .iter_mut()
            .find(|(overlay_id, _)| *overlay_id == id)
        else {
            return false;
        };
        *current = request;
        true
    }

    fn dismiss(&mut self, id: OverlayId, _reason: DismissReason) -> bool {
        let Some(index) = self
            .overlays
            .iter()
            .position(|(overlay_id, _)| *overlay_id == id)
        else {
            return false;
        };
        self.overlays
            .remove(index);
        true
    }
}

#[derive(Default)]
struct DemoAnnouncer {
    last: Option<String>,
    count: usize,
}

impl Announcer for DemoAnnouncer {
    fn announce(&mut self, announcement: Announcement) {
        self.last = Some(
            announcement
                .text()
                .to_owned(),
        );
        self.count = self
            .count
            .saturating_add(1);
    }
}
