// Shared by the application target AND the widget extension target.
//
// The two processes agree on this type: the application starts and updates an
// activity with it, and the extension draws it. `ContentState` mirrors the JSON
// that `aimer_live_activity` encodes (`payload.rs`), field for field; a key the
// JSON has and this type lacks is ignored, so adding a field to the Rust side
// does not break an application whose Swift is older.

// ActivityKit is unavailable in the macOS SDK, so the whole file is iOS only
// (Mac Catalyst included); a macOS application can ship it and gets nothing.
#if os(iOS)
import ActivityKit
import Foundation

public struct AimerLiveActivityAttributes: ActivityAttributes {
    public struct ContentState: Codable, Hashable {
        /// How the bar is drawn. The raw values are what Rust writes.
        public enum Progress: String, Codable, Hashable {
            case none
            case indeterminate
            case fraction
        }

        /// A named point along the bar, `at` from 0 to 1.
        public struct Milestone: Codable, Hashable {
            public var label: String
            public var at: Double
        }

        public var title: String
        public var body: String
        /// A few words for the compact Dynamic Island.
        public var status: String
        /// An SF Symbol name; empty for the default.
        public var icon: String
        public var progress: Progress
        /// 0 to 1, meaningful when `progress` is `.fraction`.
        public var fraction: Double
        /// Unix epoch milliseconds of a countdown the system ticks by itself, so
        /// the clock moves while the application is suspended. Both or neither.
        public var timerStart: Int64?
        public var timerEnd: Int64?
        public var milestones: [Milestone]
        /// Free-form pairs for a custom view; see `LiveActivity::state` in Rust.
        public var state: [String: String]
        /// Views Rust described with `UIKitWidget`, by region name: `lockScreen`,
        /// `expandedLeading`, `expandedTrailing`, `expandedCenter`,
        /// `expandedBottom`, `compactLeading`, `compactTrailing` and `minimal`.
        /// Absent when the activity sets none.
        public var views: [String: Node]?
    }

    /// One node of a view tree that Rust describes (`ViewNode` in
    /// `view.rs`). `type` is `text`, `symbol`, `bar`, `clock`, `row`, `column`
    /// or `gap`; the other fields apply to some of them and are absent
    /// otherwise. An unknown `type` draws nothing, so a newer Rust side does not
    /// break an older extension.
    public struct Node: Codable, Hashable {
        public var type: String
        public var text: String?
        public var name: String?
        public var style: String?
        public var tint: String?
        public var digits: Bool?
        public var lines: Int?
        public var points: Double?
        public var fraction: Double?
        public var spacing: Double?
        public var children: [Node]?
    }

    /// The countdown as a date range, when there is one that is not empty.
    /// `Text(timerInterval:)` and `ProgressView(timerInterval:)` use it.
    public static func interval(_ state: ContentState) -> ClosedRange<Date>? {
        guard let start = state.timerStart, let end = state.timerEnd, end > start else { return nil }
        return Date(timeIntervalSince1970: Double(start) / 1000)...Date(timeIntervalSince1970: Double(end) / 1000)
    }

    /// The id the Rust side names the activity by (`LiveActivityId`).
    public var activityId: UInt64

    public init(activityId: UInt64) {
        self.activityId = activityId
    }
}
#endif
