//! Explicit seams for picker sessions, overlays, and focus restoration.

use core::fmt;

/// Why an open picker was dismissed without committing its draft.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelReason {
    /// The user pressed Escape or an equivalent cancel key.
    Escape,
    /// The user activated a point outside the picker.
    OutsideClick,
    /// The caller dismissed the picker without a user gesture.
    Programmatic,
}

/// The result of closing a picker session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerOutcome<T> {
    /// The draft was accepted and became the committed value.
    Confirmed(T),
    /// The draft was discarded and the previous committed value remains.
    Cancelled {
        /// The event that caused dismissal.
        reason: CancelReason,
        /// The committed value retained after cancellation.
        value: T,
    },
}

/// Error returned when a session or its overlay cannot perform an operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerSessionError {
    /// The session must be opened before it can be edited or closed.
    Closed,
    /// No overlay host is installed for the presentation request.
    MissingHost,
    /// An installed overlay host cannot represent the presentation request.
    UnsupportedHost,
    /// The overlay policy does not allow this user-driven dismissal reason.
    DismissalNotAllowed(CancelReason),
}

impl fmt::Display for PickerSessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("picker session is closed"),
            Self::MissingHost => formatter.write_str("picker overlay host is missing"),
            Self::UnsupportedHost => formatter.write_str("picker overlay host is unsupported"),
            Self::DismissalNotAllowed(reason) => {
                write!(
                    formatter,
                    "picker overlay dismissal is disabled for {reason:?}"
                )
            }
        }
    }
}

impl std::error::Error for PickerSessionError {}

/// Transactional committed/draft state shared by picker controls.
///
/// Opening copies the committed value into a draft. Confirmation commits the
/// draft, while every cancellation reason restores the committed value. The
/// session has no overlay or focus side effects; those are injected through
/// [`OverlayConsumer`] and [`crate::FocusRestorer`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerSession<T> {
    committed: T,
    draft: T,
    open: bool,
}

impl<T: Clone> PickerSession<T> {
    /// Creates a closed session with `value` as both committed and draft state.
    pub fn new(value: T) -> Self {
        Self {
            committed: value.clone(),
            draft: value,
            open: false,
        }
    }

    /// Opens the session and resets its draft from the committed value.
    pub fn open(&mut self) {
        self.draft = self.committed.clone();
        self.open = true;
    }

    /// Returns whether the session is currently editable.
    #[inline]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// Returns the last confirmed value.
    #[inline]
    pub const fn committed(&self) -> &T {
        &self.committed
    }

    /// Returns the current uncommitted draft.
    #[inline]
    pub const fn draft(&self) -> &T {
        &self.draft
    }

    /// Replaces the draft while the session is open.
    pub fn set_draft(&mut self, value: T) -> Result<(), PickerSessionError> {
        if !self.open {
            return Err(PickerSessionError::Closed);
        }
        self.draft = value;
        Ok(())
    }

    /// Commits the draft and closes the session.
    pub fn confirm(&mut self) -> Result<PickerOutcome<T>, PickerSessionError> {
        if !self.open {
            return Err(PickerSessionError::Closed);
        }
        self.committed = self.draft.clone();
        self.open = false;
        Ok(PickerOutcome::Confirmed(self.committed.clone()))
    }

    /// Discards the draft, closes the session, and reports the retained value.
    pub fn cancel(&mut self, reason: CancelReason) -> Result<PickerOutcome<T>, PickerSessionError> {
        if !self.open {
            return Err(PickerSessionError::Closed);
        }
        self.draft = self.committed.clone();
        self.open = false;
        Ok(PickerOutcome::Cancelled {
            reason,
            value: self.committed.clone(),
        })
    }

    /// Closes an open session as a programmatic cancellation.
    pub fn close(&mut self) -> Result<PickerOutcome<T>, PickerSessionError> {
        self.cancel(CancelReason::Programmatic)
    }

    /// Opens the session only after the host accepts the overlay request.
    ///
    /// A missing or unsupported host leaves the session closed and leaves the
    /// committed and draft values untouched.
    pub fn open_with_overlay<C: OverlayConsumer>(
        &mut self,
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<PickerOverlay<C::Handle>, PickerSessionError> {
        let overlay = PickerOverlay::try_present(consumer, request, restore_focus)?;
        self.open();
        Ok(overlay)
    }

    /// Confirms the draft and dismisses the associated overlay.
    ///
    /// A closed session still dismisses the supplied overlay before returning
    /// [`PickerSessionError::Closed`], preventing a stale host layer from
    /// leaking focus or presentation state.
    pub fn confirm_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<T>, PickerSessionError> {
        if !self.is_open() {
            let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
            return Err(PickerSessionError::Closed);
        }
        let outcome = self.confirm()?;
        let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
        Ok(outcome)
    }

    /// Cancels the draft according to the overlay's dismissal policy and
    /// dismisses the associated overlay when allowed.
    pub fn cancel_with_overlay<C: OverlayConsumer, F: FocusRestorer>(
        &mut self,
        overlay: &mut PickerOverlay<C::Handle>,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> Result<PickerOutcome<T>, PickerSessionError> {
        if !self.is_open() {
            let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
            return Err(PickerSessionError::Closed);
        }
        if !overlay.should_dismiss(reason) {
            return Err(PickerSessionError::DismissalNotAllowed(reason));
        }
        let outcome = self.cancel(reason)?;
        let _ = overlay.dismiss(reason, consumer, focus);
        Ok(outcome)
    }
}

/// Stable identifier for the control that opened a picker.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FocusTarget(u64);

impl FocusTarget {
    /// Creates a focus target from an application-owned stable identifier.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the application-owned identifier.
    #[inline]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// Stable identifier for an element that anchors an overlay.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OverlayAnchor(u64);

impl OverlayAnchor {
    /// Creates an overlay anchor from an application-owned stable identifier.
    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    /// Returns the application-owned identifier.
    #[inline]
    pub const fn id(self) -> u64 {
        self.0
    }
}

/// The placement-independent data an overlay host needs to present a picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayRequest {
    anchor: OverlayAnchor,
    modal: bool,
    dismiss_on_escape: bool,
    dismiss_on_outside_click: bool,
}

impl OverlayRequest {
    /// Creates a request associated with `anchor`.
    pub const fn new(anchor: OverlayAnchor, modal: bool) -> Self {
        Self {
            anchor,
            modal,
            dismiss_on_escape: true,
            dismiss_on_outside_click: true,
        }
    }

    /// Returns the anchor used for placement.
    #[inline]
    pub const fn anchor(self) -> OverlayAnchor {
        self.anchor
    }

    /// Returns whether the overlay should trap input as a modal surface.
    #[inline]
    pub const fn modal(self) -> bool {
        self.modal
    }

    /// Controls whether the host should dismiss this request on `Escape`.
    #[inline]
    pub const fn dismiss_on_escape(mut self, dismiss: bool) -> Self {
        self.dismiss_on_escape = dismiss;
        self
    }

    /// Controls whether the host should dismiss this request on an outside
    /// press.
    #[inline]
    pub const fn dismiss_on_outside_click(mut self, dismiss: bool) -> Self {
        self.dismiss_on_outside_click = dismiss;
        self
    }

    /// Controls whether the host should dismiss this request on an outside
    /// press. This spelling mirrors the modal host terminology.
    #[inline]
    pub const fn dismiss_on_outside_press(self, dismiss: bool) -> Self {
        self.dismiss_on_outside_click(dismiss)
    }

    /// Returns whether `Escape` is allowed to dismiss this request.
    #[inline]
    pub const fn dismisses_on_escape(self) -> bool {
        self.dismiss_on_escape
    }

    /// Returns whether an outside press is allowed to dismiss this request.
    #[inline]
    pub const fn dismisses_on_outside_click(self) -> bool {
        self.dismiss_on_outside_click
    }

    /// Returns whether an outside press is allowed to dismiss this request.
    #[inline]
    pub const fn dismisses_on_outside_press(self) -> bool {
        self.dismisses_on_outside_click()
    }
}

/// The caller-owned overlay host consumed by picker controls.
///
/// Implementations may map this seam to Aimer's existing overlay primitives,
/// a browser portal, or a test recorder. The picker crate never stores a
/// global host or chooses a platform presenter.
pub trait OverlayConsumer {
    /// The host's handle for a presented overlay.
    type Handle: Clone;

    /// Presents one picker overlay and returns its host-owned handle.
    fn present(&mut self, request: OverlayRequest) -> Self::Handle;

    /// Dismisses a previously presented overlay.
    fn dismiss(&mut self, handle: Self::Handle);

    /// Reports whether the application has installed an overlay host.
    ///
    /// Existing adapters default to an available host. An adapter around an
    /// application tree without `aimer_modal::ModalHost` should return
    /// `false` so [`PickerOverlay::try_present`] can fail explicitly without
    /// invoking [`Self::present`].
    #[inline]
    fn is_available(&self) -> bool {
        true
    }

    /// Reports whether this host supports the requested picker presentation.
    ///
    /// This is the capability boundary for adapters that can be installed but
    /// cannot provide a requested modal, anchored, or dismissal behavior.
    #[inline]
    fn supports(&self, _request: OverlayRequest) -> bool {
        true
    }

    /// Checks host availability and request support before presenting.
    fn try_present(&mut self, request: OverlayRequest) -> Result<Self::Handle, PickerSessionError> {
        if !self.is_available() {
            return Err(PickerSessionError::MissingHost);
        }
        if !self.supports(request) {
            return Err(PickerSessionError::UnsupportedHost);
        }
        Ok(self.present(request))
    }
}

/// The caller-owned focus restoration boundary for picker dismissal.
pub trait FocusRestorer {
    /// Returns focus to the control identified by `target`.
    fn restore_focus(&mut self, target: FocusTarget);
}

/// A presented picker overlay tied to the focus target that opened it.
#[derive(Debug, Eq, PartialEq)]
pub struct PickerOverlay<H: Clone> {
    handle: H,
    restore_focus: FocusTarget,
    request: OverlayRequest,
    closed: bool,
}

impl<H: Clone> PickerOverlay<H> {
    /// Presents a picker through the caller-owned overlay consumer.
    pub fn present<C: OverlayConsumer<Handle = H>>(
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Self {
        Self {
            handle: consumer.present(request),
            restore_focus,
            request,
            closed: false,
        }
    }

    /// Attempts to present through a host after checking its availability and
    /// support for the request.
    pub fn try_present<C: OverlayConsumer<Handle = H>>(
        consumer: &mut C,
        request: OverlayRequest,
        restore_focus: FocusTarget,
    ) -> Result<Self, PickerSessionError> {
        Ok(Self {
            handle: consumer.try_present(request)?,
            restore_focus,
            request,
            closed: false,
        })
    }

    /// Returns the consumer-owned overlay handle.
    #[inline]
    pub fn handle(&self) -> H {
        self.handle.clone()
    }

    /// Returns the target that should regain focus on dismissal.
    #[inline]
    pub const fn restore_focus(&self) -> FocusTarget {
        self.restore_focus
    }

    /// Returns the request used to present this overlay.
    #[inline]
    pub const fn request(&self) -> OverlayRequest {
        self.request
    }

    /// Returns whether this adapter still owns a presented overlay.
    #[inline]
    pub const fn is_presented(&self) -> bool {
        !self.closed
    }

    /// Answers whether the supplied cancellation reason is enabled by this
    /// overlay's host-facing dismissal policy.
    #[inline]
    pub const fn should_dismiss(&self, reason: CancelReason) -> bool {
        match reason {
            CancelReason::Escape => self.request.dismisses_on_escape(),
            CancelReason::OutsideClick => self.request.dismisses_on_outside_click(),
            CancelReason::Programmatic => true,
        }
    }

    /// Dismisses the overlay when `reason` is enabled and restores focus once.
    ///
    /// Returns `false` when the overlay was already dismissed or when the
    /// request policy rejects the supplied user-driven reason. In either case
    /// the overlay remains available to handle a later allowed dismissal.
    pub fn dismiss<C: OverlayConsumer<Handle = H>, F: FocusRestorer>(
        &mut self,
        reason: CancelReason,
        consumer: &mut C,
        focus: &mut F,
    ) -> bool {
        if self.closed || !self.should_dismiss(reason) {
            return false;
        }
        self.closed = true;
        consumer.dismiss(self.handle.clone());
        focus.restore_focus(self.restore_focus);
        true
    }

    /// Acknowledges a dismissal performed by the native or browser host.
    ///
    /// The host has already removed its own layer, so this method only closes
    /// the caller's adapter state and restores focus. Callers should cancel
    /// the associated picker model with the same `reason` before acknowledging
    /// the host event. No second host dismissal is requested.
    ///
    /// Returns `false` when the adapter is already closed or when the request
    /// policy rejects the supplied user-driven reason.
    pub fn acknowledge_external_dismissal<F: FocusRestorer>(
        &mut self,
        reason: CancelReason,
        focus: &mut F,
    ) -> bool {
        if self.closed || !self.should_dismiss(reason) {
            return false;
        }
        self.closed = true;
        focus.restore_focus(self.restore_focus);
        true
    }

    /// Dismisses the overlay and restores focus exactly once.
    pub fn close<C: OverlayConsumer<Handle = H>, F: FocusRestorer>(
        self,
        consumer: &mut C,
        focus: &mut F,
    ) {
        let mut overlay = self;
        let _ = overlay.dismiss(CancelReason::Programmatic, consumer, focus);
    }
}
