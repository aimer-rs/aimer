//! The JSON the Apple bridge receives.
//!
//! The Swift side decodes it with `JSONDecoder` into the content state of the
//! activity. Writing it by hand keeps a serialization crate out of the
//! dependency tree for one flat document.

use std::fmt::Write;

use crate::{LiveActivity, LiveActivityError, Progress};

/// Encodes `activity` as one JSON object.
///
/// ```text
/// {"id":7,"title":"..","body":"..","status":"..","icon":"..",
///  "progress":"fraction","fraction":0.6,
///  "milestones":[{"label":"..","at":0.5}],"state":{"k":"v"}}
/// ```
///
/// `progress` is one of `"none"`, `"indeterminate"` and `"fraction"`;
/// `fraction` is `0` unless it is `"fraction"`. Numbers are always finite.
///
/// A countdown adds `"timerStart":<ms>,"timerEnd":<ms>` (Unix epoch
/// milliseconds) after `fraction`; without one the keys are absent.
///
/// Views add `"views":{"lockScreen":{..},..}` after `state`, one view tree
/// per region (see `ViewNode::write_json`); without any the key is absent.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios", test)), allow(dead_code))]
pub(crate) fn encode(activity: &LiveActivity) -> String {
    let mut out = String::with_capacity(160);
    let _ = write!(out, "{{\"id\":{}", activity.id.0);
    for (key, value) in [
        ("title", &activity.title),
        ("body", &activity.body),
        ("status", &activity.status),
        ("icon", &activity.icon),
    ] {
        let _ = write!(out, ",\"{key}\":");
        push_string(&mut out, value);
    }
    let kind = match activity.progress {
        Progress::None => "none",
        Progress::Indeterminate => "indeterminate",
        Progress::Fraction(_) => "fraction",
    };
    let _ = write!(
        out,
        ",\"progress\":\"{kind}\",\"fraction\":{}",
        activity.progress.fraction().unwrap_or(0.0)
    );
    if let Some(countdown) = activity.countdown {
        let _ = write!(
            out,
            ",\"timerStart\":{},\"timerEnd\":{}",
            countdown.start_ms, countdown.end_ms
        );
    }
    out.push_str(",\"milestones\":[");
    for (index, milestone) in activity.milestones.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"label\":");
        push_string(&mut out, &milestone.label);
        let _ = write!(out, ",\"at\":{}}}", milestone.at);
    }
    out.push_str("],\"state\":{");
    for (index, (key, value)) in activity.state.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        push_string(&mut out, key);
        out.push(':');
        push_string(&mut out, value);
    }
    out.push('}');
    if !activity.views.is_empty() {
        out.push_str(",\"views\":{");
        for (index, (region, node)) in activity.views.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{}\":", region.key());
            node.write_json(&mut out);
        }
        out.push('}');
    }
    out.push('}');
    out
}

/// What the Swift bridge reports: success.
pub(crate) const STATUS_OK: i32 = 0;
/// The user turned live activities off for the application.
pub(crate) const STATUS_DISABLED: i32 = 1;
/// The bridge could not decode the JSON it was given.
pub(crate) const STATUS_BAD_PAYLOAD: i32 = 2;
/// ActivityKit refused to start or update the activity.
pub(crate) const STATUS_REFUSED: i32 = 3;
/// The operating system predates ActivityKit.
pub(crate) const STATUS_OLD_SYSTEM: i32 = 4;

/// Turns a status code of the Swift bridge into a result.
#[cfg_attr(not(any(target_os = "macos", target_os = "ios", test)), allow(dead_code))]
pub(crate) fn bridge_status(code: i32) -> Result<(), LiveActivityError> {
    match code {
        STATUS_OK => Ok(()),
        STATUS_DISABLED => Err(LiveActivityError::Disabled),
        STATUS_BAD_PAYLOAD => Err(LiveActivityError::Platform("the bridge could not decode the activity".into())),
        STATUS_REFUSED => Err(LiveActivityError::Platform("ActivityKit refused the request".into())),
        STATUS_OLD_SYSTEM => Err(LiveActivityError::Unsupported),
        other => Err(LiveActivityError::Platform(format!("the bridge answered with status {other}"))),
    }
}

/// Appends `text` as a JSON string literal.
pub(crate) fn push_string(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LiveActivityId, Milestone};

    #[test]
    fn bridge_status_maps_every_code() {
        assert_eq!(bridge_status(STATUS_OK), Ok(()));
        assert_eq!(bridge_status(STATUS_DISABLED), Err(LiveActivityError::Disabled));
        assert_eq!(bridge_status(STATUS_OLD_SYSTEM), Err(LiveActivityError::Unsupported));
        assert!(matches!(bridge_status(STATUS_BAD_PAYLOAD), Err(LiveActivityError::Platform(_))));
        assert!(matches!(bridge_status(STATUS_REFUSED), Err(LiveActivityError::Platform(_))));
        assert!(matches!(bridge_status(99), Err(LiveActivityError::Platform(text)) if text.contains("99")));
    }

    #[test]
    fn minimal_activity() {
        let json = encode(&LiveActivity::new(LiveActivityId(7)).title("Hi"));
        assert_eq!(
            json,
            "{\"id\":7,\"title\":\"Hi\",\"body\":\"\",\"status\":\"\",\"icon\":\"\",\
             \"progress\":\"none\",\"fraction\":0,\"milestones\":[],\"state\":{}}"
        );
    }

    #[test]
    fn full_activity() {
        let activity = LiveActivity::new(LiveActivityId(u64::MAX))
            .title("Ride")
            .body("Near")
            .status("4 min")
            .icon("car.fill")
            .progress(0.5)
            .milestone(Milestone::new("Picked up", 0.25))
            .milestone(Milestone::new("Arrived", 1.0))
            .state("driver", "Ada")
            .state("plate", "X1");
        assert_eq!(
            encode(&activity),
            "{\"id\":18446744073709551615,\"title\":\"Ride\",\"body\":\"Near\",\
             \"status\":\"4 min\",\"icon\":\"car.fill\",\"progress\":\"fraction\",\
             \"fraction\":0.5,\"milestones\":[{\"label\":\"Picked up\",\"at\":0.25},\
             {\"label\":\"Arrived\",\"at\":1}],\"state\":{\"driver\":\"Ada\",\"plate\":\"X1\"}}"
        );
    }

    #[test]
    fn countdown_is_written_as_epoch_milliseconds() {
        use std::time::{Duration, UNIX_EPOCH};
        let ends = UNIX_EPOCH + Duration::from_millis(1_700_000_060_000);
        let json = encode(
            &LiveActivity::new(LiveActivityId(1))
                .title("x")
                .countdown(ends, Duration::from_secs(60)),
        );
        assert!(
            json.contains(
                "\"fraction\":0,\"timerStart\":1700000000000,\"timerEnd\":1700000060000,\"milestones\""
            ),
            "{json}"
        );
    }

    #[test]
    fn views_are_written_per_region_after_the_state() {
        use crate::{Clock, Region, Symbol};
        let json = encode(
            &LiveActivity::new(LiveActivityId(1))
                .title("x")
                .state("k", "v")
                .view(Region::CompactLeading, Symbol::new("timer"))
                .view(Region::CompactTrailing, Clock::new()),
        );
        assert!(
            json.ends_with(
                "\"state\":{\"k\":\"v\"},\"views\":{\"compactLeading\":\
                 {\"type\":\"symbol\",\"name\":\"timer\",\"tint\":\"primary\"},\
                 \"compactTrailing\":{\"type\":\"clock\",\"style\":\"body\",\"tint\":\"primary\"}}}"
            ),
            "{json}"
        );
    }

    #[test]
    fn indeterminate_has_zero_fraction() {
        let json = encode(&LiveActivity::new(LiveActivityId(1)).title("x").indeterminate());
        assert!(json.contains("\"progress\":\"indeterminate\",\"fraction\":0,"));
    }

    #[test]
    fn out_of_range_fraction_is_clamped_and_finite() {
        let json = encode(&LiveActivity::new(LiveActivityId(1)).title("x").progress(f32::NAN));
        assert!(json.contains("\"fraction\":0,"));
        let json = encode(&LiveActivity::new(LiveActivityId(1)).title("x").progress(9.0));
        assert!(json.contains("\"fraction\":1,"));
    }

    #[test]
    fn strings_are_escaped() {
        let json = encode(&LiveActivity::new(LiveActivityId(1)).title("a\"b\\c\nd\te\u{1}f \u{e9}\u{1f600}"));
        assert!(json.contains("\"title\":\"a\\\"b\\\\c\\nd\\te\\u0001f \u{e9}\u{1f600}\""));
    }
}
