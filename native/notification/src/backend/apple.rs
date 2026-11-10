//! macOS and iOS: `UNUserNotificationCenter`.
//!
//! The center talks back on threads of its own, so the delegate is an
//! any-thread Objective-C object that carries the application's handler, and
//! every completion block below only touches `Send` state.
//!
//! The center holds its delegate weakly: the backend owns it and clears it again
//! on drop.
//!
//! `UNUserNotificationCenter::currentNotificationCenter` raises an Objective-C
//! exception in a process that is not inside an app bundle, so [`create`]
//! looks for a bundle identifier first and never reaches the center without one.
//!
//! Buttons need a `UNNotificationCategory`, and `setNotificationCategories`
//! replaces the whole set. Categories are therefore shared by action set and
//! accumulated in the backend, so registering one never drops another.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::{Bool, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_foundation::{NSArray, NSBundle, NSError, NSObject, NSObjectProtocol, NSSet, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
    UNNotificationAction, UNNotificationActionOptions, UNNotificationCategory,
    UNNotificationCategoryOptions, UNNotificationPresentationOptions, UNNotificationRequest,
    UNNotificationResponse, UNNotificationSettings, UNNotificationSound, UNNotificationTrigger,
    UNTimeIntervalNotificationTrigger, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
};

use super::Backend;
use crate::{
    Notification, NotificationAction, NotificationActionId, NotificationError, NotificationEvent,
    NotificationHandler, NotificationId, Permission, PermissionCallback,
};

/// The system refuses triggers shorter than this.
const MIN_DELAY_SECONDS: f64 = 1.0;

/// The identifier of a tap on the notification itself.
///
/// This is the value of `UNNotificationDefaultActionIdentifier`. It is spelled
/// out instead of read from the framework's exported symbol: a static library
/// linked by an Xcode project only resolves a framework symbol when the
/// project's link flags already name `UserNotifications`, and a build that runs
/// with flags from before this crate was added fails to link. A test checks the
/// text against the real symbol.
const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";

/// The identifier of a dismissal; the value of
/// `UNNotificationDismissActionIdentifier`. See [`DEFAULT_ACTION`].
const DISMISS_ACTION: &str = "com.apple.UNNotificationDismissActionIdentifier";

/// How long `show` waits for the center to accept a request before assuming it
/// did. The center answers from its own queue, normally within a millisecond or
/// two; the bound keeps a stalled daemon from freezing the interface.
const ACCEPT_TIMEOUT: Duration = Duration::from_millis(100);

struct DelegateIvars {
    handler: NotificationHandler,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does not
    // implement `Drop`. The ivars are `Send + Sync`, so any thread may call in.
    #[unsafe(super(NSObject))]
    #[name = "AimerNotificationDelegate"]
    #[ivars = DelegateIvars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}

    unsafe impl UNUserNotificationCenterDelegate for Delegate {
        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: &UNUserNotificationCenter,
            _notification: &UNNotification,
            completion: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
        ) {
            // Without this the system drops notifications while the app is
            // frontmost.
            completion.call((UNNotificationPresentationOptions::Banner
                | UNNotificationPresentationOptions::List
                | UNNotificationPresentationOptions::Sound,));
        }

        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: &UNUserNotificationCenter,
            response: &UNNotificationResponse,
            completion: &block2::DynBlock<dyn Fn()>,
        ) {
            let identifier = response.notification().request().identifier().to_string();
            let action = response.actionIdentifier().to_string();
            if let Some(id) = parse_id(&identifier)
                && let Some(event) = response_event(&action, id, DEFAULT_ACTION, DISMISS_ACTION)
            {
                (self.ivars().handler)(event);
            }
            completion.call(());
        }
    }
);

impl Delegate {
    fn new(handler: NotificationHandler) -> Retained<Self> {
        let this = Self::alloc().set_ivars(DelegateIvars { handler });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

struct AppleBackend {
    center: Retained<UNUserNotificationCenter>,
    /// Kept alive because the center only holds it weakly.
    delegate: Retained<Delegate>,
    /// Every category registered so far, by identifier.
    categories: Mutex<Vec<(String, Retained<UNNotificationCategory>)>>,
}

/// Connects to the center, provided the process is inside an app bundle.
pub(super) fn create(handler: NotificationHandler) -> Result<Box<dyn Backend>, NotificationError> {
    let has_bundle = NSBundle::mainBundle().bundleIdentifier().is_some_and(|id| id.length() > 0);
    if !has_bundle {
        return Err(NotificationError::Platform(
            "notifications need an app bundle: run the application from a .app (or an iOS app), not a bare executable"
                .into(),
        ));
    }
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let delegate = Delegate::new(handler);
    center.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    Ok(Box::new(AppleBackend { center, delegate, categories: Mutex::new(Vec::new()) }))
}

impl Drop for AppleBackend {
    fn drop(&mut self) {
        // The delegate dies with `self`; the center must not keep a dangling
        // pointer to it.
        let _ = &self.delegate;
        self.center.setDelegate(None);
    }
}

impl AppleBackend {
    /// Makes sure a category with these buttons is registered and returns its
    /// identifier.
    fn register_category(&self, actions: &[NotificationAction]) -> String {
        let identifier = category_identifier(actions);
        let mut categories = self.categories.lock().unwrap_or_else(PoisonError::into_inner);
        if categories.iter().any(|(known, _)| *known == identifier) {
            return identifier;
        }
        let buttons: Vec<Retained<UNNotificationAction>> = actions
            .iter()
            .map(|action| {
                UNNotificationAction::actionWithIdentifier_title_options(
                    &NSString::from_str(&action.id.0.to_string()),
                    &NSString::from_str(&action.label),
                    UNNotificationActionOptions::Foreground,
                )
            })
            .collect();
        let category = UNNotificationCategory::categoryWithIdentifier_actions_intentIdentifiers_options(
            &NSString::from_str(&identifier),
            &NSArray::from_retained_slice(&buttons),
            &NSArray::<NSString>::new(),
            // Without this, swiping a notification that has buttons reports nothing.
            UNNotificationCategoryOptions::CustomDismissAction,
        );
        categories.push((identifier.clone(), category));
        let all: Vec<Retained<UNNotificationCategory>> =
            categories.iter().map(|(_, category)| category.clone()).collect();
        self.center.setNotificationCategories(&NSSet::from_retained_slice(&all));
        identifier
    }
}

impl Backend for AppleBackend {
    fn show(&self, notification: &Notification) -> Result<(), NotificationError> {
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&notification.title));
        if let Some(subtitle) = &notification.subtitle {
            content.setSubtitle(&NSString::from_str(subtitle));
        }
        content.setBody(&NSString::from_str(&notification.body));
        if notification.sound {
            content.setSound(Some(&UNNotificationSound::defaultSound()));
        }
        if !notification.actions.is_empty() {
            let category = self.register_category(&notification.actions);
            content.setCategoryIdentifier(&NSString::from_str(&category));
        }

        let trigger: Option<Retained<UNNotificationTrigger>> = notification.delay.map(|delay| {
            Retained::into_super(UNTimeIntervalNotificationTrigger::triggerWithTimeInterval_repeats(
                delay.as_secs_f64().max(MIN_DELAY_SECONDS),
                false,
            ))
        });
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &NSString::from_str(&notification.id.0.to_string()),
            &content,
            trigger.as_deref(),
        );

        // The block runs on a queue of the center's, and runs at most once.
        let (sender, receiver) = mpsc::sync_channel::<Result<(), NotificationError>>(1);
        let block = RcBlock::new(move |error: *mut NSError| {
            let _ = sender.try_send(error_result(error));
        });
        self.center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
        match receiver.recv_timeout(ACCEPT_TIMEOUT) {
            Ok(result) => result,
            Err(_) => Ok(()),
        }
    }

    fn close(&self, id: NotificationId) -> Result<(), NotificationError> {
        let identifiers = NSArray::from_retained_slice(&[NSString::from_str(&id.0.to_string())]);
        self.center.removePendingNotificationRequestsWithIdentifiers(&identifiers);
        self.center.removeDeliveredNotificationsWithIdentifiers(&identifiers);
        Ok(())
    }

    fn permission(&self, done: PermissionCallback) {
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            // SAFETY: the center passes a live settings object for the duration
            // of the call.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            take_and_call(&done, map_status(status));
        });
        self.center.getNotificationSettingsWithCompletionHandler(&block);
    }

    fn request_permission(&self, done: PermissionCallback) {
        let done = Mutex::new(Some(done));
        let block = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            take_and_call(&done, if granted.as_bool() { Permission::Granted } else { Permission::Denied });
        });
        self.center.requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound | UNAuthorizationOptions::Badge,
            &block,
        );
    }

    fn register_push(&self) -> Result<(), NotificationError> {
        let mtm = MainThreadMarker::new().ok_or_else(|| {
            NotificationError::Platform("push registration must be requested on the main thread".into())
        })?;
        register_for_remote_notifications(mtm);
        Ok(())
    }
}

#[cfg(target_os = "ios")]
fn register_for_remote_notifications(mtm: MainThreadMarker) {
    objc2_ui_kit::UIApplication::sharedApplication(mtm).registerForRemoteNotifications();
}

#[cfg(target_os = "macos")]
fn register_for_remote_notifications(mtm: MainThreadMarker) {
    objc2_app_kit::NSApplication::sharedApplication(mtm).registerForRemoteNotifications();
}

/// Calls the callback the first time, and never again.
fn take_and_call(done: &Mutex<Option<PermissionCallback>>, permission: Permission) {
    let callback = done.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(callback) = callback {
        callback(permission);
    }
}

fn error_result(error: *mut NSError) -> Result<(), NotificationError> {
    // SAFETY: the center passes either null or a live `NSError` for the
    // duration of the call.
    match unsafe { error.as_ref() } {
        None => Ok(()),
        Some(error) => Err(NotificationError::Platform(error.localizedDescription().to_string())),
    }
}

fn map_status(status: UNAuthorizationStatus) -> Permission {
    match status {
        UNAuthorizationStatus::NotDetermined => Permission::NotDetermined,
        UNAuthorizationStatus::Authorized
        | UNAuthorizationStatus::Provisional
        | UNAuthorizationStatus::Ephemeral => Permission::Granted,
        _ => Permission::Denied,
    }
}

/// Reads a decimal identifier written by this backend. Stricter than
/// `str::parse`: no sign, no whitespace.
fn parse_id(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// The category for a set of buttons: equal sets (ids and labels, in order)
/// share one, so categories do not pile up with every notification.
fn category_identifier(actions: &[NotificationAction]) -> String {
    let mut hasher = DefaultHasher::new();
    let mut identifier = String::from("aimer.actions");
    for action in actions {
        action.label.hash(&mut hasher);
        identifier.push('.');
        identifier.push_str(&action.id.0.to_string());
    }
    format!("{identifier}.{:016x}", hasher.finish())
}

/// What a response to notification `id` means. `default` and `dismiss` are the
/// system's identifiers for opening and swiping away.
fn response_event(action: &str, id: u64, default: &str, dismiss: &str) -> Option<NotificationEvent> {
    let notification = NotificationId(id);
    if action == default {
        Some(NotificationEvent::Clicked(notification))
    } else if action == dismiss {
        Some(NotificationEvent::Dismissed(notification))
    } else {
        let action = parse_id(action).and_then(|value| u32::try_from(value).ok())?;
        Some(NotificationEvent::Action { notification, action: NotificationActionId(action) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT: &str = DEFAULT_ACTION;
    const DISMISS: &str = DISMISS_ACTION;

    #[test]
    fn action_identifiers_match_the_framework() {
        // SAFETY: both are constant `NSString`s exported by the framework, which
        // the test binary links.
        let (default, dismiss) = unsafe {
            (
                objc2_user_notifications::UNNotificationDefaultActionIdentifier.to_string(),
                objc2_user_notifications::UNNotificationDismissActionIdentifier.to_string(),
            )
        };
        assert_eq!(default, DEFAULT_ACTION);
        assert_eq!(dismiss, DISMISS_ACTION);
    }

    fn button(id: u32, label: &str) -> NotificationAction {
        NotificationAction::new(NotificationActionId(id), label)
    }

    #[test]
    fn parse_id_reads_plain_decimals_only() {
        assert_eq!(parse_id("0"), Some(0));
        assert_eq!(parse_id("42"), Some(42));
        assert_eq!(parse_id("18446744073709551615"), Some(u64::MAX));
        assert_eq!(parse_id("18446744073709551616"), None);
        assert_eq!(parse_id(""), None);
        assert_eq!(parse_id("+1"), None);
        assert_eq!(parse_id("-1"), None);
        assert_eq!(parse_id(" 1"), None);
        assert_eq!(parse_id("1a"), None);
    }

    #[test]
    fn response_maps_default_dismiss_and_actions() {
        assert_eq!(
            response_event(DEFAULT, 7, DEFAULT, DISMISS),
            Some(NotificationEvent::Clicked(NotificationId(7)))
        );
        assert_eq!(
            response_event(DISMISS, 7, DEFAULT, DISMISS),
            Some(NotificationEvent::Dismissed(NotificationId(7)))
        );
        assert_eq!(
            response_event("3", 7, DEFAULT, DISMISS),
            Some(NotificationEvent::Action {
                notification: NotificationId(7),
                action: NotificationActionId(3)
            })
        );
    }

    #[test]
    fn response_ignores_unknown_actions() {
        assert_eq!(response_event("reply", 7, DEFAULT, DISMISS), None);
        assert_eq!(response_event("4294967296", 7, DEFAULT, DISMISS), None);
        assert_eq!(response_event("", 7, DEFAULT, DISMISS), None);
    }

    #[test]
    fn equal_action_sets_share_a_category() {
        let a = category_identifier(&[button(1, "Snooze"), button(2, "Done")]);
        let b = category_identifier(&[button(1, "Snooze"), button(2, "Done")]);
        assert_eq!(a, b);
        assert!(a.starts_with("aimer.actions.1.2."));
    }

    #[test]
    fn different_action_sets_get_different_categories() {
        let base = category_identifier(&[button(1, "Snooze")]);
        assert_ne!(base, category_identifier(&[button(2, "Snooze")]));
        assert_ne!(base, category_identifier(&[button(1, "Later")]));
        assert_ne!(base, category_identifier(&[button(1, "Snooze"), button(2, "Done")]));
    }

    #[test]
    fn authorization_status_maps_to_permission() {
        assert_eq!(map_status(UNAuthorizationStatus::NotDetermined), Permission::NotDetermined);
        assert_eq!(map_status(UNAuthorizationStatus::Denied), Permission::Denied);
        assert_eq!(map_status(UNAuthorizationStatus::Authorized), Permission::Granted);
        assert_eq!(map_status(UNAuthorizationStatus::Provisional), Permission::Granted);
        assert_eq!(map_status(UNAuthorizationStatus::Ephemeral), Permission::Granted);
    }

    #[test]
    fn permission_callback_runs_once() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let done = Mutex::new(Some(Box::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
        }) as PermissionCallback));
        take_and_call(&done, Permission::Granted);
        take_and_call(&done, Permission::Denied);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn create_without_a_bundle_fails_instead_of_raising() {
        // A test binary has no bundle identifier; reaching the center would
        // raise an Objective-C exception and abort the process.
        let result = create(std::sync::Arc::new(|_| {}));
        assert!(matches!(result, Err(NotificationError::Platform(_))));
    }
}
