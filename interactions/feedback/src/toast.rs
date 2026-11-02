use std::collections::VecDeque;
use std::time::Duration;

use aimer_widget::base::BuildContext;
use aimer_widget::{AnyElement, PortableWidget, Widget};

use super::clock::Clock;
use super::overlay::{
    Announcement, AnnouncementPriority, Announcer, DismissReason, OverlayHost, OverlayId,
    OverlayKind, OverlayModality, OverlayRequest,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(4);

/// A stable identifier assigned by [`ToastQueue`] when a toast is enqueued.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ToastId(u64);

impl ToastId {
    /// Returns the queue-local identifier.
    #[inline]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// The visual and semantic severity of a toast.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ToastKind {
    /// Neutral information.
    #[default]
    Info,
    /// A successful operation.
    Success,
    /// A condition that needs attention but is recoverable.
    Warning,
    /// A failed or urgent operation.
    Error,
}

/// A caller-owned action offered by a toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastAction {
    label: String,
    key: String,
}

impl ToastAction {
    /// Creates an action with a visible label and stable callback key.
    #[inline]
    pub fn new(label: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            key: key.into(),
        }
    }

    /// Returns the visible action label.
    #[inline]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the callback key supplied by the caller.
    #[inline]
    pub fn key(&self) -> &str {
        &self.key
    }
}

/// A single transient status message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    message: String,
    kind: ToastKind,
    timeout: Option<Duration>,
    action: Option<ToastAction>,
    replacement_key: Option<String>,
    announcement_priority: AnnouncementPriority,
}

impl Toast {
    /// Creates an informational toast with a four-second lifetime.
    #[inline]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Info,
            timeout: Some(DEFAULT_TIMEOUT),
            action: None,
            replacement_key: None,
            announcement_priority: AnnouncementPriority::Polite,
        }
    }

    /// Sets the toast severity.
    #[inline]
    pub const fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    /// Sets the lifetime after presentation.
    #[inline]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// Makes the toast persistent until an action or caller dismissal.
    #[inline]
    pub const fn persistent(mut self) -> Self {
        self.timeout = None;
        self
    }

    /// Adds a caller-owned action to the toast.
    #[inline]
    pub fn action(mut self, action: ToastAction) -> Self {
        self.action = Some(action);
        self
    }

    /// Coalesces messages with the same key while they are queued or active.
    #[inline]
    pub fn replacement_key(mut self, key: impl Into<String>) -> Self {
        self.replacement_key = Some(key.into());
        self
    }

    /// Sets the priority forwarded to the accessibility announcement adapter.
    #[inline]
    pub const fn announcement_priority(mut self, priority: AnnouncementPriority) -> Self {
        self.announcement_priority = priority;
        self
    }

    /// Returns the message text.
    #[inline]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the toast severity.
    #[inline]
    pub const fn kind_value(&self) -> ToastKind {
        self.kind
    }

    /// Returns the configured timeout, or `None` for a persistent toast.
    #[inline]
    pub const fn timeout_value(&self) -> Option<Duration> {
        self.timeout
    }

    /// Returns the optional action.
    #[inline]
    pub fn action_value(&self) -> Option<&ToastAction> {
        self.action.as_ref()
    }

    /// Returns the optional replacement key.
    #[inline]
    pub fn replacement_key_value(&self) -> Option<&str> {
        self.replacement_key.as_deref()
    }

    /// Returns the announcement priority.
    #[inline]
    pub const fn announcement_priority_value(&self) -> AnnouncementPriority {
        self.announcement_priority
    }

    /// Builds the non-modal host request for this toast.
    #[inline]
    pub fn overlay_request(&self) -> OverlayRequest {
        OverlayRequest::new(OverlayKind::Toast, self.message.clone())
            .modality(OverlayModality::NonModal)
            .z_index(100)
            .dismiss_on_escape(true)
            .dismiss_on_outside_press(true)
    }

    /// Creates the accessibility announcement for this toast.
    #[inline]
    pub fn announcement(&self) -> Announcement {
        Announcement::new(self.message.clone()).with_priority(self.announcement_priority)
    }
}

impl Widget for Toast {
    fn to_element(self, ctx: &BuildContext) -> AnyElement {
        // Toasts are normally rendered by an explicitly supplied overlay
        // host. The inline status banner keeps this model useful in a
        // host-less/portable tree without introducing a global singleton.
        let kind = match self.kind {
            ToastKind::Info => super::status::StatusKind::Info,
            ToastKind::Success => super::status::StatusKind::Success,
            ToastKind::Warning => super::status::StatusKind::Warning,
            ToastKind::Error => super::status::StatusKind::Error,
        };
        super::status::StatusBanner::new(self.message)
            .kind(kind)
            .to_element(ctx)
    }

    fn debug_name(&self) -> &'static str {
        "Toast"
    }
}

impl PortableWidget for Toast {}

/// Compatibility name for the canonical [`Toast`] state model.
pub type Snackbar = Toast;

#[derive(Clone, Debug)]
struct QueuedToast {
    id: ToastId,
    toast: Toast,
}

#[derive(Clone, Debug)]
struct ActiveToast {
    id: ToastId,
    toast: Toast,
    replacement: Option<Toast>,
    overlay_id: OverlayId,
    started_at: Duration,
    paused_at: Option<Duration>,
    needs_update: bool,
}

impl ActiveToast {
    fn expired(&self, now: Duration) -> bool {
        let Some(timeout) = self.toast.timeout else {
            return false;
        };
        if self.paused_at.is_some() {
            return false;
        }
        now.saturating_sub(self.started_at) >= timeout
    }

    fn remaining(&self, now: Duration) -> Option<Duration> {
        self.toast.timeout.map(|timeout| {
            let reference = self.paused_at.unwrap_or(now);
            timeout.saturating_sub(reference.saturating_sub(self.started_at))
        })
    }
}

/// The observable transition produced by [`ToastQueue::pump`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastQueueEvent {
    /// No toast was started, updated, or dismissed.
    Idle,
    /// A queued toast was presented.
    Presented(ToastId),
    /// The active toast was updated in place or replaced through the host seam.
    Updated(ToastId),
    /// The active toast was dismissed.
    Dismissed {
        /// The dismissed toast identifier.
        id: ToastId,
        /// The reason supplied to the overlay host.
        reason: DismissReason,
    },
}

/// An explicitly owned, clock-driven queue for transient feedback.
pub struct ToastQueue<C> {
    clock: C,
    next_id: u64,
    pending: VecDeque<QueuedToast>,
    active: Option<ActiveToast>,
}

impl<C: Clock> ToastQueue<C> {
    /// Creates an empty queue driven by the supplied clock.
    #[inline]
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            next_id: 1,
            pending: VecDeque::new(),
            active: None,
        }
    }

    /// Enqueues a toast and returns its stable identifier.
    ///
    /// A toast with a replacement key updates the matching active or pending
    /// item instead of growing the queue.
    pub fn enqueue(&mut self, toast: Toast) -> ToastId {
        if let Some(key) = toast.replacement_key_value() {
            if let Some(active) = self.active.as_mut()
                && active.toast.replacement_key_value() == Some(key)
            {
                let id = active.id;
                active.replacement = Some(toast);
                active.needs_update = true;
                return id;
            }
            if let Some(queued) = self
                .pending
                .iter_mut()
                .find(|queued| queued.toast.replacement_key_value() == Some(key))
            {
                let id = queued.id;
                queued.toast = toast;
                return id;
            }
        }

        let id = ToastId(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.pending.push_back(QueuedToast { id, toast });
        id
    }

    /// Returns the active toast, if one has been presented.
    #[inline]
    pub fn active(&self) -> Option<&Toast> {
        self.active.as_ref().map(|active| &active.toast)
    }

    /// Returns the active toast identifier and overlay handle.
    #[inline]
    pub fn active_handles(&self) -> Option<(ToastId, OverlayId)> {
        self.active
            .as_ref()
            .map(|active| (active.id, active.overlay_id))
    }

    /// Returns the number of pending toasts, excluding the active toast.
    #[inline]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Returns whether the active toast lifetime is paused.
    #[inline]
    pub fn is_paused(&self) -> bool {
        self.active
            .as_ref()
            .is_some_and(|active| active.paused_at.is_some())
    }

    /// Returns remaining active time, or `None` for no active/persistent toasts.
    #[inline]
    pub fn remaining(&self) -> Option<Duration> {
        self.active
            .as_ref()
            .and_then(|active| active.remaining(self.clock.now()))
    }

    /// Presents the next toast, updates a replacement, or expires the active toast.
    pub fn pump<H: OverlayHost>(
        &mut self,
        host: &mut H,
        announcer: Option<&mut dyn Announcer>,
    ) -> ToastQueueEvent {
        let mut announcer = announcer;
        let now = self.clock.now();

        if self
            .active
            .as_ref()
            .is_some_and(|active| active.needs_update)
        {
            let (id, overlay_id, replacement) = {
                let active = self.active.as_ref().expect("active update checked above");
                let Some(replacement) = active.replacement.as_ref() else {
                    return ToastQueueEvent::Idle;
                };
                (active.id, active.overlay_id, replacement.clone())
            };
            let request = replacement.overlay_request();
            let new_overlay_id = if host.update(overlay_id, request.clone()) {
                overlay_id
            } else {
                if !host.dismiss(overlay_id, DismissReason::Replaced) {
                    return ToastQueueEvent::Idle;
                }
                host.present(request)
            };
            let active = self.active.as_mut().expect("active update checked above");
            active.overlay_id = new_overlay_id;
            active.toast = active
                .replacement
                .take()
                .expect("replacement was retained for the update");
            active.started_at = now;
            active.paused_at = None;
            active.needs_update = false;
            if let Some(announcer) = announcer.as_deref_mut() {
                announcer.announce(active.toast.announcement());
            }
            return ToastQueueEvent::Updated(id);
        }

        if self
            .active
            .as_ref()
            .is_some_and(|active| active.expired(now))
        {
            let Some(active) = self.active.as_ref() else {
                return ToastQueueEvent::Idle;
            };
            if !host.dismiss(active.overlay_id, DismissReason::Timeout) {
                return ToastQueueEvent::Idle;
            }
            let active = self.active.take().expect("active toast checked above");
            return ToastQueueEvent::Dismissed {
                id: active.id,
                reason: DismissReason::Timeout,
            };
        }

        if self.active.is_none()
            && let Some(queued) = self.pending.pop_front()
        {
            let request = queued.toast.overlay_request();
            let overlay_id = host.present(request);
            if let Some(announcer) = announcer.as_deref_mut() {
                announcer.announce(queued.toast.announcement());
            }
            let id = queued.id;
            self.active = Some(ActiveToast {
                id,
                toast: queued.toast,
                replacement: None,
                overlay_id,
                started_at: now,
                paused_at: None,
                needs_update: false,
            });
            return ToastQueueEvent::Presented(id);
        }

        ToastQueueEvent::Idle
    }

    /// Dismisses the active toast through the overlay host.
    pub fn dismiss_active<H: OverlayHost>(&mut self, host: &mut H, reason: DismissReason) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        if !host.dismiss(active.overlay_id, reason) {
            return false;
        }
        self.active.take();
        true
    }

    /// Pauses timeout accounting for the active toast.
    #[inline]
    pub fn pause(&mut self) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if active.paused_at.is_none() {
            active.paused_at = Some(self.clock.now());
            true
        } else {
            false
        }
    }

    /// Resumes timeout accounting, excluding the paused duration.
    #[inline]
    pub fn resume(&mut self) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let Some(paused_at) = active.paused_at.take() else {
            return false;
        };
        let paused_for = self.clock.now().saturating_sub(paused_at);
        active.started_at = active.started_at.saturating_add(paused_for);
        true
    }

    /// Activates the current toast action when its key matches.
    pub fn activate_action<H: OverlayHost>(
        &mut self,
        host: &mut H,
        key: &str,
    ) -> Option<ToastAction> {
        let action = self
            .active
            .as_ref()
            .and_then(|active| active.toast.action_value())
            .filter(|action| action.key() == key)
            .cloned();
        if action.is_some() && self.dismiss_active(host, DismissReason::Action) {
            action
        } else {
            None
        }
    }
}
