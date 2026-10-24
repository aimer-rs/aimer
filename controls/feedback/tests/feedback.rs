use std::time::Duration;

use aimer_feedback::{
    Announcement, AnnouncementPriority, Announcer, DismissReason, FocusTarget, ManualClock,
    MotionPolicy, OverlayHost, OverlayId, OverlayLifecycle, OverlayModality, OverlayRequest,
    ProgressError, ProgressIndicator, ProgressState, Spinner, StatusBanner, StatusKind, Toast,
    ToastAction, ToastQueue, ToastQueueEvent, Tooltip, TooltipController, TooltipEvent,
    TooltipTouch, TouchPolicy,
};

#[derive(Default)]
struct TestHost {
    next_id: u64,
    presented: Vec<OverlayRequest>,
    dismissed: Vec<(OverlayId, DismissReason)>,
    restored: Vec<FocusTarget>,
    reject_dismiss: bool,
}

impl OverlayHost for TestHost {
    fn present(&mut self, request: OverlayRequest) -> OverlayId {
        self.next_id += 1;
        self.presented.push(request);
        OverlayId::new(self.next_id)
    }

    fn dismiss(&mut self, id: OverlayId, reason: DismissReason) -> bool {
        self.dismissed.push((id, reason));
        !self.reject_dismiss
    }

    fn restore_focus(&mut self, target: FocusTarget) {
        self.restored.push(target);
    }
}

#[derive(Default)]
struct TestAnnouncer {
    announcements: Vec<Announcement>,
}

impl Announcer for TestAnnouncer {
    fn announce(&mut self, announcement: Announcement) {
        self.announcements.push(announcement);
    }
}

#[test]
fn queue_public_seam_starts_ordered_toasts_and_announces_them() {
    let mut queue = ToastQueue::new(ManualClock::new());
    queue.enqueue(Toast::new("first"));
    queue.enqueue(Toast::new("second"));
    let mut host = TestHost::default();
    let mut announcer = TestAnnouncer::default();

    queue.pump(&mut host, Some(&mut announcer));

    assert_eq!(queue.active().map(Toast::message), Some("first"));
    assert_eq!(host.presented[0].text(), "first");
    assert_eq!(announcer.announcements[0].text(), "first");
    assert_eq!(
        announcer.announcements[0].priority_value(),
        AnnouncementPriority::Polite
    );
}

#[test]
fn queue_timeout_excludes_a_paused_interval_and_advances_in_order() {
    let clock = ManualClock::new();
    let mut queue = ToastQueue::new(clock.clone());
    queue.enqueue(Toast::new("first").timeout(Duration::from_secs(1)));
    queue.enqueue(Toast::new("second").persistent());
    let mut host = TestHost::default();

    assert!(matches!(
        queue.pump(&mut host, None),
        ToastQueueEvent::Presented(_)
    ));
    clock.advance(Duration::from_millis(500));
    assert!(queue.pause());
    clock.advance(Duration::from_secs(2));
    assert!(matches!(queue.pump(&mut host, None), ToastQueueEvent::Idle));
    assert_eq!(queue.remaining(), Some(Duration::from_millis(500)));

    assert!(queue.resume());
    clock.advance(Duration::from_millis(500));
    assert!(matches!(
        queue.pump(&mut host, None),
        ToastQueueEvent::Dismissed {
            reason: DismissReason::Timeout,
            ..
        }
    ));
    assert_eq!(host.dismissed[0].1, DismissReason::Timeout);
    assert!(matches!(
        queue.pump(&mut host, None),
        ToastQueueEvent::Presented(_)
    ));
    assert_eq!(queue.active().map(Toast::message), Some("second"));
}

#[test]
fn queue_replacement_updates_the_active_overlay_and_actions_dismiss_it() {
    let clock = ManualClock::new();
    let mut queue = ToastQueue::new(clock);
    queue.enqueue(
        Toast::new("saving")
            .replacement_key("save")
            .action(ToastAction::new("Undo", "undo")),
    );
    let mut host = TestHost::default();
    queue.pump(&mut host, None);

    let replacement_id = queue.enqueue(
        Toast::new("saved")
            .kind(aimer_feedback::ToastKind::Success)
            .replacement_key("save"),
    );
    assert_eq!(
        queue.active_handles().map(|handles| handles.0),
        Some(replacement_id)
    );
    assert!(matches!(
        queue.pump(&mut host, None),
        ToastQueueEvent::Updated(_)
    ));
    assert_eq!(queue.active().map(Toast::message), Some("saved"));
    assert_eq!(host.dismissed[0].1, DismissReason::Replaced);

    assert!(queue.activate_action(&mut host, "undo").is_none());
    assert!(queue.dismiss_active(&mut host, DismissReason::Programmatic));
}

#[test]
fn queue_keeps_active_state_when_the_host_rejects_dismissal() {
    let mut queue = ToastQueue::new(ManualClock::new());
    queue.enqueue(Toast::new("still visible").persistent());
    let mut host = TestHost {
        reject_dismiss: true,
        ..TestHost::default()
    };
    queue.pump(&mut host, None);

    assert!(!queue.dismiss_active(&mut host, DismissReason::Programmatic));
    assert_eq!(queue.active().map(Toast::message), Some("still visible"));
    assert!(queue.active_handles().is_some());
}

#[test]
fn queue_keeps_the_previous_overlay_when_replacement_cannot_be_dismissed() {
    let mut queue = ToastQueue::new(ManualClock::new());
    queue.enqueue(Toast::new("old").replacement_key("job"));
    let mut host = TestHost::default();
    queue.pump(&mut host, None);

    host.reject_dismiss = true;
    queue.enqueue(Toast::new("new").replacement_key("job"));
    assert!(matches!(queue.pump(&mut host, None), ToastQueueEvent::Idle));
    assert_eq!(queue.active().map(Toast::message), Some("old"));

    host.reject_dismiss = false;
    assert!(matches!(
        queue.pump(&mut host, None),
        ToastQueueEvent::Updated(_)
    ));
    assert_eq!(queue.active().map(Toast::message), Some("new"));
}

#[test]
fn overlay_lifecycle_restores_focus_after_an_accepted_modal_dismissal() {
    let mut lifecycle = OverlayLifecycle::new();
    let mut host = TestHost::default();
    let focus = FocusTarget::new(44);
    let id = lifecycle.present(
        &mut host,
        OverlayRequest::new(aimer_feedback::OverlayKind::Custom, "dialog")
            .modality(OverlayModality::Modal)
            .restore_focus(focus),
    );

    assert_eq!(lifecycle.active_id(), Some(id));
    assert!(lifecycle.dismiss(&mut host, DismissReason::Programmatic));
    assert_eq!(lifecycle.active_id(), None);
    assert_eq!(host.restored, vec![focus]);
}

#[test]
fn overlay_lifecycle_does_not_drop_state_when_dismissal_is_rejected() {
    let mut lifecycle = OverlayLifecycle::new();
    let mut host = TestHost {
        reject_dismiss: true,
        ..TestHost::default()
    };
    let id = lifecycle.present(
        &mut host,
        OverlayRequest::new(aimer_feedback::OverlayKind::Custom, "dialog"),
    );

    assert!(!lifecycle.dismiss(&mut host, DismissReason::Programmatic));
    assert_eq!(lifecycle.active_id(), Some(id));
}

#[test]
fn overlay_lifecycle_keeps_previous_request_when_replacement_is_rejected() {
    let mut lifecycle = OverlayLifecycle::new();
    let mut host = TestHost::default();
    let first = lifecycle.present(
        &mut host,
        OverlayRequest::new(aimer_feedback::OverlayKind::Custom, "first"),
    );

    host.reject_dismiss = true;
    let returned = lifecycle.present(
        &mut host,
        OverlayRequest::new(aimer_feedback::OverlayKind::Custom, "second"),
    );

    assert_eq!(returned, first);
    assert_eq!(lifecycle.active_id(), Some(first));
    assert_eq!(
        lifecycle.active_request().map(OverlayRequest::text),
        Some("first")
    );
    assert_eq!(host.presented.len(), 1);
}

#[test]
fn tooltip_waits_for_hover_delay_and_dismisses_when_the_trigger_leaves() {
    let clock = ManualClock::new();
    let mut tooltip = TooltipController::new(
        Tooltip::new("More information")
            .delay(Duration::from_millis(500))
            .touch_policy(TouchPolicy::Never),
        clock.clone(),
    );
    tooltip.set_anchor(aimer_feedback::Rect::new(10.0, 10.0, 20.0, 20.0));
    let mut host = TestHost::default();

    tooltip.set_hovered(true);
    assert!(matches!(tooltip.pump(&mut host), TooltipEvent::Pending));
    clock.advance(Duration::from_millis(499));
    assert!(matches!(tooltip.pump(&mut host), TooltipEvent::Pending));
    clock.advance(Duration::from_millis(1));
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Presented(_)
    ));
    assert_eq!(
        host.presented[0].kind(),
        aimer_feedback::OverlayKind::Tooltip
    );
    assert_eq!(
        host.presented[0].modality_value(),
        aimer_feedback::OverlayModality::NonModal
    );
    assert!(host.presented[0].dismiss_on_outside_press_value());

    tooltip.set_hovered(false);
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Dismissed {
            reason: DismissReason::TriggerExit,
            ..
        }
    ));
}

#[test]
fn visible_tooltip_reanchors_through_the_same_host_lifecycle() {
    let clock = ManualClock::new();
    let mut tooltip = TooltipController::new(Tooltip::new("Anchored").delay(Duration::ZERO), clock);
    let mut host = TestHost::default();
    tooltip.set_anchor(aimer_feedback::Rect::new(4.0, 4.0, 12.0, 12.0));
    tooltip.set_hovered(true);
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Presented(_)
    ));

    tooltip.set_anchor(aimer_feedback::Rect::new(80.0, 80.0, 12.0, 12.0));
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Presented(_)
    ));
    assert_eq!(host.dismissed[0].1, DismissReason::Replaced);
    assert_eq!(host.presented[1].anchor_value().unwrap().x(), 80.0);
}

#[test]
fn tooltip_keeps_visible_state_when_the_host_rejects_dismissal() {
    let clock = ManualClock::new();
    let mut tooltip =
        TooltipController::new(Tooltip::new("Still visible").delay(Duration::ZERO), clock);
    let mut host = TestHost::default();
    tooltip.set_hovered(true);
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Presented(_)
    ));

    host.reject_dismiss = true;
    tooltip.set_hovered(false);
    assert!(matches!(tooltip.pump(&mut host), TooltipEvent::Idle));
    assert!(tooltip.is_visible());

    host.reject_dismiss = false;
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Dismissed {
            reason: DismissReason::TriggerExit,
            ..
        }
    ));
    assert!(!tooltip.is_visible());
}

#[test]
fn immediate_touch_policy_can_show_without_hover_and_hides_on_touch_end() {
    let clock = ManualClock::new();
    let mut tooltip = TooltipController::new(
        Tooltip::new("Touch help")
            .delay(Duration::from_secs(30))
            .touch_policy(TouchPolicy::Immediate)
            .show_on_focus(false),
        clock,
    );
    let mut host = TestHost::default();

    tooltip.touch(TooltipTouch::Started);
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Presented(_)
    ));
    tooltip.touch(TooltipTouch::Ended);
    assert!(matches!(
        tooltip.pump(&mut host),
        TooltipEvent::Dismissed {
            reason: DismissReason::TouchEnd,
            ..
        }
    ));
}

#[test]
fn focused_tooltip_can_announce_through_the_explicit_accessibility_adapter() {
    let clock = ManualClock::new();
    let mut tooltip = TooltipController::new(
        Tooltip::new("Keyboard help")
            .delay(Duration::ZERO)
            .show_on_hover(false),
        clock,
    );
    let mut host = TestHost::default();
    let mut announcer = TestAnnouncer::default();

    tooltip.set_focused(true);
    assert!(matches!(
        tooltip.pump_with_announcer(&mut host, Some(&mut announcer)),
        TooltipEvent::Presented(_)
    ));
    assert_eq!(announcer.announcements[0].text(), "Keyboard help");
}

#[test]
fn progress_accepts_finite_boundaries_and_rejects_invalid_fractions() {
    let mut progress = ProgressIndicator::determinate(0.0).expect("zero is a valid boundary");
    assert_eq!(progress.state(), ProgressState::Determinate(0.0));
    assert_eq!(progress.fraction(), Some(0.0));
    assert_eq!(progress.semantics().current, Some(0.0));
    assert_eq!(progress.semantics().min, 0.0);
    assert_eq!(progress.semantics().max, 1.0);

    progress
        .set_determinate(1.0)
        .expect("one is a valid boundary");
    assert_eq!(progress.fraction(), Some(1.0));
    assert_eq!(
        ProgressIndicator::determinate(-0.01),
        Err(ProgressError::OutOfRange)
    );
    assert_eq!(
        ProgressIndicator::determinate(1.01),
        Err(ProgressError::OutOfRange)
    );
    assert_eq!(
        ProgressIndicator::determinate(f32::NAN),
        Err(ProgressError::NonFinite)
    );

    progress.set_indeterminate();
    assert_eq!(progress.state(), ProgressState::Indeterminate);
    assert_eq!(progress.fraction(), None);
}

#[test]
fn reduced_motion_freezes_spinner_phase_without_changing_its_state_model() {
    let mut spinner = Spinner::with_period(Duration::from_secs(1)).expect("period is positive");
    spinner.advance(Duration::from_millis(250));
    assert_eq!(spinner.phase(), 0.25);

    spinner.set_motion_policy(MotionPolicy::Reduced);
    spinner.advance(Duration::from_millis(500));
    assert_eq!(spinner.phase(), 0.25);

    spinner.set_motion_policy(MotionPolicy::Full);
    spinner.advance(Duration::from_millis(750));
    assert_eq!(spinner.phase(), 0.0);
}

#[test]
fn feedback_indicators_and_status_banners_are_composable_widgets() {
    fn assert_widget(_widget: impl aimer_widget::Widget) {}

    assert_widget(ProgressIndicator::determinate(0.5).unwrap());
    assert_widget(Spinner::new());
    assert_widget(Tooltip::new("help"));
    assert_widget(Toast::new("saved"));
    assert_widget(StatusBanner::new("Saved").kind(StatusKind::Success));
    assert_widget(
        aimer_feedback::FeedbackSlot::new(StatusKind::Loading).child(aimer_container::ZeroSizedBox),
    );
    assert_eq!(
        StatusBanner::new("failure")
            .kind(StatusKind::Error)
            .announcement()
            .priority_value(),
        AnnouncementPriority::Assertive
    );
}

#[test]
fn status_banner_accepts_theme_visual_overrides() {
    let background = aimer_widget::base::Color::Rgba(18, 24, 32, 230);
    let foreground = aimer_widget::base::Color::Rgba(245, 247, 250, 255);
    let padding = aimer_style::LayoutSpacing::new()
        .top(10)
        .bottom(8)
        .left(14)
        .right(14);
    let banner = StatusBanner::success("Saved")
        .background_color(background)
        .foreground_color(foreground)
        .padding(padding);

    assert_eq!(banner.kind_value(), StatusKind::Success);
    assert_eq!(banner.background_color_value(), Some(background));
    assert_eq!(banner.foreground_color_value(), foreground);
    assert_eq!(banner.padding_value(), padding);
}
