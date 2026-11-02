use aimer_picker::{
    CalendarError, CancelReason, Date, DateBounds, DatePicker, DatePickerError, DateRangePolicy,
    DateSelection, FocusRestorer, FocusTarget, OverlayAnchor, OverlayConsumer, OverlayRequest,
    PickerOutcome, PickerSession,
};

#[derive(Default)]
struct RecordingHost {
    next_handle: u32,
    presented: Vec<OverlayRequest>,
    dismissed: Vec<u32>,
}

impl OverlayConsumer for RecordingHost {
    type Handle = u32;

    fn present(&mut self, request: OverlayRequest) -> Self::Handle {
        self.next_handle += 1;
        self.presented.push(request);
        self.next_handle
    }

    fn dismiss(&mut self, handle: Self::Handle) {
        self.dismissed.push(handle);
    }
}

#[derive(Default)]
struct RecordingFocus {
    restored: Vec<FocusTarget>,
}

impl FocusRestorer for RecordingFocus {
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
fn session_opens_through_the_host_and_confirm_restores_focus() {
    let request = OverlayRequest::new(OverlayAnchor::new(3), true);
    let focus_target = FocusTarget::new(99);
    let mut session = PickerSession::new(11_u8);
    let mut host = RecordingHost::default();
    let mut overlay = session
        .open_with_overlay(&mut host, request, focus_target)
        .unwrap();
    let mut focus = RecordingFocus::default();

    session.set_draft(22).unwrap();
    assert_eq!(session.committed(), &11);
    assert_eq!(
        session
            .confirm_with_overlay(&mut overlay, &mut host, &mut focus)
            .unwrap(),
        PickerOutcome::Confirmed(22)
    );
    assert!(!session.is_open());
    assert_eq!(session.committed(), &22);
    assert_eq!(host.presented, vec![request]);
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![focus_target]);
}

#[test]
fn session_close_is_a_programmatic_cancel_that_rolls_back_the_draft() {
    let mut session = PickerSession::new(11_u8);
    session.open();
    session.set_draft(22).unwrap();

    assert_eq!(
        session.close().unwrap(),
        PickerOutcome::Cancelled {
            reason: CancelReason::Programmatic,
            value: 11,
        }
    );
    assert!(!session.is_open());
    assert_eq!(session.committed(), &11);
    assert_eq!(session.draft(), &11);
}

#[test]
fn date_picker_overlay_cancel_rolls_back_calendar_draft_and_restores_focus() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let maximum = Date::try_new(2024, 5, 20).unwrap();
    let bounds = DateBounds::new(Some(initial), Some(maximum)).unwrap();
    let request = OverlayRequest::new(OverlayAnchor::new(8), true);
    let focus_target = FocusTarget::new(100);
    let mut picker = DatePicker::try_new(Some(initial), bounds).unwrap();
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, focus_target)
        .unwrap();
    let mut focus = RecordingFocus::default();

    picker.select(maximum).unwrap();
    assert_eq!(picker.draft(), DateSelection::Single(Some(maximum)));
    assert_eq!(
        picker
            .cancel_with_overlay(
                &mut overlay,
                CancelReason::OutsideClick,
                &mut host,
                &mut focus,
            )
            .unwrap(),
        PickerOutcome::Cancelled {
            reason: CancelReason::OutsideClick,
            value: DateSelection::Single(Some(initial)),
        }
    );
    assert_eq!(picker.selection(), DateSelection::Single(Some(initial)));
    assert_eq!(picker.draft(), DateSelection::Single(Some(initial)));
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![focus_target]);
}

#[test]
fn date_picker_overlay_keeps_the_draft_when_escape_dismissal_is_disabled() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let maximum = Date::try_new(2024, 5, 20).unwrap();
    let bounds = DateBounds::new(Some(initial), Some(maximum)).unwrap();
    let request = OverlayRequest::new(OverlayAnchor::new(8), true).dismiss_on_escape(false);
    let mut picker = DatePicker::try_new(Some(initial), bounds).unwrap();
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, FocusTarget::new(100))
        .unwrap();
    let mut focus = RecordingFocus::default();

    picker.select(maximum).unwrap();
    assert_eq!(
        picker.cancel_with_overlay(&mut overlay, CancelReason::Escape, &mut host, &mut focus,),
        Err(DatePickerError::DismissalNotAllowed(CancelReason::Escape))
    );
    assert!(picker.is_open());
    assert_eq!(picker.draft(), DateSelection::Single(Some(maximum)));
    assert!(overlay.is_presented());
    assert!(host.dismissed.is_empty());
    assert!(focus.restored.is_empty());
}

#[test]
fn date_picker_overlay_confirm_commits_the_calendar_draft_and_closes() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let maximum = Date::try_new(2024, 5, 20).unwrap();
    let bounds = DateBounds::new(Some(initial), Some(maximum)).unwrap();
    let request = OverlayRequest::new(OverlayAnchor::new(8), true);
    let focus_target = FocusTarget::new(100);
    let mut picker = DatePicker::try_new(Some(initial), bounds).unwrap();
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, focus_target)
        .unwrap();
    let mut focus = RecordingFocus::default();

    picker.select(maximum).unwrap();
    assert_eq!(
        picker
            .confirm_with_overlay(&mut overlay, &mut host, &mut focus)
            .unwrap(),
        PickerOutcome::Confirmed(DateSelection::Single(Some(maximum)))
    );
    assert!(!picker.is_open());
    assert_eq!(picker.selection(), DateSelection::Single(Some(maximum)));
    assert_eq!(host.dismissed, vec![1]);
    assert_eq!(focus.restored, vec![focus_target]);
}

#[test]
fn date_picker_overlay_does_not_close_on_an_incomplete_range_confirmation() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let maximum = Date::try_new(2024, 5, 20).unwrap();
    let bounds = DateBounds::new(Some(initial), Some(maximum)).unwrap();
    let selection = DateSelection::Range {
        start: Some(initial),
        end: None,
    };
    let request = OverlayRequest::new(OverlayAnchor::new(8), true);
    let mut picker =
        DatePicker::try_range(selection, bounds, DateRangePolicy::inclusive()).unwrap();
    let mut host = RecordingHost::default();
    let mut overlay = picker
        .open_with_overlay(&mut host, request, FocusTarget::new(100))
        .unwrap();
    let mut focus = RecordingFocus::default();

    assert_eq!(
        picker.confirm_with_overlay(&mut overlay, &mut host, &mut focus),
        Err(DatePickerError::IncompleteRange)
    );
    assert!(picker.is_open());
    assert_eq!(picker.draft(), selection);
    assert!(overlay.is_presented());
    assert!(host.dismissed.is_empty());
    assert!(focus.restored.is_empty());
}

#[test]
fn date_picker_stays_closed_when_the_overlay_host_is_missing_or_unsupported() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let bounds = DateBounds::new(Some(initial), None).unwrap();
    let request = OverlayRequest::new(OverlayAnchor::new(8), true);

    let mut missing_picker = DatePicker::try_new(Some(initial), bounds).unwrap();
    let mut missing = CapabilityHost {
        available: false,
        supported: true,
        presented: 0,
    };
    assert_eq!(
        missing_picker.open_with_overlay(&mut missing, request, FocusTarget::new(100)),
        Err(DatePickerError::MissingHost)
    );
    assert!(!missing_picker.is_open());
    assert_eq!(missing.presented, 0);

    let mut unsupported_picker = DatePicker::try_new(Some(initial), bounds).unwrap();
    let mut unsupported = CapabilityHost {
        available: true,
        supported: false,
        presented: 0,
    };
    assert_eq!(
        unsupported_picker.open_with_overlay(&mut unsupported, request, FocusTarget::new(100)),
        Err(DatePickerError::UnsupportedHost)
    );
    assert!(!unsupported_picker.is_open());
    assert_eq!(unsupported.presented, 0);
}

#[test]
fn date_picker_exposes_disabled_dates_through_its_calendar_policy() {
    let initial = Date::try_new(2024, 5, 15).unwrap();
    let disabled = Date::try_new(2024, 5, 20).unwrap();
    let mut picker = DatePicker::new(Some(initial)).with_disabled_dates([disabled]);

    assert!(picker.is_date_disabled(disabled));
    picker.open();
    assert_eq!(
        picker.select(disabled),
        Err(DatePickerError::Calendar(CalendarError::DisabledDate(
            disabled
        )))
    );
    assert_eq!(picker.draft(), DateSelection::Single(Some(initial)));
}
