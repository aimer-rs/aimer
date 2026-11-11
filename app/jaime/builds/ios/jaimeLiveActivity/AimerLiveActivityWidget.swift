// Widget extension target only.
//
// The bundle holds one `ActivityConfiguration` and no `Widget` of the usual
// kind, so the extension can only ever show a Live Activity: there is no home
// screen widget in the gallery, and nothing here can be placed inside an Aimer
// widget tree (an extension is a separate process that cannot host one).
//
// The template below draws the common fields of `LiveActivity`. Two ways to
// draw something of your own:
//   - Describe it in Rust with `UIKitWidget` and `LiveActivity::view(region, ..)`.
//     The tree arrives in `context.state.views` and `AimerNodeView` below draws
//     it; a region the app sets no view for keeps the template's content.
//   - Change the SwiftUI here and read `context.state.state`, the free-form pairs
//     set with `LiveActivity::state` in Rust.

// ActivityKit is unavailable in the macOS SDK, so the whole file is iOS only
// (Mac Catalyst included); a macOS application can ship it and gets nothing.
#if os(iOS)
import ActivityKit
import SwiftUI
import WidgetKit

@main
struct AimerLiveActivityBundle: WidgetBundle {
    var body: some Widget {
        AimerLiveActivityWidget()
    }
}

struct AimerLiveActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: AimerLiveActivityAttributes.self) { context in
            AimerRegion(context.state, "lockScreen") {
                AimerLockScreenView(state: context.state)
            }
            .padding()
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    AimerRegion(context.state, "expandedLeading") {
                        AimerIcon(state: context.state)
                    }
                    .aimerIsland()
                }
                DynamicIslandExpandedRegion(.trailing) {
                    AimerRegion(context.state, "expandedTrailing") {
                        AimerStatus(state: context.state, size: 12)
                            .font(.caption.monospacedDigit())
                    }
                    .aimerIsland()
                }
                DynamicIslandExpandedRegion(.center) {
                    AimerRegion(context.state, "expandedCenter") {
                        Text(context.state.title)
                            .font(.headline)
                            .lineLimit(1)
                    }
                    .aimerIsland()
                }
                DynamicIslandExpandedRegion(.bottom) {
                    AimerRegion(context.state, "expandedBottom") {
                        AimerProgressSection(state: context.state)
                    }
                    .aimerIsland()
                }
            } compactLeading: {
                AimerRegion(context.state, "compactLeading") {
                    AimerIcon(state: context.state)
                }
                .aimerIsland()
            } compactTrailing: {
                AimerRegion(context.state, "compactTrailing") {
                    AimerStatus(state: context.state, size: 11)
                        .font(.caption2.monospacedDigit())
                }
                .aimerIsland()
            } minimal: {
                AimerRegion(context.state, "minimal") {
                    AimerIcon(state: context.state)
                }
                .aimerIsland()
            }
        }
    }
}

extension View {
    /// The Dynamic Island is always black, but the environment of the extension
    /// follows the system appearance, so in light mode the default colours
    /// (`Color.primary`, the text and progress defaults) come out black on
    /// black and the region looks empty. Drawing the island dark keeps them
    /// readable.
    func aimerIsland() -> some View {
        self
            .environment(\.colorScheme, .dark)
            .foregroundStyle(.white)
    }
}

/// One slot of the activity: the view Rust described for `name`, or the
/// template's own content when it described none.
struct AimerRegion<Fallback: View>: View {
    let state: AimerLiveActivityAttributes.ContentState
    let name: String
    let fallback: Fallback

    init(
        _ state: AimerLiveActivityAttributes.ContentState,
        _ name: String,
        @ViewBuilder fallback: () -> Fallback
    ) {
        self.state = state
        self.name = name
        self.fallback = fallback()
    }

    var body: some View {
        if let node = state.views?[name] {
            AimerNodeView(node: node, state: state)
        } else {
            fallback
        }
    }
}

/// Draws a view tree described in Rust with `UIKitWidget`.
///
/// This is a plain interpreter: the tree is data, so no Rust code runs in the
/// extension. A node with a `type` this file does not know draws nothing.
struct AimerNodeView: View {
    let node: AimerLiveActivityAttributes.Node
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        switch node.type {
        case "text":
            Text(node.text ?? "")
                .font(AimerStyle.font(node.style, digits: node.digits ?? false))
                .foregroundStyle(AimerStyle.color(node.tint))
                .lineLimit(node.lines)
        case "symbol":
            Image(systemName: (node.name ?? "").isEmpty ? "bell.fill" : node.name ?? "")
                .font(node.points.map { Font.system(size: CGFloat($0)) })
                .foregroundStyle(AimerStyle.color(node.tint))
        case "bar":
            if let fraction = node.fraction {
                ProgressView(value: min(max(fraction, 0), 1))
                    .tint(AimerStyle.color(node.tint))
            } else {
                AimerBar(state: state)
                    .tint(AimerStyle.color(node.tint))
            }
        case "clock":
            AimerStatus(state: state, size: AimerStyle.points(node.style))
                .font(AimerStyle.font(node.style, digits: true))
                .foregroundStyle(AimerStyle.color(node.tint))
        case "row":
            HStack(spacing: node.spacing.map { CGFloat($0) }) {
                AimerChildren(node: node, state: state)
            }
        case "column":
            VStack(alignment: .leading, spacing: node.spacing.map { CGFloat($0) }) {
                AimerChildren(node: node, state: state)
            }
        case "gap":
            Spacer(minLength: 0)
        default:
            EmptyView()
        }
    }
}

struct AimerChildren: View {
    let node: AimerLiveActivityAttributes.Node
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        ForEach(Array((node.children ?? []).enumerated()), id: \.offset) { item in
            AimerNodeView(node: item.element, state: state)
        }
    }
}

/// The names Rust writes for `TextStyle` and `Tint`.
enum AimerStyle {
    static func font(_ style: String?, digits: Bool) -> Font {
        let base: Font
        switch style {
        case "title": base = .title2.bold()
        case "headline": base = .headline
        case "caption": base = .caption
        default: base = .body
        }
        return digits ? base.monospacedDigit() : base
    }

    /// The point size of the matching `font`, to size a clock by.
    static func points(_ style: String?) -> CGFloat {
        switch style {
        case "title": return 22
        case "caption": return 12
        default: return 17
        }
    }

    static func color(_ tint: String?) -> Color {
        switch tint {
        case "secondary": return .secondary
        case "accent": return .accentColor
        case "red": return .red
        case "orange": return .orange
        case "yellow": return .yellow
        case "green": return .green
        case "blue": return .blue
        case "gray": return .gray
        default: return .primary
        }
    }
}

/// The short status: a clock the system ticks by itself while the state has a
/// countdown, the plain `status` text otherwise.
///
/// The application cannot move the text of an activity while it is suspended in
/// the background, but `Text(timerInterval:)` is drawn by the system and keeps
/// counting regardless. It stops at 0:00 on its own at the deadline.
///
/// `Text(timerInterval:)` is greedy: it takes every point it is offered, and
/// forced to its ideal size (`fixedSize`) it asks for far more than the clock
/// needs, which overflows the row it is in and clips the whole region blank.
/// The width is therefore capped at what the longest reading of this countdown
/// needs at `size` points.
struct AimerStatus: View {
    let state: AimerLiveActivityAttributes.ContentState
    var size: CGFloat = 15

    /// The widest the clock gets: `M:SS` with as many minute digits as the
    /// whole countdown has. Hours are shown as minutes (`showsHours: false`).
    private var clockWidth: CGFloat {
        guard let start = state.timerStart, let end = state.timerEnd, end > start else { return size * 4 }
        let minutes = Int((end - start) / 60_000)
        let digits = String(max(minutes, 0)).count
        return CGFloat(digits + 3) * size * 0.65
    }

    var body: some View {
        if let interval = AimerLiveActivityAttributes.interval(state) {
            Text(timerInterval: interval, pauseTime: nil, countsDown: true, showsHours: false)
                .multilineTextAlignment(.trailing)
                .lineLimit(1)
                .frame(maxWidth: clockWidth, alignment: .trailing)
        } else {
            Text(state.status)
                .lineLimit(1)
                .minimumScaleFactor(0.6)
        }
    }
}

struct AimerIcon: View {
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        Image(systemName: state.icon.isEmpty ? "bell.fill" : state.icon)
    }
}

struct AimerLockScreenView: View {
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                AimerIcon(state: state)
                Text(state.title).font(.headline).lineLimit(1)
                Spacer()
                AimerStatus(state: state).font(.subheadline.monospacedDigit())
            }
            if !state.body.isEmpty {
                Text(state.body).font(.subheadline).foregroundStyle(.secondary)
            }
            AimerProgressSection(state: state)
        }
    }
}

/// The bar of the activity: filled by the system as time passes while there is
/// a countdown, else by the progress fraction.
struct AimerBar: View {
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        if let interval = AimerLiveActivityAttributes.interval(state) {
            ProgressView(timerInterval: interval, countsDown: false) {
                EmptyView()
            } currentValueLabel: {
                EmptyView()
            }
        } else {
            switch state.progress {
            case .none:
                EmptyView()
            case .indeterminate:
                ProgressView().progressViewStyle(.linear)
            case .fraction:
                ProgressView(value: min(max(state.fraction, 0), 1))
            }
        }
    }
}

struct AimerProgressSection: View {
    let state: AimerLiveActivityAttributes.ContentState

    var body: some View {
        switch state.progress {
        case .none:
            EmptyView()
        case .indeterminate:
            ProgressView().progressViewStyle(.linear)
        case .fraction:
            VStack(alignment: .leading, spacing: 4) {
                AimerBar(state: state)
                ForEach(state.milestones, id: \.self) { milestone in
                    HStack(spacing: 4) {
                        Image(systemName: milestone.at <= state.fraction ? "checkmark.circle.fill" : "circle")
                        Text(milestone.label).font(.caption)
                    }
                }
            }
        }
    }
}
#endif
