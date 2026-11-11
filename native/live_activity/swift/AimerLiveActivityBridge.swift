// Application target only.
//
// Exports the three functions `aimer_live_activity` finds with `dlsym`. Status
// codes (the same as `payload.rs`): 0 ok, 1 live activities turned off by the
// user, 2 undecodable payload, 3 ActivityKit refused, 4 system too old.
//
// Every `@_cdecl` below is `public` on purpose: `dlsym` only sees exported
// symbols, and a Release build (whole-module optimization) gives an `internal`
// function hidden visibility, so the Rust side would silently find nothing.
//
// Call these from the main thread, which is where an Aimer event loop runs and
// where ActivityKit wants to be started from. The application must be in the
// foreground to *start* an activity; updating and ending work in the background.

// ActivityKit is unavailable in the macOS SDK, so the whole file is iOS only
// (Mac Catalyst included); a macOS application can ship it and gets nothing.
#if os(iOS)
import ActivityKit
import Foundation

private let statusOk: Int32 = 0
private let statusDisabled: Int32 = 1
private let statusBadPayload: Int32 = 2
private let statusRefused: Int32 = 3
private let statusOldSystem: Int32 = 4

/// The key of the JSON, which is not part of the content state.
private struct ActivityKey: Decodable {
    let id: UInt64
}

@available(iOS 16.2, *)
private func running(_ id: UInt64) -> Activity<AimerLiveActivityAttributes>? {
    Activity<AimerLiveActivityAttributes>.activities.first { $0.attributes.activityId == id }
}

@_cdecl("aimer_live_activity_availability")
public func aimer_live_activity_availability() -> Int32 {
    guard #available(iOS 16.2, *) else { return statusOldSystem }
    return ActivityAuthorizationInfo().areActivitiesEnabled ? statusOk : statusDisabled
}

@_cdecl("aimer_live_activity_show")
public func aimer_live_activity_show(_ bytes: UnsafePointer<UInt8>?, _ length: Int) -> Int32 {
    guard #available(iOS 16.2, *) else { return statusOldSystem }
    guard let bytes, length > 0 else { return statusBadPayload }

    // `Data(bytes:count:)` copies, so Rust may free the buffer on return.
    let data = Data(bytes: bytes, count: length)
    let decoder = JSONDecoder()
    guard
        let key = try? decoder.decode(ActivityKey.self, from: data),
        let state = try? decoder.decode(AimerLiveActivityAttributes.ContentState.self, from: data)
    else { return statusBadPayload }

    guard ActivityAuthorizationInfo().areActivitiesEnabled else { return statusDisabled }

    let content = ActivityContent(state: state, staleDate: nil)
    if let activity = running(key.id) {
        Task { await activity.update(content) }
        return statusOk
    }
    do {
        _ = try Activity.request(
            attributes: AimerLiveActivityAttributes(activityId: key.id),
            content: content,
            pushType: nil
        )
        return statusOk
    } catch {
        return statusRefused
    }
}

@_cdecl("aimer_live_activity_end")
public func aimer_live_activity_end(_ id: UInt64) -> Int32 {
    // On a system without ActivityKit nothing can be running.
    guard #available(iOS 16.2, *) else { return statusOk }
    if let activity = running(id) {
        Task { await activity.end(nil, dismissalPolicy: .immediate) }
    }
    return statusOk
}
#endif
