//! macOS global shortcuts through Carbon's hot-key API.
//!
//! `RegisterEventHotKey` is the one system-wide key facility that needs no
//! Accessibility permission, and it is what every Mac launcher uses. Hot keys
//! are delivered as Carbon events to the application's event target, which
//! AppKit's own event loop (winit's) already services, so there is no thread
//! to run: the handler fires on the main thread.

use std::collections::HashMap;
use std::ffi::c_void;

use super::*;
use crate::global_keys::{carbon_key, carbon_mods};

type OSStatus = i32;
type EventRef = *mut c_void;
type EventHandlerCallRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

/// `'keyb'`.
const EVENT_CLASS_KEYBOARD: u32 = 0x6B65_7962;
/// `kEventHotKeyPressed`.
const EVENT_HOT_KEY_PRESSED: u32 = 5;
/// `'----'`, `kEventParamDirectObject`.
const EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D_2D2D;
/// `'hkid'`, `typeEventHotKeyID`.
const TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B_6964;
/// `'AIMR'`: marks the hot keys as this framework's.
const SIGNATURE: u32 = 0x4149_4D52;
/// `eventHotKeyExistsErr`.
const HOT_KEY_EXISTS: OSStatus = -9878;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> EventTargetRef;
    fn InstallEventHandler(
        target: EventTargetRef,
        handler: extern "C" fn(EventHandlerCallRef, EventRef, *mut c_void) -> OSStatus,
        num_types: u32,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
    fn RegisterEventHotKey(
        key_code: u32,
        modifiers: u32,
        id: EventHotKeyID,
        target: EventTargetRef,
        options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn UnregisterEventHotKey(hot_key: EventHotKeyRef) -> OSStatus;
    fn GetEventParameter(
        event: EventRef,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> OSStatus;
}

/// Runs on the main thread for every hot key press of this application.
extern "C" fn on_hot_key(_: EventHandlerCallRef, event: EventRef, _: *mut c_void) -> OSStatus {
    let mut hot_key = EventHotKeyID { signature: 0, id: 0 };
    // SAFETY: `event` is the live event Carbon is delivering, and the buffer is
    // exactly the size of the `EventHotKeyID` the parameter is documented as.
    let status = unsafe {
        GetEventParameter(
            event,
            EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            std::ptr::null_mut(),
            (&raw mut hot_key).cast(),
        )
    };
    if status == 0 && hot_key.signature == SIGNATURE {
        notify(GlobalShortcutId::from_raw(hot_key.id));
    }
    0
}

struct Carbon {
    handler_installed: bool,
    keys: HashMap<GlobalShortcutId, EventHotKeyRef>,
}

impl Carbon {
    fn install_handler(&mut self) -> Result<(), GlobalShortcutError> {
        if self.handler_installed {
            return Ok(());
        }
        let spec = EventTypeSpec { event_class: EVENT_CLASS_KEYBOARD, event_kind: EVENT_HOT_KEY_PRESSED };
        // SAFETY: the target is the process-wide application target, `spec`
        // outlives the call (Carbon copies it), and the handler is a plain
        // function that stays valid for the life of the process.
        let status = unsafe {
            InstallEventHandler(
                GetApplicationEventTarget(),
                on_hot_key,
                1,
                &raw const spec,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if status != 0 {
            return Err(GlobalShortcutError::Platform(format!("InstallEventHandler failed ({status})")));
        }
        self.handler_installed = true;
        Ok(())
    }
}

impl Backend for Carbon {
    fn register(&mut self, id: GlobalShortcutId, chord: &KeyChord) -> Result<(), GlobalShortcutError> {
        let key = carbon_key(chord.key()).ok_or(GlobalShortcutError::KeyNotSupported)?;
        self.install_handler()?;
        let mut hot_key: EventHotKeyRef = std::ptr::null_mut();
        // SAFETY: `hot_key` is a valid out pointer, and the target is the
        // process-wide application target the handler was installed on.
        let status = unsafe {
            RegisterEventHotKey(
                key,
                carbon_mods(chord.mods()),
                EventHotKeyID { signature: SIGNATURE, id: id.as_raw() },
                GetApplicationEventTarget(),
                0,
                &raw mut hot_key,
            )
        };
        match status {
            0 if !hot_key.is_null() => {
                self.keys.insert(id, hot_key);
                Ok(())
            }
            HOT_KEY_EXISTS => Err(GlobalShortcutError::Taken),
            other => Err(GlobalShortcutError::Platform(format!("RegisterEventHotKey failed ({other})"))),
        }
    }

    fn unregister(&mut self, id: GlobalShortcutId) {
        if let Some(hot_key) = self.keys.remove(&id) {
            // SAFETY: `hot_key` came from a successful `RegisterEventHotKey`
            // and is removed from the map, so it is unregistered exactly once.
            unsafe { UnregisterEventHotKey(hot_key) };
        }
    }
}

impl Drop for Carbon {
    fn drop(&mut self) {
        for (_, hot_key) in self.keys.drain() {
            // SAFETY: as in `unregister`; draining releases each key once.
            unsafe { UnregisterEventHotKey(hot_key) };
        }
    }
}

pub(super) fn backend() -> Box<dyn Backend> {
    Box::new(Carbon { handler_installed: false, keys: HashMap::new() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(text: &str) -> KeyChord {
        KeyChord::parse_with_primary(text, ChordMods::META).expect(text)
    }

    /// Talks to the real window server's hot-key table, so it needs a logged-in
    /// macOS session and is run on demand.
    #[test]
    #[ignore = "needs a macOS login session"]
    fn the_system_accepts_a_hot_key_and_refuses_a_second_claim_on_it() {
        let mut carbon = Carbon { handler_installed: false, keys: HashMap::new() };
        let unlikely = chord("Ctrl+Alt+Shift+Meta+F19");

        carbon.register(GlobalShortcutId::from_raw(1), &unlikely).expect("a free chord registers");
        assert_eq!(
            carbon.register(GlobalShortcutId::from_raw(2), &unlikely),
            Err(GlobalShortcutError::Taken)
        );

        carbon.unregister(GlobalShortcutId::from_raw(1));
        carbon.register(GlobalShortcutId::from_raw(3), &unlikely).expect("released chords are free");
    }

    #[test]
    fn a_key_without_a_carbon_code_is_refused_before_the_system_is_asked() {
        let mut carbon = Carbon { handler_installed: false, keys: HashMap::new() };
        assert_eq!(
            carbon.register(GlobalShortcutId::from_raw(1), &chord("Cmd+Plus")),
            Err(GlobalShortcutError::KeyNotSupported)
        );
        assert!(!carbon.handler_installed);
    }
}
