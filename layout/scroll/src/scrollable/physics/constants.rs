//! Named physics/feel constants for the scrollable container.
//!
//! These were previously scattered as magic numbers throughout
//! `controller.rs` and `handle_scroll.rs`. Hoisting them here makes the
//! scroll feel tunable and self-documenting.

/// Reference frame duration used to normalize velocity against a 120 Hz frame.
pub const FRAME_REF_120: f32 = 1.0 / 120.0;
/// Reference frame duration used to normalize velocity against a 60 Hz frame.
pub const FRAME_REF_60: f32 = 1.0 / 60.0;

/// Upper clamp for a single frame's delta time (seconds) during momentum.
/// Guards against huge jumps after the app was paused/backgrounded.
pub const MAX_FRAME_DT: f32 = 0.05;
/// Lower clamp for the delta time measured between scroll-wheel events.
pub const MIN_EVENT_DT: f32 = 0.005;
/// Lower clamp for the delta time measured between pointer-move events.
pub const MIN_MOVE_DT: f32 = 0.001;

/// Minimum wall-clock slice (seconds) that must elapse before a fresh drag
/// **velocity** sample is emitted from the accumulated finger delta.
///
/// On web, winit delivers one native `pointermove` as a burst of *coalesced*
/// samples dispatched in a single callback that all read (almost) the same
/// `Instant`. Computing velocity per sample then divides a small delta by a ~0
/// dt, so the value explodes and the release fling launches far too fast on
/// touch. Gating sampling on this slice makes same-frame coalesced samples
/// accumulate into one realistic velocity, while native (one sample per frame,
/// dt ≥ ~8 ms) keeps sampling every frame. Kept below a 120 Hz frame so it
/// never throttles native input.
pub const VELOCITY_SAMPLE_MIN_DT: f32 = FRAME_REF_120 * 0.5;

/// Velocity magnitude (px/frame) below which momentum is considered stopped.
pub const VELOCITY_EPSILON: f32 = 0.01;

/// Extra velocity damping applied when overshooting further out of bounds.
/// Lower values let the content coast further into the rubber-band zone.
pub const OOB_OVERSHOOT_DAMPING: f32 = 0.25;

/// Drag resistance multiplier applied at the very start of an overscroll.
///
/// Content moves at this fraction of finger speed just past the edge, and at a
/// steadily smaller fraction as the stretch grows, reaching zero at the
/// overscroll limit (see [`MAX_OVERSCROLL_VIEWPORT_FRACTION`]). A *constant*
/// factor would make the edge a fixed gear ratio instead of a rubber band: a
/// gesture the user keeps feeding — scrolling and holding at an edge — would
/// then creep outward without bound until the hard cap stopped it, which is
/// visible as the content parking half a screen away from its edge.
///
/// Lower = harder to pull past the boundary. `0.1` = 10% of finger speed.
pub const OOB_DRAG_RESISTANCE: f32 = 0.1;

/// Visual rubber-band coefficient for the display transform.
/// Controls how pronounced the rubber-band stretch appears on screen.
pub const RUBBER_BAND_VISUAL_COEFFICIENT: f32 = 1.5;

/// Per-60 Hz-frame velocity retention applied *on top of* normal friction
/// when the content is out-of-bounds during momentum. Lower values =
/// stronger braking in the overscroll zone, so the content decelerates
/// faster once it crosses the edge. Applied as
/// `overscroll_friction.powf(frame_ratio)`, same frame-rate-independent
/// model as the normal `friction` field.
///
/// `0.85` per 60 fps ≈ velocity halves in ~0.1 s of overscroll (aggressive).
pub const OVERSCROLL_FRICTION: f32 = 0.85;

/// Distance (px) under which spring-back snaps exactly to the clamped offset.
pub const SNAP_EPSILON: f32 = 0.5;

/// Minimum viewport extent (px) used to scale out-of-bounds resistance.
pub const MIN_VIEWPORT: f32 = 100.0;

/// Maximum fling velocity (px/frame at scale 1.0) from a scroll-wheel event.
pub const MAX_SCROLL_VELOCITY: f32 = 15000.0;

/// Gain applied to the peak drag velocity when a touch/mouse drag is released.
///
/// On release the peak finger velocity now seeds the SAME velocity +
/// exponential-friction momentum the trackpad/wheel `Scroll` path uses (see
/// `handle_scroll.rs`), so a touch flick decelerates exactly like a trackpad
/// flick. A trackpad already feeds a near-1:1 velocity into that model, so this
/// gain is kept close to 1.0 — just a light projection so a quick swipe gets a
/// satisfying push without overshooting the natural, trackpad-matched glide
/// that the shared `friction` decay then carries to rest.
pub const RELEASE_VELOCITY_GAIN: f32 = 1.5;

/// Control points of the cubic-bézier curve that shapes the release fling.
///
/// `cubic-bezier(0.17, 0.74, 0.30, 1.0)` — a smooth, gliding deceleration curve
/// (P0 = (0,0), P3 = (1,1) are implicit). The content position over the fling
/// is `start + distance · bezier(t / duration)`. Compared with the previous,
/// strongly front-loaded curve (which shot off fast then crawled — a "gliding
/// on mud" feel), this gentler initial slope launches the content without a
/// harsh jump and carries it through a long, even glide that eases cleanly to
/// rest, like sliding on ice.
pub const FLING_BEZIER_X1: f32 = 0.17;
pub const FLING_BEZIER_Y1: f32 = 0.74;
pub const FLING_BEZIER_X2: f32 = 0.30;
pub const FLING_BEZIER_Y2: f32 = 1.0;

/// Total duration (seconds) of a release fling shaped by the cubic-bézier
/// curve.
///
/// The glide eases from the release speed to a stop over this fixed window. The
/// coast distance is derived per axis from the release velocity so the curve's
/// initial slope matches the finger speed (no visible jump on lift-off) — a
/// longer duration therefore also glides farther. Tunable: raise it to slide
/// farther / settle slower, lower it for a snappier stop.
pub const FLING_DURATION_S: f32 = 2.0;

/// Normalized time fraction after which the bézier fling may snap to rest.
///
/// The curve reaches ~95% of its distance early (by `X2`), then crawls toward
/// the target over a long, barely-visible tail. Once past this fraction we let
/// the fling end as soon as the per-frame step becomes sub-pixel, so the glide
/// settles cleanly instead of creeping for seconds.
pub const FLING_TAIL_START: f32 = 0.5;
/// Per-frame step (px) below which, in the tail, the bézier fling snaps to
/// rest.
pub const FLING_END_STEP_PX: f32 = 0.5;

/// Maximum duration (seconds) for the velocity-based momentum after a touch
/// release.  Without this cap the exponential friction decay tails off
/// asymptotically.
pub const MAX_MOMENTUM_DURATION_S: f32 = 4.5;

/// Duration (seconds) before the hard cap during which extra friction is
/// applied to bleed off remaining velocity, so the glide fades to a natural
/// stop instead of hitting a wall.  Must be < MAX_MOMENTUM_DURATION_S.
pub const MOMENTUM_FADEOUT_S: f32 = MAX_MOMENTUM_DURATION_S - 0.05;

/// Size of the ring buffer of drag-velocity samples.
///
/// Sized so the buffer can hold a full [`VELOCITY_HORIZON_S`] window even at
/// the densest cadence sampling allows ([`VELOCITY_SAMPLE_MIN_DT`]); anything
/// older is outside the horizon and would be ignored anyway.
pub const VELOCITY_HISTORY_SIZE: usize = 24;

/// How far back (seconds) a drag-velocity sample may lie and still describe
/// the finger at the moment it left the glass.
///
/// A fling is launched with the speed of the *end* of a gesture, not its
/// average: a drag that races down the page and then eases to a standstill
/// before the lift must launch nothing at all. Everything recorded inside this
/// window is averaged by duration (travel ÷ time) and everything older is
/// dropped, so the release speed follows the finger's last moments and decays
/// to zero on its own once the finger stops feeding motion.
///
/// `0.1 s` matches the window Android's `VelocityTracker` and iOS use: long
/// enough to smooth out per-frame jitter, short enough that a deliberate stop
/// of a few frames is respected.
pub const VELOCITY_HORIZON_S: f32 = 0.1;

/// Drag activation threshold in device-independent pixels.
pub const DRAG_START_THRESHOLD_DP: f32 = 10.0;
/// Base blend weight for newly measured drag velocity.
pub const DRAG_BLEND_BASE: f32 = 0.4;
/// Time window (seconds) over which drag velocity blend ramps to full weight.
pub const DRAG_BLEND_WINDOW: f32 = 0.1;

/// Smoothing factor for direct scrollbar-thumb dragging (kept old / new).
pub const SCROLLBAR_DRAG_SMOOTH_OLD: f32 = 0.4;
pub const SCROLLBAR_DRAG_SMOOTH_NEW: f32 = 0.6;

/// Idle time (ms) after the last event past which residual velocity is cleared.
/// Must be generous enough for mobile platforms where touch delivery latency
/// (iOS gesture arbitration, ProMotion batching) can easily exceed 100ms
/// between the last PointerMove and the PointerUp that ends the gesture.
pub const VELOCITY_RESET_IDLE_MS: u128 = 400;

/// Time (ms) after which a lingering `active_touch_id` is considered stale and
/// cleared on the next PointerDown. This is a safety net for iOS where the app
/// can be backgrounded without receiving a Cancel/PointerUp, leaving the
/// scrollable stuck with a dead touch ID that rejects all new touches.
pub const STALE_TOUCH_THRESHOLD_MS: u128 = 1000;

/// Ease-out exponent for spring-back (1 − (1−t)^n).
/// Lower values = smoother, more gradual recovery.
pub const EASE_OUT_CUBIC: f32 = 1.5;

/// Keyboard scroll step (logical px per keypress).
pub const KEYBOARD_SCROLL_STEP: f32 = 40.0;
/// Keyboard page-scroll fraction of viewport (0.0–1.0).
pub const KEYBOARD_PAGE_FRACTION: f32 = 0.85;

/// Duration (ms) for scrollbar fade-in / fade-out transitions.
pub const SCROLLBAR_SHOW_DURATION_MS: u64 = 200;
pub const SCROLLBAR_HIDE_DURATION_MS: u64 = 400;

/// Spring-back stiffness (N/m equivalent). Controls how strongly the
/// overscrolled content is pulled toward the valid edge. Higher values
/// produce a snappier, more energetic bounce.
pub const SPRING_STIFFNESS: f32 = 2000.0;

/// Damping ratio (ζ) of the spring-back. ζ < 1 = underdamped (oscillates),
/// ζ = 1 = critically damped (no overshoot), ζ > 1 = overdamped (sluggish).
/// 1.0 = critically damped — the fastest return without any overshoot.
/// Content slides back to the boundary and stops, no shaking.
///
/// The ratio only describes what the user sees while the spring is advanced in
/// steps no longer than [`SPRING_SUBSTEP_S`]; see
/// [`spring`](crate::scrollable::spring) for why one step per rendered frame is
/// not enough on a 60 Hz host.
pub const SPRING_DAMPING_RATIO: f32 = 0.99;

/// Longest integration step (seconds) the overscroll spring may take at once.
///
/// An explicit step of the damped spring is stable only while
/// `dt < 1 / (2·ζ·√k)` ≈ 11 ms for [`SPRING_STIFFNESS`] /
/// [`SPRING_DAMPING_RATIO`]. A native ProMotion frame (8.3 ms) is inside that
/// limit; a browser frame (~16.7 ms, and the web target cannot render faster
/// than the compositor) is not, which made the same spring ping-pong on the
/// web while looking correct on native.
///
/// One 120 Hz frame is therefore the integration step everywhere: native keeps
/// taking exactly one step per frame (identical feel), and a longer frame is
/// split into as many steps as it covers, so both walk the same trajectory.
pub const SPRING_SUBSTEP_S: f32 = 1.0 / 120.0;

/// Maximum grace period (ms) after first crossing a scroll boundary before
/// bouncy recovery begins, even though no terminating
/// [`TouchPhase::Ended`](aimer_events::element::TouchPhase) frame arrived.
///
/// Later events do not refresh the grace period. This prevents a native
/// momentum tail from delaying recovery while still allowing the first edge
/// event to render without immediately fighting the spring.
pub const OVERSCROLL_HOLD_IDLE_MS: u128 = 4;

/// Maximum gap (ms) between device scroll events that still counts as the same
/// overscroll episode.
///
/// The overscroll high-water mark exists only to swallow a *live* momentum tail
/// that would otherwise re-stretch an edge that is already recovering. A tail
/// delivers an event every frame, so once nothing has refreshed the mark for
/// this long the gesture is over and the mark must stop gating input —
/// otherwise a gesture boundary the platform never reported would leave the
/// edge permanently unable to overscroll.
pub const OVERSCROLL_PEAK_IDLE_MS: u128 = 120;

/// Maximum overscroll as a fraction of the **viewport** dimension
/// (0.15 = 15%).
///
/// This is both the hard cap on the stored offset and the point where
/// [`OOB_DRAG_RESISTANCE`] has faded to zero, so a gesture asymptotically
/// approaches it instead of hitting a wall.
///
/// The limit is viewport-relative because the stretch is judged against what
/// the user can see: a fraction of the *content* let a long page stretch
/// hundreds of pixels while a short one barely moved. 15% of the viewport
/// matches the stretch Chrome and iOS allow.
pub const MAX_OVERSCROLL_VIEWPORT_FRACTION: f32 = 0.15;
