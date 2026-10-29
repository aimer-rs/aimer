//! Hands the application's semantic tree to the platform's assistive
//! technology, and takes its action requests back.
//!
//! The tree itself is platform-neutral ([`aimer_accessibility`]); this module is
//! the seam the frame loop talks to. A window is *attached* once, which on
//! AppKit adds a hit-test-transparent overlay view that answers accessibility
//! queries. Nothing is collected until an assistive technology actually asks:
//! [`is_requested`] stays `false` for an application nobody is inspecting, so
//! the frame loop pays nothing for accessibility it is not providing.
//!
//! Actions flow the other way through an [`ActionQueue`]. The platform's
//! callbacks run wherever the system chooses to call them and must not reach
//! into the widget tree, so they post here and wake the event loop, which
//! performs the actions on the UI thread.

use std::collections::VecDeque;
use std::sync::Mutex;

use aimer_accessibility::{ActionRequest, SemanticSnapshot};
use winit::window::Window;

/// A bounded, thread-safe queue of action requests from assistive technology.
#[derive(Default)]
pub struct ActionQueue {
    requests: Mutex<VecDeque<(usize, ActionRequest)>>,
}

impl ActionQueue {
    /// More requests than a person can issue before the loop wakes; the oldest
    /// are dropped.
    pub const CAPACITY: usize = 32;

    /// Creates an empty queue.
    pub const fn new() -> Self {
        Self { requests: Mutex::new(VecDeque::new()) }
    }

    /// Queues `request` for the window identified by `window`, returning whether
    /// the event loop needs waking: only the first request after a drain does,
    /// because a wake-up already on its way drains the rest.
    pub fn post(&self, window: usize, request: ActionRequest) -> bool {
        // A poisoned lock only means another thread panicked holding it; the
        // queue itself is still a valid queue.
        let mut requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let was_empty = requests.is_empty();
        if requests.len() == Self::CAPACITY {
            requests.pop_front();
        }
        requests.push_back((window, request));
        was_empty
    }

    /// Takes the requests posted for `window`, oldest first, leaving other
    /// windows' requests queued.
    pub fn drain(&self, window: usize) -> Vec<ActionRequest> {
        let mut requests = self.requests.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut taken = Vec::new();
        requests.retain(|(owner, request)| {
            if *owner == window {
                taken.push(request.clone());
                false
            } else {
                true
            }
        });
        taken
    }
}

/// Requests posted by the platform, shared by every window.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub(crate) static ACTIONS: ActionQueue = ActionQueue::new();

/// Wakes the event loop so it publishes the tree or performs queued actions.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios")), allow(dead_code))]
pub(crate) fn wake_event_loop() {
    if let Some(proxy) = crate::event::EVENT_PROXY.get() {
        let _ = proxy.send_event(crate::event::AimerNativePlatformEvent::Accessibility);
    }
}

/// Makes `window` able to answer accessibility queries. Idempotent.
///
/// Must be called on the main thread. Platforms without a bridge ignore it.
pub fn attach(window: &Window) {
    #[cfg(target_os = "macos")]
    crate::ffi_utils::macos_accessibility::attach(window);
    #[cfg(target_os = "ios")]
    crate::ffi_utils::ios_accessibility::attach(window);
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = window;
}

/// Whether an assistive technology has queried `window`'s tree.
///
/// Sticky: once a screen reader has looked, the tree is kept current.
pub fn is_requested(window: &Window) -> bool {
    #[cfg(target_os = "macos")]
    return crate::ffi_utils::macos_accessibility::is_requested(window);
    #[cfg(target_os = "ios")]
    return crate::ffi_utils::ios_accessibility::is_requested(window);
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        let _ = window;
        false
    }
}

/// Replaces the tree `window` answers queries with.
///
/// Native elements keep their identity across calls for nodes whose
/// [`NodeId`](aimer_accessibility::NodeId) is unchanged, so a screen reader's
/// cursor survives an update.
pub fn publish(window: &Window, snapshot: &SemanticSnapshot) {
    #[cfg(target_os = "macos")]
    crate::ffi_utils::macos_accessibility::publish(window, snapshot);
    #[cfg(target_os = "ios")]
    crate::ffi_utils::ios_accessibility::publish(window, snapshot);
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let _ = (window, snapshot);
}

/// Takes the action requests assistive technology made on `window`.
pub fn take_actions(window: &Window) -> Vec<ActionRequest> {
    #[cfg(target_os = "macos")]
    return crate::ffi_utils::macos_accessibility::take_actions(window);
    #[cfg(target_os = "ios")]
    return crate::ffi_utils::ios_accessibility::take_actions(window);
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    {
        let _ = window;
        Vec::new()
    }
}

/// The accessibility role and subrole AppKit knows a [`Role`](aimer_accessibility::Role) by.
///
/// The strings are the values of the `NSAccessibility*Role` constants. They are
/// kept as data, apart from the FFI that applies them, because choosing them is
/// the part that can be stated exactly and tested on any host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppleRole {
    /// The `NSAccessibilityRole`.
    pub role: &'static str,
    /// The `NSAccessibilitySubrole`, when the role alone is ambiguous.
    pub subrole: Option<&'static str>,
}

/// Maps a platform-neutral role to the AppKit one.
pub fn apple_role(role: aimer_accessibility::Role) -> AppleRole {
    use aimer_accessibility::Role;

    let (role, subrole) = match role {
        Role::Button => ("AXButton", None),
        Role::Checkbox => ("AXCheckBox", None),
        Role::Combobox => ("AXComboBox", None),
        Role::Dialog => ("AXGroup", Some("AXDialog")),
        Role::Heading => ("AXHeading", None),
        Role::Image => ("AXImage", None),
        Role::Link => ("AXLink", None),
        Role::List => ("AXList", None),
        Role::ListItem => ("AXGroup", None),
        Role::Menu => ("AXMenu", None),
        Role::MenuItem => ("AXMenuItem", None),
        Role::ProgressBar => ("AXProgressIndicator", None),
        Role::Radio => ("AXRadioButton", None),
        Role::ScrollView => ("AXScrollArea", None),
        Role::Slider => ("AXSlider", None),
        Role::SpinButton => ("AXIncrementor", None),
        Role::Switch => ("AXCheckBox", Some("AXSwitch")),
        Role::Tab => ("AXRadioButton", Some("AXTabButton")),
        Role::TabList => ("AXTabGroup", None),
        Role::TextField => ("AXTextField", None),
        Role::Text => ("AXStaticText", None),
        Role::Tree => ("AXOutline", None),
        Role::TreeItem => ("AXRow", None),
        Role::Generic | Role::Group | Role::TabPanel | Role::Presentation => ("AXGroup", None),
    };
    AppleRole { role, subrole }
}

/// A `UIAccessibilityTrait` a role can carry.
///
/// Named here and resolved to UIKit's runtime constants by the FFI, because the
/// bit values are UIKit's to define while the choice of traits per role is ours
/// to state and test.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IosTrait {
    /// `UIAccessibilityTraitButton`.
    Button,
    /// `UIAccessibilityTraitLink`.
    Link,
    /// `UIAccessibilityTraitImage`.
    Image,
    /// `UIAccessibilityTraitHeader`.
    Header,
    /// `UIAccessibilityTraitStaticText`.
    StaticText,
    /// `UIAccessibilityTraitAdjustable`.
    Adjustable,
    /// `UIAccessibilityTraitSearchField`.
    SearchField,
}

/// The traits UIKit announces a role by. Roles UIKit has no trait for are
/// announced by their label alone.
pub fn ios_traits(role: aimer_accessibility::Role) -> &'static [IosTrait] {
    use aimer_accessibility::Role;

    match role {
        Role::Button | Role::Checkbox | Role::Radio | Role::Switch | Role::Tab | Role::MenuItem => {
            &[IosTrait::Button]
        }
        Role::Link => &[IosTrait::Link],
        Role::Image => &[IosTrait::Image],
        Role::Heading => &[IosTrait::Header],
        Role::Text => &[IosTrait::StaticText],
        Role::Slider | Role::SpinButton => &[IosTrait::Adjustable],
        Role::Combobox => &[IosTrait::Button],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use aimer_accessibility::{NodeId, Role, SemanticAction};

    use super::*;

    fn request(node: u64) -> ActionRequest {
        ActionRequest::new(NodeId::new(node), SemanticAction::Activate)
    }

    #[test]
    fn only_the_first_request_after_a_drain_needs_a_wake() {
        let queue = ActionQueue::new();

        assert!(queue.post(1, request(1)));
        assert!(!queue.post(1, request(2)));
        queue.drain(1);
        assert!(queue.post(1, request(3)));
    }

    #[test]
    fn requests_come_out_in_order_and_only_for_their_window() {
        let queue = ActionQueue::new();
        queue.post(1, request(10));
        queue.post(2, request(20));
        queue.post(1, request(11));

        let first: Vec<_> = queue.drain(1).iter().map(|r| r.node_id().get()).collect();
        let second: Vec<_> = queue.drain(2).iter().map(|r| r.node_id().get()).collect();

        assert_eq!(first, [10, 11]);
        assert_eq!(second, [20]);
        assert!(queue.drain(1).is_empty());
    }

    #[test]
    fn a_full_queue_drops_the_oldest_request() {
        let queue = ActionQueue::new();
        for node in 0..(ActionQueue::CAPACITY as u64 + 3) {
            queue.post(1, request(node));
        }

        let kept = queue.drain(1);

        assert_eq!(kept.len(), ActionQueue::CAPACITY);
        assert_eq!(kept[0].node_id().get(), 3);
    }

    #[test]
    fn roles_map_to_the_traits_uikit_announces() {
        use aimer_accessibility::Role;

        assert_eq!(ios_traits(Role::Button), [IosTrait::Button]);
        assert_eq!(ios_traits(Role::Switch), [IosTrait::Button]);
        assert_eq!(ios_traits(Role::Heading), [IosTrait::Header]);
        assert_eq!(ios_traits(Role::Slider), [IosTrait::Adjustable]);
        assert!(ios_traits(Role::Group).is_empty());
    }

    #[test]
    fn roles_map_to_their_appkit_names() {
        assert_eq!(apple_role(Role::Button), AppleRole { role: "AXButton", subrole: None });
        assert_eq!(apple_role(Role::Text).role, "AXStaticText");
        assert_eq!(
            apple_role(Role::Switch),
            AppleRole { role: "AXCheckBox", subrole: Some("AXSwitch") }
        );
        assert_eq!(apple_role(Role::Generic).role, "AXGroup");
    }
}
