//! macOS: an `NSStatusItem` in the system status bar.
//!
//! AppKit objects may only be touched from the main thread, so everything here
//! holds a [`MainThreadMarker`] and `build` refuses to run anywhere else. The
//! click and menu callbacks are Objective-C actions, which need an object to
//! land on: [`TrayTarget`] is that object, and it carries the application's
//! handler.
//!
//! A status item with a menu opens it by itself on any click and never sends its
//! button action, so clicks are reported only while there is no menu.

use std::slice;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBitmapImageRep, NSControlStateValueOff, NSControlStateValueOn, NSEventMask,
    NSEventType, NSImage, NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{NSObject, NSSize, NSString, ns_string};

use super::Backend;
use crate::{TrayButton, TrayError, TrayEvent, TrayHandler, TrayIcon, TrayItemId, TrayMenu, TrayMenuEntry};

/// Height of the status bar's content, in points; the picture is scaled to it.
const ICON_POINTS: f64 = 18.0;

struct TargetIvars {
    handler: TrayHandler,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class neither
    // implements `Drop` nor is used from any thread but the main one.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AimerTrayTarget"]
    #[ivars = TargetIvars]
    struct TrayTarget;

    impl TrayTarget {
        #[unsafe(method(itemChosen:))]
        fn item_chosen(&self, sender: &NSMenuItem) {
            // The tag round-trips a `u32`, so it is never negative.
            let id = TrayItemId(sender.tag() as u32);
            (self.ivars().handler)(TrayEvent::MenuItem(id));
        }

        #[unsafe(method(statusClicked:))]
        fn status_clicked(&self, _sender: Option<&AnyObject>) {
            let mtm = self.mtm();
            let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
                return;
            };
            let button = match event.r#type() {
                NSEventType::LeftMouseUp => TrayButton::Left,
                NSEventType::RightMouseUp => TrayButton::Right,
                NSEventType::OtherMouseUp => TrayButton::Middle,
                _ => return,
            };
            let handler = &self.ivars().handler;
            handler(TrayEvent::Click(button));
            if event.clickCount() == 2 {
                handler(TrayEvent::DoubleClick(button));
            }
        }
    }
);

impl TrayTarget {
    fn new(mtm: MainThreadMarker, handler: TrayHandler) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(TargetIvars { handler });
        // SAFETY: `init` of `NSObject` takes no arguments.
        unsafe { msg_send![super(this), init] }
    }
}

struct MacTray {
    mtm: MainThreadMarker,
    item: Retained<NSStatusItem>,
    target: Retained<TrayTarget>,
    has_menu: bool,
}

pub(super) fn create(
    icon: &TrayIcon,
    tooltip: &str,
    menu: Option<&TrayMenu>,
    handler: TrayHandler,
) -> Result<Box<dyn Backend>, TrayError> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| TrayError::Platform("a tray icon must be created on the main thread".into()))?;
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    let target = TrayTarget::new(mtm, handler);
    let mut tray = MacTray { mtm, item, target, has_menu: false };
    tray.set_icon(icon);
    tray.set_tooltip(tooltip);
    tray.set_menu(menu);
    Ok(Box::new(tray))
}

impl Backend for MacTray {
    fn set_icon(&mut self, icon: &TrayIcon) {
        let Some(image) = make_image(icon) else { return };
        if let Some(button) = self.item.button(self.mtm) {
            button.setImage(Some(&image));
        }
    }

    fn set_tooltip(&mut self, tooltip: &str) {
        if let Some(button) = self.item.button(self.mtm) {
            button.setToolTip(Some(&NSString::from_str(tooltip)));
        }
    }

    fn set_menu(&mut self, menu: Option<&TrayMenu>) {
        match menu {
            Some(menu) => {
                let native = build_menu(self.mtm, &self.target, menu);
                self.item.setMenu(Some(&native));
            }
            None => {
                self.item.setMenu(None);
                if let Some(button) = self.item.button(self.mtm) {
                    // SAFETY: `statusClicked:` is defined on `TrayTarget`, which
                    // the tray keeps alive for as long as the button exists.
                    unsafe {
                        button.setTarget(Some(&self.target));
                        button.setAction(Some(sel!(statusClicked:)));
                    }
                    button.sendActionOn(NSEventMask::LeftMouseUp | NSEventMask::RightMouseUp | NSEventMask::OtherMouseUp);
                }
            }
        }
        self.has_menu = menu.is_some();
    }
}

impl Drop for MacTray {
    fn drop(&mut self) {
        NSStatusBar::systemStatusBar().removeStatusItem(&self.item);
    }
}

/// A native image of `icon`, `ICON_POINTS` tall.
fn make_image(icon: &TrayIcon) -> Option<Retained<NSImage>> {
    let (width, height) = (icon.width() as isize, icon.height() as isize);
    // SAFETY: a null plane pointer asks AppKit to allocate the pixel storage;
    // the other arguments describe straight 8-bit RGBA, which AppKit's default
    // bitmap format is. The copy below fills exactly the allocated rows.
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            width,
            height,
            8,
            4,
            true,
            false,
            ns_string!("NSDeviceRGBColorSpace"),
            width * 4,
            32,
        )?
    };
    let data = rep.bitmapData();
    if data.is_null() {
        return None;
    }
    let pixels = icon.rgba();
    // SAFETY: the representation owns `width * 4 * height` bytes, which is
    // `pixels.len()` by `TrayIcon`'s invariant, and nothing else aliases them.
    unsafe { slice::from_raw_parts_mut(data, pixels.len()).copy_from_slice(pixels) };

    let aspect = icon.width() as f64 / icon.height() as f64;
    let image = NSImage::initWithSize(NSImage::alloc(), NSSize::new(ICON_POINTS * aspect, ICON_POINTS));
    image.addRepresentation(&rep);
    Some(image)
}

fn build_menu(mtm: MainThreadMarker, target: &TrayTarget, menu: &TrayMenu) -> Retained<NSMenu> {
    let native = NSMenu::new(mtm);
    // Without this AppKit re-enables items that have a live target, ignoring
    // `TrayMenuItem::enabled`.
    native.setAutoenablesItems(false);
    for entry in menu.entries() {
        match entry {
            TrayMenuEntry::Separator => native.addItem(&NSMenuItem::separatorItem(mtm)),
            TrayMenuEntry::Item(item) => {
                // SAFETY: `itemChosen:` is defined on `TrayTarget` with one
                // `NSMenuItem` argument, matching this action's signature.
                let native_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        mtm.alloc(),
                        &NSString::from_str(&item.title),
                        Some(sel!(itemChosen:)),
                        ns_string!(""),
                    )
                };
                native_item.setTag(item.id.0 as isize);
                // SAFETY: the target outlives the menu: the tray owns both.
                unsafe { native_item.setTarget(Some(target)) };
                native_item.setEnabled(item.enabled);
                if let Some(checked) = item.checked {
                    native_item.setState(if checked { NSControlStateValueOn } else { NSControlStateValueOff });
                }
                native.addItem(&native_item);
            }
            TrayMenuEntry::Submenu { title, enabled, menu } => {
                // SAFETY: no action is installed, so there is nothing to mismatch.
                let native_item = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        mtm.alloc(),
                        &NSString::from_str(title),
                        None,
                        ns_string!(""),
                    )
                };
                native_item.setEnabled(*enabled);
                native_item.setSubmenu(Some(&build_menu(mtm, target, menu)));
                native.addItem(&native_item);
            }
        }
    }
    native
}
