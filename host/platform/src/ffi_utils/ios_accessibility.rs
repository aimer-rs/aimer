//! The UIKit half of [`crate::accessibility`].
//!
//! UIKit asks a view for `accessibilityElements`: a flat, ordered list that
//! VoiceOver walks. winit owns the window's root view, so a transparent overlay
//! view is added on top of it (`hitTest:` answers `nil`, so every touch still
//! reaches winit's view) and the overlay answers with
//! `UIAccessibilityElement` objects, kept by node and updated in place so
//! VoiceOver's cursor survives a publish.
//!
//! The tree is flattened: a node that can be activated is one element and its
//! descendants are not separate stops, which is how VoiceOver users expect a
//! button with a label inside it to behave.
//!
//! Unlike AppKit, UIKit can say whether VoiceOver is running, so the tree is
//! kept current for as long as it is.
//!
//! **Not exercised on a device or simulator.** It compiles for iOS and shares
//! its logic with the macOS bridge, which is verified; see the module tests in
//! [`crate::accessibility`] for the part that is.

// The `objc` 0.2 macros test for a `cargo-clippy` cfg this workspace never sets.
#![allow(unexpected_cfgs)]

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use aimer_accessibility::{
    ActionRequest, NodeId, SemanticAction, SemanticNode, SemanticSnapshot,
};
use objc::declare::ClassDecl;
use objc::rc::StrongPtr;
use objc::runtime::{BOOL, Class, NO, Object, Sel, YES};
use objc::{class, msg_send, sel, sel_impl};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window;

use crate::accessibility::{ACTIONS, IosTrait, ios_traits, wake_event_loop};
use crate::ffi_utils::apple_accessibility::{Point, Rect, Size, ns_string};

const OVERLAY_CLASS: &str = "AimerAccessibilityOverlay";
const ELEMENT_CLASS: &str = "AimerAccessibilityElement";

/// `UIViewAutoresizingFlexibleWidth | UIViewAutoresizingFlexibleHeight`.
const AUTORESIZE_FILL: usize = 2 | 16;

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" {
    static UIAccessibilityTraitButton: u64;
    static UIAccessibilityTraitLink: u64;
    static UIAccessibilityTraitImage: u64;
    static UIAccessibilityTraitHeader: u64;
    static UIAccessibilityTraitStaticText: u64;
    static UIAccessibilityTraitAdjustable: u64;
    static UIAccessibilityTraitSearchField: u64;
    static UIAccessibilityTraitSelected: u64;
    static UIAccessibilityTraitNotEnabled: u64;
    static UIAccessibilityLayoutChangedNotification: u32;

    fn UIAccessibilityIsVoiceOverRunning() -> BOOL;
    fn UIAccessibilityIsSwitchControlRunning() -> BOOL;
    fn UIAccessibilityPostNotification(notification: u32, argument: *mut Object);
    fn UIAccessibilityConvertFrameToScreenCoordinates(rect: Rect, view: *mut Object) -> Rect;
}

/// What the overlay of one window holds.
struct Overlay {
    /// The overlay view, owned here and by its superview.
    view: StrongPtr,
    /// Native elements by node, kept across publishes.
    elements: HashMap<u64, StrongPtr>,
    /// The elements VoiceOver walks, in reading order.
    ordered: Vec<*mut Object>,
    /// Node order of the last publish, to tell a structural change from a
    /// property one.
    shape: Vec<u64>,
    /// Set the first time UIKit asks for the elements.
    requested: bool,
}

thread_local! {
    /// Overlays by the address of the winit view they sit on.
    static OVERLAYS: RefCell<HashMap<usize, Overlay>> = RefCell::new(HashMap::new());
}

/// The address of `window`'s root view, which names its overlay.
fn root_view(window: &Window) -> Option<*mut Object> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::UiKit(uikit) => Some(uikit.ui_view.as_ptr().cast()),
        _ => None,
    }
}

/// Registers the overlay view class on first use.
fn overlay_class() -> Option<&'static Class> {
    if let Some(existing) = Class::get(OVERLAY_CLASS) {
        return Some(existing);
    }
    let mut decl = ClassDecl::new(OVERLAY_CLASS, Class::get("UIView")?)?;
    unsafe {
        decl.add_method(
            sel!(isAccessibilityElement),
            overlay_is_element as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityElements),
            overlay_elements as extern "C" fn(&Object, Sel) -> *mut Object,
        );
        decl.add_method(
            sel!(hitTest:withEvent:),
            overlay_hit_test as extern "C" fn(&Object, Sel, Point, *mut Object) -> *mut Object,
        );
    }
    Some(decl.register())
}

/// The overlay is a container, not an element of its own.
extern "C" fn overlay_is_element(_this: &Object, _cmd: Sel) -> BOOL {
    NO
}

/// Nothing is ever hit: touches belong to winit's view underneath.
extern "C" fn overlay_hit_test(
    _this: &Object,
    _cmd: Sel,
    _point: Point,
    _event: *mut Object,
) -> *mut Object {
    std::ptr::null_mut()
}

/// The elements VoiceOver walks.
///
/// The first query also tells the application that an assistive technology
/// exists, so it wakes the event loop to publish the tree.
extern "C" fn overlay_elements(this: &Object, _cmd: Sel) -> *mut Object {
    let me = this as *const Object as usize;
    let (items, first) = OVERLAYS.with(|overlays| {
        let mut overlays = overlays.borrow_mut();
        let Some(overlay) = overlays.values_mut().find(|o| *o.view as usize == me) else {
            return (Vec::new(), false);
        };
        let first = !overlay.requested;
        overlay.requested = true;
        (overlay.ordered.clone(), first)
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
    let mut decl = ClassDecl::new(ELEMENT_CLASS, Class::get("UIAccessibilityElement")?)?;
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
        decl.add_method(
            sel!(accessibilityActivate),
            element_activate as extern "C" fn(&Object, Sel) -> BOOL,
        );
        decl.add_method(
            sel!(accessibilityIncrement),
            element_increment as extern "C" fn(&Object, Sel),
        );
        decl.add_method(
            sel!(accessibilityDecrement),
            element_decrement as extern "C" fn(&Object, Sel),
        );
        decl.add_method(
            sel!(accessibilityPerformEscape),
            element_escape as extern "C" fn(&Object, Sel) -> BOOL,
        );
    }
    Some(decl.register())
}

/// Bits of `aimerActions`, one per action an element accepts.
const ACTION_ACTIVATE: u32 = 1;
const ACTION_INCREMENT: u32 = 1 << 1;
const ACTION_DECREMENT: u32 = 1 << 2;
const ACTION_ESCAPE: u32 = 1 << 3;

fn actions_mask(actions: &[SemanticAction]) -> u32 {
    actions.iter().fold(0, |mask, action| {
        mask | match action {
            SemanticAction::Activate | SemanticAction::Expand | SemanticAction::Collapse => {
                ACTION_ACTIVATE
            }
            SemanticAction::Increment => ACTION_INCREMENT,
            SemanticAction::Decrement => ACTION_DECREMENT,
            SemanticAction::Dismiss => ACTION_ESCAPE,
            SemanticAction::SetValue | SemanticAction::Custom(_) => 0,
        }
    })
}

/// The element's rectangle on screen, from its rectangle in the overlay.
///
/// Converted when asked, so a rotation or a window move between publishes still
/// reports where the element is.
extern "C" fn element_frame(this: &Object, _cmd: Sel) -> Rect {
    // SAFETY: the ivars are declared on this class; the overlay view outlives
    // its elements' use by UIKit because the registry keeps both.
    unsafe {
        let overlay = *this.get_ivar::<usize>("aimerOverlay") as *mut Object;
        if overlay.is_null() {
            return Rect::default();
        }
        let local = Rect {
            origin: Point { x: *this.get_ivar("aimerX"), y: *this.get_ivar("aimerY") },
            size: Size { width: *this.get_ivar("aimerW"), height: *this.get_ivar("aimerH") },
        };
        UIAccessibilityConvertFrameToScreenCoordinates(local, overlay)
    }
}

/// Posts `action` for the element's node when the node accepts `bit`.
fn perform(this: &Object, bit: u32, action: SemanticAction) -> BOOL {
    // SAFETY: the ivars are declared on this class.
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

extern "C" fn element_activate(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_ACTIVATE, SemanticAction::Activate)
}

extern "C" fn element_increment(this: &Object, _cmd: Sel) {
    perform(this, ACTION_INCREMENT, SemanticAction::Increment);
}

extern "C" fn element_decrement(this: &Object, _cmd: Sel) {
    perform(this, ACTION_DECREMENT, SemanticAction::Decrement);
}

extern "C" fn element_escape(this: &Object, _cmd: Sel) -> BOOL {
    perform(this, ACTION_ESCAPE, SemanticAction::Dismiss)
}

/// Adds the overlay to `window`'s root view. Idempotent.
pub fn attach(window: &Window) {
    let Some(parent) = root_view(window) else {
        return;
    };
    let key = parent as usize;
    if OVERLAYS.with(|overlays| overlays.borrow().contains_key(&key)) {
        return;
    }
    let Some(class) = overlay_class() else {
        return;
    };
    // SAFETY: `parent` is the live root view of a winit window and this runs on
    // the main thread. The superview retains the overlay once added; the
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
                ordered: Vec::new(),
                shape: Vec::new(),
                requested: false,
            },
        );
    });
}

/// Whether an assistive technology is attached to `window`: UIKit has said so,
/// or it has asked the overlay for its elements.
pub fn is_requested(window: &Window) -> bool {
    let Some(key) = root_view(window).map(|view| view as usize) else {
        return false;
    };
    let queried = OVERLAYS.with(|overlays| overlays.borrow().get(&key).is_some_and(|o| o.requested));
    // SAFETY: plain UIKit status queries with no arguments.
    queried || unsafe { UIAccessibilityIsVoiceOverRunning() == YES || UIAccessibilityIsSwitchControlRunning() == YES }
}

/// Takes the actions the system requested on `window`.
pub fn take_actions(window: &Window) -> Vec<ActionRequest> {
    root_view(window).map_or_else(Vec::new, |view| ACTIONS.drain(view as usize))
}

/// Replaces what the overlay of `window` answers with.
pub fn publish(window: &Window, snapshot: &SemanticSnapshot) {
    let Some(key) = root_view(window).map(|view| view as usize) else {
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
        let mut ordered = Vec::new();
        let mut shape = Vec::new();
        for child in snapshot.root().children() {
            flatten(overlay, class, view, child, &mut seen, &mut ordered, &mut shape);
        }
        overlay.elements.retain(|id, _| seen.contains(id));
        let changed = shape != overlay.shape;
        overlay.ordered = ordered;
        overlay.shape = shape;
        if changed {
            // SAFETY: a notification with no argument; UIKit re-reads the
            // elements.
            unsafe { UIAccessibilityPostNotification(UIAccessibilityLayoutChangedNotification, std::ptr::null_mut()) };
        }
    });
}

/// Whether `node` is announced on its own, as opposed to being a bare grouping.
fn is_stop(node: &SemanticNode) -> bool {
    node.name().is_some() || node.value().is_some() || !node.actions().is_empty()
}

/// Appends the elements for `node` and its descendants, in reading order.
fn flatten(
    overlay: &mut Overlay,
    class: &'static Class,
    overlay_view: *mut Object,
    node: &SemanticNode,
    seen: &mut HashSet<u64>,
    ordered: &mut Vec<*mut Object>,
    shape: &mut Vec<u64>,
) {
    let activatable = node.actions().contains(&SemanticAction::Activate);
    if is_stop(node) {
        let id = node.id().get();
        seen.insert(id);
        shape.push(id);
        let element: *mut Object = **overlay.elements.entry(id).or_insert_with(|| unsafe {
            let object: *mut Object = msg_send![class, alloc];
            let object: *mut Object =
                msg_send![object, initWithAccessibilityContainer: overlay_view];
            StrongPtr::new(object)
        });
        // SAFETY: `element` is an instance of the element class kept alive by
        // the overlay, on the main thread.
        unsafe { apply(element, overlay_view, node) };
        ordered.push(element);
    }
    // An activatable node is one stop; whatever it contains is its label.
    if !activatable {
        for child in node.children() {
            flatten(overlay, class, overlay_view, child, seen, ordered, shape);
        }
    }
}

/// The UIKit trait bit for `which`.
fn trait_bits(which: IosTrait) -> u64 {
    // SAFETY: reads UIKit's immutable trait constants.
    unsafe {
        match which {
            IosTrait::Button => UIAccessibilityTraitButton,
            IosTrait::Link => UIAccessibilityTraitLink,
            IosTrait::Image => UIAccessibilityTraitImage,
            IosTrait::Header => UIAccessibilityTraitHeader,
            IosTrait::StaticText => UIAccessibilityTraitStaticText,
            IosTrait::Adjustable => UIAccessibilityTraitAdjustable,
            IosTrait::SearchField => UIAccessibilityTraitSearchField,
        }
    }
}

/// Pushes `node`'s properties onto `element`.
///
/// # Safety
///
/// `element` must be an instance of the element class and `overlay_view` a live
/// view, on the main thread.
unsafe fn apply(element: *mut Object, overlay_view: *mut Object, node: &SemanticNode) {
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

        match node.name() {
            Some(name) => {
                let _: () = msg_send![element, setAccessibilityLabel: *ns_string(name)];
            }
            None => {
                let _: () = msg_send![element, setAccessibilityLabel: std::ptr::null_mut::<Object>()];
            }
        }
        if let Some(description) = node.description() {
            let _: () = msg_send![element, setAccessibilityHint: *ns_string(description)];
        }
        // VoiceOver speaks the value after the label; a checked state is the
        // words it would otherwise lack.
        let value = node.value().map(str::to_owned).or_else(|| {
            node.state().checked().map(|checked| {
                match checked {
                    aimer_accessibility::CheckedState::Unchecked => "off",
                    aimer_accessibility::CheckedState::Checked => "on",
                    aimer_accessibility::CheckedState::Mixed => "mixed",
                }
                .to_owned()
            })
        });
        match value {
            Some(value) => {
                let _: () = msg_send![element, setAccessibilityValue: *ns_string(&value)];
            }
            None => {
                let _: () = msg_send![element, setAccessibilityValue: std::ptr::null_mut::<Object>()];
            }
        }

        let state = node.state();
        let mut traits = ios_traits(node.role()).iter().fold(0_u64, |bits, t| bits | trait_bits(*t));
        if !state.enabled() {
            traits |= UIAccessibilityTraitNotEnabled;
        }
        if state.selected() == Some(true) || state.checked() == Some(aimer_accessibility::CheckedState::Checked) {
            traits |= UIAccessibilityTraitSelected;
        }
        let _: () = msg_send![element, setAccessibilityTraits: traits];
    }
}
