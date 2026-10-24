use aimer_picker::{
    CancelReason, ColorChannel, ColorKey, ColorPicker, Date, DateTime, DateTimePicker,
    DateTimePickerPolicy, FocusTarget, Hsva, OverlayAnchor, OverlayConsumer, OverlayRequest,
    PickerOutcome, PickerOverlay, PickerSessionError, TimeOfDay, TimeZonePolicy,
};

#[derive(Default)]
struct RecordingHost {
    next_handle: u32,
    dismissed: Vec<u32>,
}

impl OverlayConsumer for RecordingHost {
    type Handle = u32;

    fn present(&mut self, _request: OverlayRequest) -> Self::Handle {
        self.next_handle += 1;
        self.next_handle
    }

    fn dismiss(&mut self, handle: Self::Handle) {
        self.dismissed.push(handle);
    }
}

struct RecordingFocus {
    restored: Vec<FocusTarget>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CloneOnlyHandle(u32);

struct CloneOnlyHost;

impl OverlayConsumer for CloneOnlyHost {
    type Handle = CloneOnlyHandle;

    fn present(&mut self, _request: OverlayRequest) -> Self::Handle {
        CloneOnlyHandle(1)
    }

    fn dismiss(&mut self, _handle: Self::Handle) {}
}

impl aimer_picker::FocusRestorer for RecordingFocus {
    fn restore_focus(&mut self, target: FocusTarget) {
        self.restored.push(target);
    }
}

struct CapabilityHost {
    available: bool,
    supported: bool,
    presented: u32,
}

impl OverlayConsumer for CapabilityHost {
    type Handle = u32;

    fn present(&mut self, _request: OverlayRequest) -> Self::Handle {
        self.presented += 1;
        self.presented
    }

    fn dismiss(&mut self, _handle: Self::Handle) {}

    fn is_available(&self) -> bool {
        self.available
    }

    fn supports(&self, _request: OverlayRequest) -> bool {
        self.supported
    }
}

#[test]
fn overlay_request_controls_escape_and_outside_click_dismissal() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), true)
        .dismiss_on_escape(false)
        .dismiss_on_outside_click(true);
    let mut host = RecordingHost::default();
    let overlay = PickerOverlay::present(&mut host, request, FocusTarget::new(11));

    assert!(!overlay.should_dismiss(CancelReason::Escape));
    assert!(overlay.should_dismiss(CancelReason::OutsideClick));
    assert!(overlay.should_dismiss(CancelReason::Programmatic));
}

#[test]
fn overlay_dismissal_restores_focus_once_and_honors_the_policy() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), true).dismiss_on_escape(false);
    let mut host = RecordingHost {
        next_handle: 0,
        dismissed: Vec::new(),
    };
    let mut overlay = PickerOverlay::present(&mut host, request, FocusTarget::new(11));
    let mut focus = RecordingFocus {
        restored: Vec::new(),
    };

    assert!(!overlay.dismiss(CancelReason::Escape, &mut host, &mut focus));
    assert!(overlay.is_presented());
    assert!(overlay.dismiss(CancelReason::OutsideClick, &mut host, &mut focus));
    assert!(!overlay.is_presented());
    assert!(!overlay.dismiss(CancelReason::Programmatic, &mut host, &mut focus));
    overlay.close(&mut host, &mut focus);
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![FocusTarget::new(11)]);
}

#[test]
fn external_dismissal_acknowledgement_closes_without_repeating_host_dismissal() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), true);
    let mut host = RecordingHost::default();
    let mut overlay = PickerOverlay::present(&mut host, request, FocusTarget::new(11));
    let mut focus = RecordingFocus {
        restored: Vec::new(),
    };

    assert!(overlay.acknowledge_external_dismissal(CancelReason::Escape, &mut focus));
    assert!(!overlay.is_presented());
    assert!(overlay.acknowledge_external_dismissal(CancelReason::Escape, &mut focus) == false);
    assert!(host.dismissed.is_empty());
    assert_eq!(focus.restored, vec![FocusTarget::new(11)]);
}

#[test]
fn external_dismissal_acknowledgement_honors_the_request_policy() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), true).dismiss_on_escape(false);
    let mut host = RecordingHost::default();
    let mut overlay = PickerOverlay::present(&mut host, request, FocusTarget::new(11));
    let mut focus = RecordingFocus {
        restored: Vec::new(),
    };

    assert!(!overlay.acknowledge_external_dismissal(CancelReason::Escape, &mut focus));
    assert!(overlay.is_presented());
    assert!(focus.restored.is_empty());
}

#[test]
fn overlay_accepts_a_clone_only_host_handle() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), false);
    let mut host = CloneOnlyHost;
    let overlay = PickerOverlay::present(&mut host, request, FocusTarget::new(11));

    assert_eq!(overlay.handle(), CloneOnlyHandle(1));
}

#[test]
fn overlay_presentation_reports_missing_and_unsupported_hosts_without_side_effects() {
    let request = OverlayRequest::new(OverlayAnchor::new(7), true);

    let mut missing = CapabilityHost {
        available: false,
        supported: true,
        presented: 0,
    };
    assert_eq!(
        PickerOverlay::try_present(&mut missing, request, FocusTarget::new(11)),
        Err(PickerSessionError::MissingHost)
    );
    assert_eq!(missing.presented, 0);

    let mut unsupported = CapabilityHost {
        available: true,
        supported: false,
        presented: 0,
    };
    assert_eq!(
        PickerOverlay::try_present(&mut unsupported, request, FocusTarget::new(11)),
        Err(PickerSessionError::UnsupportedHost)
    );
    assert_eq!(unsupported.presented, 0);
}

#[test]
fn date_time_picker_overlay_confirmation_commits_and_restores_focus() {
    let timezone = TimeZonePolicy::fixed_offset(330).unwrap();
    let date = Date::try_new(2024, 5, 15).unwrap();
    let initial =
        DateTime::try_new(date, TimeOfDay::try_new(9, 30, 0, 0).unwrap(), timezone).unwrap();
    let changed_time = TimeOfDay::try_new(10, 45, 0, 0).unwrap();
    let mut picker =
        DateTimePicker::try_new(Some(initial), DateTimePickerPolicy::unbounded(timezone)).unwrap();
    let request = OverlayRequest::new(OverlayAnchor::new(12), true);
    let focus_target = FocusTarget::new(34);
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, focus_target)
        .unwrap();

    picker.set_time(changed_time).unwrap();
    let mut focus = RecordingFocus {
        restored: Vec::new(),
    };
    let expected = DateTime::try_new(date, changed_time, timezone).unwrap();
    assert_eq!(
        picker
            .confirm_with_overlay(&mut overlay, &mut host, &mut focus)
            .unwrap(),
        PickerOutcome::Confirmed(Some(expected))
    );
    assert_eq!(picker.value(), Some(expected));
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![focus_target]);
}

#[test]
fn color_picker_overlay_escape_rolls_back_and_restores_focus() {
    let initial = Hsva::try_new(120, 40, 80, 100).unwrap();
    let mut picker = ColorPicker::new(initial, true);
    let request = OverlayRequest::new(OverlayAnchor::new(13), true);
    let focus_target = FocusTarget::new(35);
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, focus_target)
        .unwrap();

    picker
        .handle_key(ColorChannel::Hue, ColorKey::Increase)
        .unwrap();
    assert_ne!(picker.draft(), initial);
    let mut focus = RecordingFocus {
        restored: Vec::new(),
    };
    assert_eq!(
        picker
            .cancel_with_overlay(&mut overlay, CancelReason::Escape, &mut host, &mut focus,)
            .unwrap(),
        PickerOutcome::Cancelled {
            reason: CancelReason::Escape,
            value: initial,
        }
    );
    assert_eq!(picker.value(), initial);
    assert_eq!(picker.draft(), initial);
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![focus_target]);
}
