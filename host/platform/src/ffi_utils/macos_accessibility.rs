//! The AppKit half of [`crate::accessibility`].
//!
//! winit owns the window's content view and is not asked to cooperate. Instead
//! a transparent overlay view is added on top of it: `hitTest:` answers `nil`
//! so every pointer event still reaches winit's view, and the overlay's
//! `accessibilityChildren` are the application's [`NSAccessibilityElement`]
//! objects. Elements are kept by [`NodeId`] and updated in place, so the system
//! (and a screen reader's cursor) sees the same object for the same node across
//! publishes.
//!
//! Everything here runs on the main thread, which is where AppKit calls into
//! the overlay and where the event loop runs.
//!
//! [`NodeId`]: aimer_accessibility::NodeId

// The `objc` 0.2 macros test for a `cargo-clippy` cfg this workspace never sets.
#![allow(unexpected_cfgs)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use aimer_accessibility::{
    ActionRequest, CheckedState, NodeId, SemanticAction, SemanticNode, SemanticSnapshot,
};
use objc::declare::ClassDecl;
use objc::rc::StrongPtr;
use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::accessibility::{ACTIONS, apple_role, wake_event_loop};
use crate::ffi_utils::apple_accessibility::{Point, Rect, Size, ns_string};

const OVERLAY_CLASS: &str = "AimerAccessibilityOverlay";
const ELEMENT_CLASS: &str = "AimerAccessibilityElement";

/// `NSViewWidthSizable | NSViewHeightSizable`: keeps the overlay covering the
/// content view as the window is resized.
const AUTORESIZE_FILL: usize = 2 | 16;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {
    fn NSAccessibilityPostNotification(element: *mut Object, notification: *mut Object);
}

/// What the overlay of one window holds.
struct Overlay {
    /// The overlay view, owned here and by its superview.
    view: StrongPtr,
    /// Native elements by node, kept across publishes.
    elements: HashMap<u64, StrongPtr>,
    /// The outermost elements, in document order.
    top: Vec<*mut Object>,
    /// Node order of the last publish, to tell a structural change from a
    /// property one.
    shape: Vec<u64>,
    /// Set the first time the system asks for the children.
    requested: bool,
}

thread_local! {
    /// Overlays by the address of the winit content view they sit on.
    static OVERLAYS: RefCell<HashMap<usize, Overlay>> = RefCell::new(HashMap::new());
}

/// The address of `window`'s content view, which names its overlay.
fn content_view(window: &Window) -> Option<*mut Object> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::AppKit(appkit) => Some(appkit.ns_view.as_ptr().cast()),
        _ => None,
    }
}

/// Registers the overlay view class on first use.
fn overlay_class() -> Option<&'static Class> {
    if let Some(existing) = Class::get(OVERLAY_CLASS) {
        return Some(existing);
    }
    let mut decl = ClassDecl::new(OVERLAY_CLASS, Class::get("NSView")?)?;
    unsafe {
        decl.add_method(
            sel!(isAccessibilityElement),
            overlay_is_element as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityRole),
            overlay_role as extern "C" fn(&Object, Sel) -> *mut Object,
        );
        decl.add_method(
            sel!(accessibilityChildren),
            overlay_children as extern "C" fn(&Object, Sel) -> *mut Object,
        );
        decl.add_method(
            sel!(hitTest:),
            overlay_hit_test as extern "C" fn(&Object, Sel, Point) -> *mut Object,
        );
        decl.add_method(
            sel!(isFlipped),
            overlay_is_flipped as extern "C" fn(&Object, Sel) -> BOOL,
        );
    }
    Some(decl.register())
}

/// The overlay is a pass-through container, not an element of its own.
extern "C" fn overlay_is_element(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

extern "C" fn overlay_role(_this: &Object, _cmd: Sel) -> *mut Object {
    // `autorelease` consumes the owning pointer, handing its one reference to
    // the pool; releasing the pointer *and* autoreleasing it would free the
    // string while AppKit still holds it.
    ns_string("AXGroup").autorelease()
}

/// Nothing is ever hit: pointer events belong to winit's view underneath.
extern "C" fn overlay_hit_test(_this: &Object, _cmd: Sel, _point: Point) -> *mut Object {
    std::ptr::null_mut()
}

/// Window coordinates start at the top left, like the widget tree's.
extern "C" fn overlay_is_flipped(_this: &Object, _cmd: Sel) -> BOOL {
    YES
}

/// The application's outermost elements.
///
/// The first query is also the moment the application learns that an assistive
/// technology exists, so it wakes the event loop to publish the tree.
extern "C" fn overlay_children(this: &Object, _cmd: Sel) -> *mut Object {
    let me = this as *const Object as usize;
    let (items, first) = OVERLAYS.with(|overlays| {
        let mut overlays = overlays.borrow_mut();
        let Some(overlay) = overlays.values_mut().find(|o| *o.view as usize == me) else {
            return (Vec::new(), false);
        };
        let first = !overlay.requested;
        overlay.requested = true;
        (overlay.top.clone(), first)
    });
    if first {
        wake_event_loop();
    }
    // SAFETY: every pointer in `items` is an element kept alive by `elements`.
    unsafe {
        msg_send![
            class!(NSArray),
            arrayWithObjects: items.as_ptr()
            count: items.len()
        ]
    }
}

/// Registers the element class on first use.
fn element_class() -> Option<&'static Class> {
    if let Some(existing) = Class::get(ELEMENT_CLASS) {
        return Some(existing);
    }
    let mut decl = ClassDecl::new(ELEMENT_CLASS, Class::get("NSAccessibilityElement")?)?;
    decl.add_ivar::<u64>("aimerNode");
    decl.add_ivar::<usize>("aimerOverlay");
    decl.add_ivar::<u32>("aimerActions");
    decl.add_ivar::<f64>("aimerX");
    decl.add_ivar::<f64>("aimerY");
    decl.add_ivar::<f64>("aimerW");
    decl.add_ivar::<f64>("aimerH");
    unsafe {
        decl.add_method(
            sel!(accessibilityFrame),
            element_frame as extern "C" fn(&Object, Sel) -> Rect,
        );
        // AppKit lists an element's actions by asking which of the
        // `accessibilityPerform*` selectors it responds to, so a node that does
        // not publish an action must not claim the selector.
        decl.add_method(
            sel!(respondsToSelector:),
            element_responds as extern "C" fn(&Object, Sel, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityPerformPress),
            element_press as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityPerformIncrement),
            element_increment as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityPerformDecrement),
            element_decrement as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityPerformCancel),
            element_cancel as extern "C" fn(&Object, Sel) -> BOOL,
        );
    }
    Some(decl.register())
}

/// Bits of `aimerActions`, one per action an element accepts.
const ACTION_PRESS: u32 = 1;
const ACTION_INCREMENT: u32 = 1 << 1;
const ACTION_DECREMENT: u32 = 1 << 2;
const ACTION_CANCEL: u32 = 1 << 3;

fn actions_mask(actions: &[SemanticAction]) -> u32 {
    actions.iter().fold(0, |mask, action| {
        mask | match action {
            SemanticAction::Activate | SemanticAction::Expand | SemanticAction::Collapse => {
                ACTION_PRESS
            }
            SemanticAction::Increment => ACTION_INCREMENT,
            SemanticAction::Decrement => ACTION_DECREMENT,
            SemanticAction::Dismiss => ACTION_CANCEL,
            SemanticAction::SetValue | SemanticAction::Custom(_) => 0,
        }
    })
}

/// Answers for the action selectors from the node's published actions, and for
/// everything else as `NSAccessibilityElement` does.
extern "C" fn element_responds(this: &Object, _cmd: Sel, selector: Sel) -> BOOL {
    let bit = if selector == sel!(accessibilityPerformPress) {
        ACTION_PRESS
    } else if selector == sel!(accessibilityPerformIncrement) {
        ACTION_INCREMENT
    } else if selector == sel!(accessibilityPerformDecrement) {
        ACTION_DECREMENT
    } else if selector == sel!(accessibilityPerformCancel) {
        ACTION_CANCEL
    } else {
        // SAFETY: forwards the same message to the superclass implementation.
        return unsafe {
            msg_send![super(this, class!(NSAccessibilityElement)), respondsToSelector: selector]
        };
    };
    // SAFETY: `aimerActions` is declared on this class.
    let mask = unsafe { *this.get_ivar::<u32>("aimerActions") };
    if mask & bit != 0 { YES } else { NO }
}

/// The element's rectangle on screen, from its rectangle in the overlay.
///
/// Converted when asked, not when published, so a window that moved or was
/// resized between publishes still reports where the element is.
extern "C" fn element_frame(this: &Object, _cmd: Sel) -> Rect {
    unsafe {
        let overlay = *this.get_ivar::<usize>("aimerOverlay") as *mut Object;
        let local = Rect {
            origin: Point { x: *this.get_ivar("aimerX"), y: *this.get_ivar("aimerY") },
            size: Size { width: *this.get_ivar("aimerW"), height: *this.get_ivar("aimerH") },
        };
        if overlay.is_null() {
            return Rect::default();
        }
        let window: *mut Object = msg_send![overlay, window];
        if window.is_null() {
            return Rect::default();
        }
        let in_window: Rect = msg_send![overlay, convertRect: local toView: std::ptr::null_mut::<Object>()];
        msg_send![window, convertRectToScreen: in_window]
    }
}

/// Posts `action` for the element's node when the node accepts `bit`.
fn perform(this: &Object, bit: u32, action: SemanticAction) -> BOOL {
    let (node, overlay, mask) = unsafe {
        (
            *this.get_ivar::<u64>("aimerNode"),
            *this.get_ivar::<usize>("aimerOverlay"),
            *this.get_ivar::<u32>("aimerActions"),
        )
    };
    if mask & bit == 0 {
        return NO;
    }
    let window = OVERLAYS.with(|overlays| {
        overlays
            .borrow()
            .iter()
            .find(|(_, o)| *o.view as usize == overlay)
            .map(|(key, _)| *key)
    });
    let Some(window) = window else {
        return NO;
    };
    if ACTIONS.post(window, ActionRequest::new(NodeId::new(node), action)) {
        wake_event_loop();
    }
    YES
}

extern "C" fn element_press(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_PRESS, SemanticAction::Activate)
}

extern "C" fn element_increment(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_INCREMENT, SemanticAction::Increment)
}

extern "C" fn element_decrement(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_DECREMENT, SemanticAction::Decrement)
}

extern "C" fn element_cancel(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_CANCEL, SemanticAction::Dismiss)
}

/// Adds the overlay to `window`'s content view. Idempotent.
pub fn attach(window: &Window) {
    let Some(parent) = content_view(window) else {
        return;
    };
    let key = parent as usize;
    if OVERLAYS.with(|overlays| overlays.borrow().contains_key(&key)) {
        return;
    }
    let Some(class) = overlay_class() else {
        return;
    };
    // SAFETY: `parent` is the live content view of a winit window and this runs
    // on the main thread. The superview retains the overlay once added; the
    // `StrongPtr` keeps it for the registry.
    let view = unsafe {
        let bounds: Rect = msg_send![parent, bounds];
        let view: *mut Object = msg_send![class, alloc];
        let view: *mut Object = msg_send![view, initWithFrame: bounds];
        if view.is_null() {
            return;
        }
        let _: () = msg_send![view, setAutoresizingMask: AUTORESIZE_FILL];
        let _: () = msg_send![parent, addSubview: view];
        StrongPtr::new(view)
    };
    OVERLAYS.with(|overlays| {
        overlays.borrow_mut().insert(
            key,
            Overlay {
                view,
                elements: HashMap::new(),
                top: Vec::new(),
                shape: Vec::new(),
                requested: false,
            },
        );
    });
}

/// Whether the system has asked `window`'s overlay for its children.
pub fn is_requested(window: &Window) -> bool {
    let Some(key) = content_view(window).map(|view| view as usize) else {
        return false;
    };
    OVERLAYS.with(|overlays| overlays.borrow().get(&key).is_some_and(|o| o.requested))
}

/// Takes the actions the system requested on `window`.
pub fn take_actions(window: &Window) -> Vec<ActionRequest> {
    content_view(window).map_or_else(Vec::new, |view| ACTIONS.drain(view as usize))
}

/// Replaces what the overlay of `window` answers with.
pub fn publish(window: &Window, snapshot: &SemanticSnapshot) {
    let Some(key) = content_view(window).map(|view| view as usize) else {
        return;
    };
    OVERLAYS.with(|overlays| {
        let mut overlays = overlays.borrow_mut();
        let Some(overlay) = overlays.get_mut(&key) else {
            return;
        };
        let Some(class) = element_class() else {
            return;
        };
        let view = *overlay.view;
        let mut seen = HashSet::new();
        let mut shape = Vec::new();
        let mut top = Vec::new();
        for child in snapshot.root().children() {
            top.push(sync_node(overlay, class, view, view, child, &mut seen, &mut shape));
        }
        overlay.elements.retain(|id, _| seen.contains(id));
        let changed = shape != overlay.shape;
        overlay.top = top;
        overlay.shape = shape;
        if changed {
            let name = ns_string("AXLayoutChanged");
            // SAFETY: `view` is the live overlay; the notification name is an
            // owned string for the duration of the call.
            unsafe { NSAccessibilityPostNotification(view, *name) };
        }
    });
}

/// Creates or updates the element for `node` and its descendants, returning it.
fn sync_node(
    overlay: &mut Overlay,
    class: &'static Class,
    overlay_view: *mut Object,
    parent: *mut Object,
    node: &SemanticNode,
    seen: &mut HashSet<u64>,
    shape: &mut Vec<u64>,
) -> *mut Object {
    let id = node.id().get();
    seen.insert(id);
    shape.push(id);
    let element: *mut Object = **overlay
        .elements
        .entry(id)
        .or_insert_with(|| unsafe {
            let object: *mut Object = msg_send![class, alloc];
            let object: *mut Object = msg_send![object, init];
            StrongPtr::new(object)
        });
    let mut children = Vec::with_capacity(node.children().len());
    for child in node.children() {
        children.push(sync_node(overlay, class, overlay_view, element, child, seen, shape));
    }
    shape.push(u64::MAX);
    // SAFETY: `element` is an instance of the element class, kept alive by the
    // overlay; every object message below is sent on the main thread.
    unsafe {
        apply(element, overlay_view, parent, node, &children);
    }
    element
}

/// Pushes `node`'s properties onto `element`.
///
/// # Safety
///
/// `element` must be an instance of the element class and `parent` and
/// `children` live Objective-C objects, on the main thread.
unsafe fn apply(
    element: *mut Object,
    overlay_view: *mut Object,
    parent: *mut Object,
    node: &SemanticNode,
    children: &[*mut Object],
) {
    unsafe {
        let this = &mut *element;
        this.set_ivar("aimerNode", node.id().get());
        this.set_ivar("aimerOverlay", overlay_view as usize);
        this.set_ivar("aimerActions", actions_mask(node.actions()));
        let bounds = node.bounds();
        this.set_ivar("aimerX", bounds.map_or(0.0, |b| f64::from(b.x())));
        this.set_ivar("aimerY", bounds.map_or(0.0, |b| f64::from(b.y())));
        this.set_ivar("aimerW", bounds.map_or(0.0, |b| f64::from(b.width())));
        this.set_ivar("aimerH", bounds.map_or(0.0, |b| f64::from(b.height())));

        let role = apple_role(node.role());
        let _: () = msg_send![element, setAccessibilityRole: *ns_string(role.role)];
        match role.subrole {
            Some(subrole) => {
                let _: () = msg_send![element, setAccessibilitySubrole: *ns_string(subrole)];
            }
            None => {
                let _: () = msg_send![element, setAccessibilitySubrole: std::ptr::null_mut::<Object>()];
            }
        }
        match node.name() {
            Some(name) => {
                let _: () = msg_send![element, setAccessibilityLabel: *ns_string(name)];
            }
            None => {
                let _: () = msg_send![element, setAccessibilityLabel: std::ptr::null_mut::<Object>()];
            }
        }
        if let Some(description) = node.description() {
            let _: () = msg_send![element, setAccessibilityHelp: *ns_string(description)];
        }
        let state = node.state();
        let _: () = msg_send![element, setAccessibilityEnabled: if state.enabled() { YES } else { NO }];

        // The value: a checked state is the number AppKit expects of a check
        // box, a range or text value the string a screen reader speaks.
        let value: *mut Object = if let Some(checked) = state.checked() {
            let number = match checked {
                CheckedState::Unchecked => 0_i64,
                CheckedState::Checked => 1,
                CheckedState::Mixed => 2,
            };
            msg_send![class!(NSNumber), numberWithLongLong: number]
        } else if let Some(value) = node.value() {
            ns_string(value).autorelease()
        } else if let Some(range) = node.value_range() {
            msg_send![class!(NSNumber), numberWithDouble: f64::from(range.current())]
        } else {
            std::ptr::null_mut()
        };
        let _: () = msg_send![element, setAccessibilityValue: value];
        if let Some(selected) = state.selected() {
            let _: () = msg_send![element, setAccessibilitySelected: if selected { YES } else { NO }];
        }
        if let Some(expanded) = state.expanded() {
            let _: () = msg_send![element, setAccessibilityExpanded: if expanded { YES } else { NO }];
        }

        let _: () = msg_send![element, setAccessibilityParent: parent];
        let array: *mut Object = msg_send![
            class!(NSArray),
            arrayWithObjects: children.as_ptr()
            count: children.len()
        ];
        let _: () = msg_send![element, setAccessibilityChildren: array];
    }
}

#[cfg(test)]
mod tests {
    use aimer_accessibility::{NodeId, Role, SemanticNode};

    use super::*;

    /// The value a screen reader reads must still be there after the autorelease
    /// pool of the call that set it is drained.
    ///
    /// Short ASCII strings are tagged pointers that are never freed, which
    /// hides a reference-count mistake; a string of bullets is heap allocated,
    /// and it is what a password field publishes.
    #[test]
    fn a_published_text_value_outlives_the_pool_it_was_set_in() {
        let bullets = "\u{2022}".repeat(12);
        let node = SemanticNode::new(NodeId::new(1), Role::TextField).with_value(bullets.as_str());
        // SAFETY: the element class is registered before use, every message is
        // sent from this one thread, and `parent`/`children` may be empty/null.
        unsafe {
            let class = element_class().expect("the element class registers");
            let element: *mut Object = msg_send![class, alloc];
            let element: *mut Object = msg_send![element, init];
            let element = StrongPtr::new(element);

            let pool: *mut Object = msg_send![class!(NSAutoreleasePool), new];
            apply(*element, std::ptr::null_mut(), std::ptr::null_mut(), &node, &[]);
            let _: () = msg_send![pool, drain];

            let value: *mut Object = msg_send![*element, accessibilityValue];
            let expected = ns_string(&bullets);
            let equal: BOOL = msg_send![value, isEqualToString: *expected];
            assert_eq!(equal, YES);
        }
    }
}
