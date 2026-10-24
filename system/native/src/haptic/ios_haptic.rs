use super::{ImpactStyle, NotificationStyle};
use objc2::MainThreadMarker;
use objc2::MainThreadOnly;
use objc2_ui_kit::{
    UIImpactFeedbackGenerator, UIImpactFeedbackStyle, UINotificationFeedbackGenerator,
    UINotificationFeedbackType, UISelectionFeedbackGenerator,
};

impl From<ImpactStyle> for UIImpactFeedbackStyle {
    fn from(style: ImpactStyle) -> Self {
        match style {
            ImpactStyle::Light => Self::Light,
            ImpactStyle::Medium => Self::Medium,
            ImpactStyle::Heavy => Self::Heavy,
            ImpactStyle::Soft => Self::Soft,
            ImpactStyle::Rigid => Self::Rigid,
        }
    }
}

impl From<NotificationStyle> for UINotificationFeedbackType {
    fn from(style: NotificationStyle) -> Self {
        match style {
            NotificationStyle::Success => Self::Success,
            NotificationStyle::Warning => Self::Warning,
            NotificationStyle::Error => Self::Error,
        }
    }
}

pub(super) fn impact(style: ImpactStyle) {
    // UIFeedbackGenerator subclasses are UIKit objects: main-thread only.
    // If we're off the main thread (e.g. called from a background render
    // task), skip rather than crash.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // `initWithStyle:` is soft-deprecated in favour of
    // `feedbackGeneratorWithStyle:forView:`, which needs a `UIView` to
    // attach the interaction to. We deliberately have no view here — the
    // framework surfaces haptics as a free function — so the view-less
    // initialiser stays the correct call.
    #[allow(deprecated)]
    let generator = UIImpactFeedbackGenerator::initWithStyle(
        UIImpactFeedbackGenerator::alloc(mtm),
        style.into(),
    );
    // `prepare` primes the Taptic Engine so the actual trigger has
    // minimal latency. Cheap to call right before the trigger.
    generator.prepare();
    generator.impactOccurred();
}

pub(super) fn selection() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let generator = UISelectionFeedbackGenerator::new(mtm);
    generator.prepare();
    generator.selectionChanged();
}

pub(super) fn notification(style: NotificationStyle) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let generator = UINotificationFeedbackGenerator::new(mtm);
    generator.prepare();
    generator.notificationOccurred(style.into());
}

// ---- Programmable patterns (Core Haptics) --------------------------
//
// Patterns are built as a plain dictionary in Apple's AHAP schema (the
// same shape as an `.ahap` file) and handed to
// `CHHapticPattern initWithDictionary:error:`. The keys come from the
// `CHHapticPatternKey*` / `CHHapticEventType*` / `CHHapticEventParameterID*`
// constants rather than hand-written strings, so a typo is a link error
// instead of a silently ignored event.
//
// Unlike the UIKit generators above — where UIKit owns the object that
// does the work — Core Haptics hands ownership of both the engine and the
// pattern player to the caller, and playback is asynchronous. Everything
// therefore lives in a long-lived [`Session`]; see its docs for why.

use super::{HapticEvent, HapticPattern};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_core_haptics::{
    CHHapticDeviceCapability, CHHapticEngine, CHHapticEventParameterIDHapticIntensity,
    CHHapticEventParameterIDHapticSharpness, CHHapticEventType, CHHapticEventTypeHapticContinuous,
    CHHapticEventTypeHapticTransient, CHHapticPattern, CHHapticPatternKeyEvent,
    CHHapticPatternKeyEventDuration, CHHapticPatternKeyEventParameters,
    CHHapticPatternKeyEventType, CHHapticPatternKeyParameterID, CHHapticPatternKeyParameterValue,
    CHHapticPatternKeyPattern, CHHapticPatternKeyTime, CHHapticPatternKeyVersion,
    CHHapticPatternPlayer, CHHapticTimeImmediate,
};
use objc2_foundation::{NSError, NSMutableArray, NSMutableDictionary, NSNumber, NSString};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// AHAP dictionaries are heterogeneous (strings, numbers, arrays,
/// nested dictionaries), so the values stay untyped `AnyObject`s.
type AhapDict = NSMutableDictionary<NSString, AnyObject>;

/// `{ "ParameterID": id, "ParameterValue": value }`
fn parameter_dict(id: &NSString, value: f32) -> Retained<AhapDict> {
    let dict = AhapDict::new();
    dict.insert(unsafe { CHHapticPatternKeyParameterID }, id);
    dict.insert(
        unsafe { CHHapticPatternKeyParameterValue },
        &*NSNumber::new_f32(value),
    );
    dict
}

/// `{ "Event": { "EventType": .., "Time": .., "EventParameters": [..] } }`
fn event_dict(event: &HapticEvent) -> Retained<AhapDict> {
    let params: Retained<NSMutableArray<AhapDict>> = NSMutableArray::new();
    params.addObject(&parameter_dict(
        unsafe { CHHapticEventParameterIDHapticIntensity },
        event.intensity,
    ));
    params.addObject(&parameter_dict(
        unsafe { CHHapticEventParameterIDHapticSharpness },
        event.sharpness,
    ));

    let event_type: &CHHapticEventType = if event.duration.is_some() {
        unsafe { CHHapticEventTypeHapticContinuous }
    } else {
        unsafe { CHHapticEventTypeHapticTransient }
    };

    let inner = AhapDict::new();
    inner.insert(unsafe { CHHapticPatternKeyEventType }, event_type);
    inner.insert(
        unsafe { CHHapticPatternKeyTime },
        &*NSNumber::new_f64(event.time),
    );
    if let Some(duration) = event.duration {
        inner.insert(
            unsafe { CHHapticPatternKeyEventDuration },
            &*NSNumber::new_f64(duration),
        );
    }
    inner.insert(unsafe { CHHapticPatternKeyEventParameters }, &*params);

    let wrapper = AhapDict::new();
    wrapper.insert(unsafe { CHHapticPatternKeyEvent }, &*inner);
    wrapper
}

/// The whole AHAP document: `{ "Version": 1, "Pattern": [ .. ] }`
fn ahap_dict(pattern: &HapticPattern) -> Retained<AhapDict> {
    let events: Retained<NSMutableArray<AhapDict>> = NSMutableArray::new();
    for event in &pattern.events {
        events.addObject(&event_dict(event));
    }

    let root = AhapDict::new();
    root.insert(unsafe { CHHapticPatternKeyVersion }, &*NSNumber::new_i32(1));
    root.insert(unsafe { CHHapticPatternKeyPattern }, &*events);
    root
}

/// A pattern player, alive for as long as it still has something to play.
struct Playing {
    /// Never read: holding the player *is* its purpose, since dropping it
    /// cancels the pattern it is playing.
    #[allow(dead_code)]
    player: Retained<ProtocolObject<dyn CHHapticPatternPlayer>>,
    /// When the pattern is over and the player may be released.
    until: Instant,
}

/// The engine every [`play_pattern`] call goes through, plus the players
/// it has handed out and that are still running.
///
/// Both halves exist purely for lifetime reasons. Core Haptics playback
/// is asynchronous: `startAtTime:` only schedules the pattern and returns
/// immediately, and releasing the engine tears down the whole session
/// while releasing a player cancels its pattern. Dropping either at the
/// end of `play_pattern` therefore silences everything but, at best, the
/// leading transient — so the engine outlives all calls and each player
/// is held until its pattern has run out.
struct Session {
    engine: Retained<CHHapticEngine>,
    playing: Vec<Playing>,
}

thread_local! {
    /// One [`Session`] per thread, created on first use.
    ///
    /// `Retained` is neither `Send` nor `Sync`, so the engine cannot be a
    /// global; per-thread is the natural fit anyway, since haptics are
    /// triggered from the thread that handles input.
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// How long a finished player is kept around beyond its pattern's
/// duration.
///
/// The engine may delay a pattern slightly (it starts the hardware
/// lazily when auto shutdown kicked in), and `Instant` measures wall
/// clock rather than playback time, so releasing a player the very
/// moment its nominal duration elapsed risks cutting the tail off.
const RELEASE_GRACE: Duration = Duration::from_millis(250);

/// Report a Core Haptics failure.
///
/// The public API is infallible on purpose — a widget cannot do anything
/// useful about a missing Taptic Engine — but silently swallowing the
/// framework's `NSError` turns every mistake into "haptics just don't
/// work", which is close to impossible to diagnose from the outside. The
/// message goes to stderr, i.e. the Xcode console / device log.
fn report(what: &str, error: &NSError) {
    eprintln!("aimer_native::haptic: {what} failed: {error}");
}

impl Session {
    /// Create the engine, or `None` on hardware that cannot play
    /// haptics.
    fn new() -> Option<Self> {
        // SAFETY: plain message sends to the framework's own class and to
        // an object we just allocated and exclusively own.
        unsafe {
            if !CHHapticEngine::capabilitiesForHardware().supportsHaptics() {
                return None;
            }
            // `initAndReturnError:` is the designated initialiser, and
            // the *only* usable one: `-[CHHapticEngine init]` is declared
            // `NS_UNAVAILABLE` in `CHHapticEngine.h`, so going through it
            // yields an engine that never plays anything.
            let engine = match CHHapticEngine::initAndReturnError(CHHapticEngine::alloc()) {
                Ok(engine) => engine,
                Err(error) => {
                    report("CHHapticEngine init", &error);
                    return None;
                }
            };
            // Haptics-only playback keeps Core Haptics from claiming the
            // audio session, which would duck or interrupt whatever the
            // app (or another app) is playing.
            engine.setPlaysHapticsOnly(true);
            // Let the engine power the hardware down while idle; it is
            // brought back up by the explicit `start` in `play`.
            engine.setAutoShutdownEnabled(true);
            Some(Self {
                engine,
                playing: Vec::new(),
            })
        }
    }

    /// Schedule `pattern` on the engine.
    fn play(&mut self, pattern: &HapticPattern) {
        self.release_finished();

        // SAFETY: the dictionary follows the AHAP schema and the extern
        // statics are the framework's own key constants; every call is a
        // message send to an object we own.
        unsafe {
            // Idempotent while running, and the way back up after the
            // engine stopped on its own — auto shutdown, an audio session
            // interruption, or the app having been backgrounded.
            if let Err(error) = self.engine.startAndReturnError() {
                report("CHHapticEngine start", &error);
                return;
            }

            let dict = ahap_dict(pattern);
            let chpattern =
                match CHHapticPattern::initWithDictionary_error(CHHapticPattern::alloc(), &dict) {
                    Ok(chpattern) => chpattern,
                    Err(error) => {
                        report("CHHapticPattern init", &error);
                        return;
                    }
                };
            let player = match self.engine.createPlayerWithPattern_error(&chpattern) {
                Ok(player) => player,
                Err(error) => {
                    report("CHHapticEngine createPlayerWithPattern", &error);
                    return;
                }
            };
            if let Err(error) = player.startAtTime_error(CHHapticTimeImmediate) {
                report("CHHapticPatternPlayer start", &error);
                return;
            }

            self.playing.push(Playing {
                player,
                until: Instant::now() + Duration::from_secs_f64(pattern.duration()) + RELEASE_GRACE,
            });
        }
    }

    /// Drop the players whose patterns have run out.
    fn release_finished(&mut self) {
        let now = Instant::now();
        self.playing.retain(|playing| playing.until > now);
    }
}

pub(super) fn play_pattern(pattern: &HapticPattern) {
    // An empty pattern is rejected by Core Haptics; nothing to play.
    if pattern.events.is_empty() {
        return;
    }

    SESSION.with_borrow_mut(|session| {
        // A device without a Taptic Engine keeps the slot empty, so every
        // later call is just this failed initialisation again.
        let session = match session {
            Some(session) => session,
            None => match Session::new() {
                Some(created) => session.insert(created),
                None => return,
            },
        };
        session.play(pattern);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::haptic::Haptics;

    #[test]
    fn new_pattern_is_empty() {
        assert!(HapticPattern::new().events.is_empty());
    }

    #[test]
    fn transient_event_has_no_duration() {
        let pattern = HapticPattern::new().transient(0.0, 1.0, 0.5);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.0,
                duration: None,
                intensity: 1.0,
                sharpness: 0.5,
            }]
        );
    }

    #[test]
    fn continuous_event_carries_its_duration() {
        let pattern = HapticPattern::new().continuous(0.1, 0.4, 0.6, 0.2);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.1,
                duration: Some(0.4),
                intensity: 0.6,
                sharpness: 0.2,
            }]
        );
    }

    #[test]
    fn events_keep_insertion_order() {
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .continuous(0.1, 0.4, 0.6, 0.2)
            .transient(0.6, 0.3, 0.9);

        let times: Vec<f64> = pattern.events.iter().map(|event| event.time).collect();
        assert_eq!(times, vec![0.0, 0.1, 0.6]);
    }

    #[test]
    fn duration_spans_until_the_last_event_ends() {
        let pattern = HapticPattern::new()
            .transient(0.0, 1.0, 1.0)
            .continuous(0.1, 0.4, 0.6, 0.2);

        // The buzz starts at 0.1s and lasts 0.4s, so the pattern is 0.5s long.
        assert!((pattern.duration() - 0.5).abs() < 1e-9, "{pattern:?}");
    }

    #[test]
    fn duration_of_an_empty_pattern_is_zero() {
        assert_eq!(HapticPattern::new().duration(), 0.0);
    }

    #[test]
    fn duration_does_not_depend_on_insertion_order() {
        let late_first = HapticPattern::new()
            .transient(2.0, 1.0, 1.0)
            .continuous(0.0, 0.5, 1.0, 1.0);

        assert_eq!(late_first.duration(), 2.0);
    }

    #[test]
    fn out_of_range_parameters_are_clamped() {
        // Core Haptics rejects the *whole* pattern if a single parameter is
        // outside 0..=1, so the builder never lets one through.
        let pattern = HapticPattern::new().transient(-1.0, 4.0, -2.0);

        assert_eq!(
            pattern.events,
            vec![HapticEvent {
                time: 0.0,
                duration: None,
                intensity: 1.0,
                sharpness: 0.0,
            }]
        );
    }

    #[test]
    fn a_negative_duration_is_clamped_to_zero() {
        let pattern = HapticPattern::new().continuous(0.0, -1.0, 0.5, 0.5);

        assert_eq!(pattern.events[0].duration, Some(0.0));
    }

    #[test]
    fn the_engine_is_never_built_with_the_unavailable_initialiser() {
        // Regression guard: `-[CHHapticEngine init]` is `NS_UNAVAILABLE` in
        // `CHHapticEngine.h` — the designated initialiser is
        // `initAndReturnError:`. Going through plain `init` yields an engine
        // that silently plays nothing, which is invisible from the host (the
        // whole `ios` module is `cfg`'d out here) and, on device, looks like
        // "`play_pattern` does nothing while `impact` works". Checking the
        // source is the only way to catch it without a Taptic Engine.
        let source = include_str!("../haptic.rs");
        // Spelled in pieces so these needles don't match this test itself.
        let unavailable = concat!("CHHapticEngine::", "init(CHHapticEngine::alloc())");
        let designated = concat!(
            "CHHapticEngine::",
            "initAndReturnError(CHHapticEngine::alloc())"
        );

        assert!(
            !source.contains(unavailable),
            "CHHapticEngine must be created with initAndReturnError:"
        );
        assert!(
            source.contains(designated),
            "the designated initialiser call went missing"
        );
    }

    #[test]
    fn calls_never_panic() {
        // The point of the `cfg`-free API: widgets can call these on any
        // target without a panic or a `cfg` guard at the call site.
        Haptics::impact(ImpactStyle::Light);
        Haptics::selection();
        Haptics::notification(NotificationStyle::Success);
        Haptics::play_pattern(&HapticPattern::new().transient(0.0, 1.0, 1.0));
    }
}
