use std::fmt;

/// The kind of content an overlay host is being asked to present.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverlayKind {
    /// A short, anchored description of a control.
    Tooltip,
    /// A transient status message in the application feedback region.
    Toast,
    /// A caller-owned overlay with no more specific feedback kind.
    Custom,
}

/// Whether an overlay blocks interaction with content below it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverlayModality {
    /// The overlay does not take focus or block pointer input below it.
    #[default]
    NonModal,
    /// The host should trap interaction until the overlay is dismissed.
    Modal,
}

/// The reason an overlay was dismissed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DismissReason {
    /// The caller explicitly dismissed the overlay.
    Programmatic,
    /// The overlay's configured lifetime elapsed.
    Timeout,
    /// An action in the overlay was activated.
    Action,
    /// The trigger that anchored the overlay is no longer active.
    TriggerExit,
    /// A touch interaction ended before the overlay should remain visible.
    TouchEnd,
    /// A newer request replaced the existing overlay in place.
    Replaced,
    /// The user pressed Escape while the overlay was active.
    Escape,
    /// The user pressed or tapped outside the overlay.
    OutsidePress,
}

/// An opaque focus target used by an overlay host when restoring focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FocusTarget(u64);

impl FocusTarget {
    /// Creates a focus target from an application-owned stable identifier.
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the application-owned identifier.
    #[inline]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// An opaque overlay handle returned by [`OverlayHost::present`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OverlayId(u64);

impl OverlayId {
    /// Creates an overlay identifier for a host-owned stable value.
    #[inline]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the host-owned identifier.
    #[inline]
    pub const fn value(self) -> u64 {
        self.0
    }
}

impl fmt::Display for OverlayId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The priority an accessibility adapter should use for an announcement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AnnouncementPriority {
    /// Announce when the platform is otherwise idle.
    #[default]
    Polite,
    /// Interrupt lower-priority announcements because the status is urgent.
    Assertive,
}

/// Text sent to an accessibility announcement adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    text: String,
    priority: AnnouncementPriority,
}

impl Announcement {
    /// Creates an announcement with [`AnnouncementPriority::Polite`] priority.
    #[inline]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            priority: AnnouncementPriority::Polite,
        }
    }

    /// Sets the announcement priority.
    #[inline]
    pub fn with_priority(mut self, priority: AnnouncementPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Returns the announcement text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the announcement priority.
    #[inline]
    pub const fn priority_value(&self) -> AnnouncementPriority {
        self.priority
    }
}

/// The W1-facing seam used by feedback widgets to announce visible status.
pub trait Announcer {
    /// Delivers an announcement to the platform or application accessibility adapter.
    fn announce(&mut self, announcement: Announcement);
}

/// An axis-aligned rectangle in the host's logical coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    /// Creates a rectangle, normalizing non-finite and negative dimensions.
    #[inline]
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x: finite_or_zero(x),
            y: finite_or_zero(y),
            width: finite_or_zero(width).max(0.0),
            height: finite_or_zero(height).max(0.0),
        }
    }

    /// Returns the left coordinate.
    #[inline]
    pub const fn x(self) -> f32 {
        self.x
    }

    /// Returns the top coordinate.
    #[inline]
    pub const fn y(self) -> f32 {
        self.y
    }

    /// Returns the width.
    #[inline]
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Returns the height.
    #[inline]
    pub const fn height(self) -> f32 {
        self.height
    }

    /// Returns the right coordinate.
    #[inline]
    pub const fn right(self) -> f32 {
        self.x + self.width
    }

    /// Returns the bottom coordinate.
    #[inline]
    pub const fn bottom(self) -> f32 {
        self.y + self.height
    }
}

/// A non-negative overlay size in the host's logical coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    width: f32,
    height: f32,
}

impl Size {
    /// Creates a size, normalizing non-finite and negative dimensions.
    #[inline]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: finite_or_zero(width).max(0.0),
            height: finite_or_zero(height).max(0.0),
        }
    }

    /// Returns the width.
    #[inline]
    pub const fn width(self) -> f32 {
        self.width
    }

    /// Returns the height.
    #[inline]
    pub const fn height(self) -> f32 {
        self.height
    }
}

/// The side of an anchor on which an overlay is requested.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OverlaySide {
    /// Above the anchor.
    Top,
    /// Below the anchor.
    Bottom,
    /// To the left of the anchor.
    Left,
    /// To the right of the anchor.
    Right,
}

impl OverlaySide {
    /// Returns the opposite side.
    #[inline]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Top => Self::Bottom,
            Self::Bottom => Self::Top,
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// Alignment of an overlay along the cross-axis of its anchor.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PlacementAlign {
    /// Align leading edges.
    #[default]
    Start,
    /// Center the overlay on the anchor.
    Center,
    /// Align trailing edges.
    End,
}

/// What an overlay host should do when the requested placement does not fit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum OverflowPolicy {
    /// Try the opposite side and then shift into the viewport.
    #[default]
    Flip,
    /// Keep the side and shift into the viewport.
    Shift,
    /// Preserve the requested position even when it is clipped.
    Fixed,
}

/// Placement preferences resolved against an anchor and viewport by the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementSpec {
    side: OverlaySide,
    align: PlacementAlign,
    gap: f32,
    overflow: OverflowPolicy,
}

impl Default for PlacementSpec {
    fn default() -> Self {
        Self::new()
    }
}

impl PlacementSpec {
    /// Creates a bottom/start placement with an eight-point gap and flip policy.
    #[inline]
    pub const fn new() -> Self {
        Self {
            side: OverlaySide::Bottom,
            align: PlacementAlign::Start,
            gap: 8.0,
            overflow: OverflowPolicy::Flip,
        }
    }

    /// Sets the preferred side.
    #[inline]
    pub const fn side(mut self, side: OverlaySide) -> Self {
        self.side = side;
        self
    }

    /// Sets cross-axis alignment.
    #[inline]
    pub const fn align(mut self, align: PlacementAlign) -> Self {
        self.align = align;
        self
    }

    /// Sets the main-axis gap; invalid or negative values become zero.
    #[inline]
    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = finite_or_zero(gap).max(0.0);
        self
    }

    /// Sets the overflow policy.
    #[inline]
    pub const fn overflow(mut self, overflow: OverflowPolicy) -> Self {
        self.overflow = overflow;
        self
    }

    /// Returns the preferred side.
    #[inline]
    pub const fn side_value(self) -> OverlaySide {
        self.side
    }

    /// Returns the cross-axis alignment.
    #[inline]
    pub const fn align_value(self) -> PlacementAlign {
        self.align
    }

    /// Returns the main-axis gap.
    #[inline]
    pub const fn gap_value(self) -> f32 {
        self.gap
    }

    /// Returns the overflow policy.
    #[inline]
    pub const fn overflow_value(self) -> OverflowPolicy {
        self.overflow
    }
}

/// The resolved position of an overlay after collision handling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResolvedPlacement {
    rect: Rect,
    side: OverlaySide,
    clipped: bool,
}

impl ResolvedPlacement {
    /// Returns the resolved rectangle.
    #[inline]
    pub const fn rect(self) -> Rect {
        self.rect
    }

    /// Returns the side selected after optional flipping.
    #[inline]
    pub const fn side(self) -> OverlaySide {
        self.side
    }

    /// Returns whether some part of the overlay lies outside the viewport.
    #[inline]
    pub const fn clipped(self) -> bool {
        self.clipped
    }
}

/// Resolves an anchored overlay placement and applies the requested collision policy.
pub fn resolve_placement(
    anchor: Rect,
    overlay: Size,
    viewport: Rect,
    spec: PlacementSpec,
) -> ResolvedPlacement {
    let preferred = candidate(anchor, overlay, spec.side, spec.align, spec.gap);
    let (rect, side) = match spec.overflow {
        OverflowPolicy::Fixed => (preferred, spec.side),
        OverflowPolicy::Shift => (shift_into(preferred, viewport), spec.side),
        OverflowPolicy::Flip => {
            let opposite_side = spec.side.flipped();
            let opposite = candidate(anchor, overlay, opposite_side, spec.align, spec.gap);
            if fits(preferred, viewport) || !fits(opposite, viewport) {
                (shift_into(preferred, viewport), spec.side)
            } else {
                (shift_into(opposite, viewport), opposite_side)
            }
        }
    };
    let clipped = !fits(rect, viewport);
    ResolvedPlacement {
        rect,
        side,
        clipped,
    }
}

/// A request passed to the application's overlay host.
#[derive(Clone, Debug, PartialEq)]
pub struct OverlayRequest {
    kind: OverlayKind,
    text: String,
    anchor: Option<Rect>,
    placement: PlacementSpec,
    modality: OverlayModality,
    z_index: i32,
    dismiss_on_escape: bool,
    dismiss_on_outside_press: bool,
    restore_focus: Option<FocusTarget>,
}

impl OverlayRequest {
    /// Creates a request with non-modal presentation and default placement.
    #[inline]
    pub fn new(kind: OverlayKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            anchor: None,
            placement: PlacementSpec::default(),
            modality: OverlayModality::NonModal,
            z_index: 0,
            dismiss_on_escape: false,
            dismiss_on_outside_press: false,
            restore_focus: None,
        }
    }

    /// Sets the anchor rectangle used by anchored overlay hosts.
    #[inline]
    pub const fn anchor(mut self, anchor: Rect) -> Self {
        self.anchor = Some(anchor);
        self
    }

    /// Sets placement preferences for this request.
    #[inline]
    pub const fn placement(mut self, placement: PlacementSpec) -> Self {
        self.placement = placement;
        self
    }

    /// Sets whether the host should make the overlay modal.
    #[inline]
    pub const fn modality(mut self, modality: OverlayModality) -> Self {
        self.modality = modality;
        self
    }

    /// Sets the z-order requested from the host.
    #[inline]
    pub const fn z_index(mut self, z_index: i32) -> Self {
        self.z_index = z_index;
        self
    }

    /// Sets whether Escape should dismiss the overlay.
    #[inline]
    pub const fn dismiss_on_escape(mut self, dismiss: bool) -> Self {
        self.dismiss_on_escape = dismiss;
        self
    }

    /// Sets whether an outside press should dismiss the overlay.
    #[inline]
    pub const fn dismiss_on_outside_press(mut self, dismiss: bool) -> Self {
        self.dismiss_on_outside_press = dismiss;
        self
    }

    /// Requests focus restoration after the host dismisses this overlay.
    #[inline]
    pub const fn restore_focus(mut self, target: FocusTarget) -> Self {
        self.restore_focus = Some(target);
        self
    }

    /// Returns the requested overlay kind.
    #[inline]
    pub const fn kind(&self) -> OverlayKind {
        self.kind
    }

    /// Returns the display text.
    #[inline]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Returns the optional anchor.
    #[inline]
    pub const fn anchor_value(&self) -> Option<Rect> {
        self.anchor
    }

    /// Returns placement preferences.
    #[inline]
    pub const fn placement_value(&self) -> PlacementSpec {
        self.placement
    }

    /// Returns the requested modality.
    #[inline]
    pub const fn modality_value(&self) -> OverlayModality {
        self.modality
    }

    /// Marks this request as modal and enables the host's standard dismissal
    /// policy for Escape and outside presses.
    #[inline]
    pub const fn modal(self) -> Self {
        self.modality(OverlayModality::Modal)
            .dismiss_on_escape(true)
            .dismiss_on_outside_press(true)
    }

    /// Marks this request as non-modal while retaining its other policy.
    #[inline]
    pub const fn non_modal(self) -> Self {
        self.modality(OverlayModality::NonModal)
    }

    /// Returns the requested z-order.
    #[inline]
    pub const fn z_index_value(&self) -> i32 {
        self.z_index
    }

    /// Returns whether Escape dismissal is requested.
    #[inline]
    pub const fn dismiss_on_escape_value(&self) -> bool {
        self.dismiss_on_escape
    }

    /// Returns whether outside-press dismissal is requested.
    #[inline]
    pub const fn dismiss_on_outside_press_value(&self) -> bool {
        self.dismiss_on_outside_press
    }

    /// Returns the optional focus target to restore after dismissal.
    #[inline]
    pub const fn restore_focus_value(&self) -> Option<FocusTarget> {
        self.restore_focus
    }
}

/// A small adapter implemented by the existing application overlay host.
pub trait OverlayHost {
    /// Presents an overlay and returns its host-owned handle.
    fn present(&mut self, request: OverlayRequest) -> OverlayId;

    /// Updates an existing overlay in place when the host supports it.
    ///
    /// Returning `false` asks the feedback model to replace the overlay by
    /// dismissing and presenting it again.
    #[inline]
    fn update(&mut self, _id: OverlayId, _request: OverlayRequest) -> bool {
        false
    }

    /// Dismisses an overlay and reports whether the host accepted the request.
    fn dismiss(&mut self, id: OverlayId, reason: DismissReason) -> bool;

    /// Restores focus to a target captured before a modal overlay was shown.
    #[inline]
    fn restore_focus(&mut self, _target: FocusTarget) {}
}

/// The explicitly owned lifecycle of one feedback overlay.
//
// The host remains responsible for the actual layer and z-order. This value
// only retains the request and host handle, making ownership visible to the
// caller and keeping feedback free of process-global overlay state.
#[derive(Clone, Debug, Default)]
pub struct OverlayLifecycle {
    active: Option<HostedOverlay>,
}

#[derive(Clone, Debug)]
struct HostedOverlay {
    id: OverlayId,
    request: OverlayRequest,
}

impl OverlayLifecycle {
    /// Creates an empty lifecycle with no presented overlay.
    #[inline]
    pub const fn new() -> Self {
        Self { active: None }
    }

    /// Presents a request and records its host-owned handle.
    ///
    /// An existing overlay is replaced first. Focus is intentionally restored
    /// only when the final active overlay is dismissed, so replacing one
    /// modal with another does not briefly move focus through the old trigger.
    pub fn present<H: OverlayHost>(&mut self, host: &mut H, request: OverlayRequest) -> OverlayId {
        if let Some(previous) = self.active.take() {
            if !host.dismiss(previous.id, DismissReason::Replaced) {
                // Do not create an orphaned second overlay when the host
                // rejects replacement. The caller can retry after the host
                // accepts dismissal; the existing request remains active.
                let id = previous.id;
                self.active = Some(previous);
                return id;
            }
        }
        let id = host.present(request.clone());
        self.active = Some(HostedOverlay { id, request });
        id
    }

    /// Returns the active host-owned overlay handle, if any.
    #[inline]
    pub fn active_id(&self) -> Option<OverlayId> {
        self.active.as_ref().map(|active| active.id)
    }

    /// Returns the active request, if any.
    #[inline]
    pub fn active_request(&self) -> Option<&OverlayRequest> {
        self.active.as_ref().map(|active| &active.request)
    }

    /// Returns whether a request is currently retained by this lifecycle.
    #[inline]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Updates the active request in place when the host supports it.
    ///
    /// A `false` result means the host did not accept the update; the current
    /// request remains retained so the caller can choose a replacement path.
    pub fn update<H: OverlayHost>(&mut self, host: &mut H, request: OverlayRequest) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        if !host.update(active.id, request.clone()) {
            return false;
        }
        active.request = request;
        true
    }

    /// Dismisses the active overlay and restores its requested focus target.
    ///
    /// When the host rejects dismissal, the lifecycle remains active and no
    /// focus restoration is attempted.
    pub fn dismiss<H: OverlayHost>(&mut self, host: &mut H, reason: DismissReason) -> bool {
        let Some(active) = self.active.as_ref() else {
            return false;
        };
        if !host.dismiss(active.id, reason) {
            return false;
        }
        let active = self.active.take().expect("active overlay checked above");
        restore_focus(host, &active.request);
        true
    }

    /// Clears local state after a host dismissed an overlay externally.
    ///
    /// This does not call the host and is useful when the host reports an
    /// Escape or outside-press dismissal through its own event path.
    pub fn acknowledge_dismissal(&mut self) -> bool {
        self.active.take().is_some()
    }
}

fn restore_focus<H: OverlayHost>(host: &mut H, request: &OverlayRequest) {
    if let Some(target) = request.restore_focus_value() {
        host.restore_focus(target);
    }
}

fn finite_or_zero(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn candidate(
    anchor: Rect,
    overlay: Size,
    side: OverlaySide,
    align: PlacementAlign,
    gap: f32,
) -> Rect {
    let cross_x = match align {
        PlacementAlign::Start => anchor.x(),
        PlacementAlign::Center => anchor.x() + (anchor.width() - overlay.width()) * 0.5,
        PlacementAlign::End => anchor.right() - overlay.width(),
    };
    let cross_y = match align {
        PlacementAlign::Start => anchor.y(),
        PlacementAlign::Center => anchor.y() + (anchor.height() - overlay.height()) * 0.5,
        PlacementAlign::End => anchor.bottom() - overlay.height(),
    };
    match side {
        OverlaySide::Top => Rect::new(
            cross_x,
            anchor.y() - gap - overlay.height(),
            overlay.width(),
            overlay.height(),
        ),
        OverlaySide::Bottom => Rect::new(
            cross_x,
            anchor.bottom() + gap,
            overlay.width(),
            overlay.height(),
        ),
        OverlaySide::Left => Rect::new(
            anchor.x() - gap - overlay.width(),
            cross_y,
            overlay.width(),
            overlay.height(),
        ),
        OverlaySide::Right => Rect::new(
            anchor.right() + gap,
            cross_y,
            overlay.width(),
            overlay.height(),
        ),
    }
}

fn fits(rect: Rect, viewport: Rect) -> bool {
    rect.x() >= viewport.x()
        && rect.y() >= viewport.y()
        && rect.right() <= viewport.right()
        && rect.bottom() <= viewport.bottom()
}

fn shift_into(rect: Rect, viewport: Rect) -> Rect {
    let x = if rect.width() >= viewport.width() {
        viewport.x()
    } else {
        rect.x()
            .max(viewport.x())
            .min(viewport.right() - rect.width())
    };
    let y = if rect.height() >= viewport.height() {
        viewport.y()
    } else {
        rect.y()
            .max(viewport.y())
            .min(viewport.bottom() - rect.height())
    };
    Rect::new(x, y, rect.width(), rect.height())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flip_placement_uses_the_opposite_side_when_only_it_fits() {
        let result = resolve_placement(
            Rect::new(40.0, 90.0, 20.0, 10.0),
            Size::new(30.0, 20.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            PlacementSpec::new().side(OverlaySide::Bottom).gap(4.0),
        );

        assert_eq!(result.side(), OverlaySide::Top);
        assert_eq!(result.rect().y(), 66.0);
        assert!(!result.clipped());
    }

    #[test]
    fn shift_placement_keeps_a_small_overlay_inside_the_viewport() {
        let result = resolve_placement(
            Rect::new(0.0, 20.0, 10.0, 10.0),
            Size::new(50.0, 20.0),
            Rect::new(0.0, 0.0, 100.0, 100.0),
            PlacementSpec::new()
                .side(OverlaySide::Top)
                .align(PlacementAlign::End)
                .overflow(OverflowPolicy::Shift),
        );

        assert_eq!(result.rect().x(), 0.0);
        assert_eq!(result.rect().y(), 0.0);
        assert!(!result.clipped());
    }
}
