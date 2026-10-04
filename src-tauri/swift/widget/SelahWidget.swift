import AppKit
import Darwin
import Foundation
import SwiftUI
import WidgetKit

private let snapshotName = "widget-snapshot.json"
private let appGroupID = "group.com.kgu.selah"

struct SelahClass: Codable, Identifiable {
    var day: Int
    var period: Int
    var name: String
    var room: String
    var startMinutes: Int
    var endMinutes: Int

    var id: String { "\(day)-\(period)-\(name)" }
}

struct SelahTodo: Codable, Identifiable {
    var title: String
    var course: String
    var dueUnix: Int64

    var id: String { "\(dueUnix)-\(title)" }
}

struct SelahSnapshot: Codable {
    var summary: String
    var classes: [SelahClass]
    var todos: [SelahTodo]

    static let empty = SelahSnapshot(
        summary: "Selah を開くと表示されます",
        classes: [],
        todos: []
    )
}

struct SelahEntry: TimelineEntry {
    var date: Date
    var snapshot: SelahSnapshot
}

enum SnapshotStore {
    static func load() -> SelahSnapshot {
        for url in candidateURLs() {
            guard let data = try? Data(contentsOf: url),
                  let snapshot = try? JSONDecoder().decode(SelahSnapshot.self, from: data)
            else { continue }
            return snapshot
        }
        return .empty
    }

    private static func candidateURLs() -> [URL] {
        var urls: [URL] = []
        var seen = Set<String>()
        func append(_ url: URL) {
            let path = url.path
            if seen.contains(path) { return }
            seen.insert(path)
            urls.append(url)
        }
        if let container = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: appGroupID) {
            append(container.appendingPathComponent(snapshotName))
        }
        let home = realHomeDirectory()
        append(home.appendingPathComponent("Library/Application Support/com.kgu.selah", isDirectory: true).appendingPathComponent(snapshotName))
        append(home.appendingPathComponent("Library/Group Containers/\(appGroupID)", isDirectory: true).appendingPathComponent(snapshotName))
        let sandboxHome = FileManager.default.homeDirectoryForCurrentUser
        append(sandboxHome.appendingPathComponent("Library/Application Support/com.kgu.selah", isDirectory: true).appendingPathComponent(snapshotName))
        return urls
    }
}

private func realHomeDirectory() -> URL {
    if let entry = getpwuid(getuid()), let dir = entry.pointee.pw_dir {
        let path = String(cString: dir)
        if path.hasPrefix("/") {
            return URL(fileURLWithPath: path, isDirectory: true)
        }
    }
    return FileManager.default.homeDirectoryForCurrentUser
}

struct SelahProvider: TimelineProvider {
    func placeholder(in context: Context) -> SelahEntry {
        SelahEntry(date: Date(), snapshot: SelahSnapshot(
            summary: "今日はあと2コマ",
            classes: [
                SelahClass(day: 6, period: 3, name: "情報科学", room: "B201", startMinutes: 13 * 60 + 30, endMinutes: 15 * 60),
                SelahClass(day: 6, period: 4, name: "英語 III", room: "A105", startMinutes: 15 * 60 + 10, endMinutes: 16 * 60 + 40)
            ],
            todos: [SelahTodo(title: "第7回レポートの下書きを提出する", course: "情報科学", dueUnix: Int64(Date().timeIntervalSince1970) + 86400)]
        ))
    }

    func getSnapshot(in context: Context, completion: @escaping (SelahEntry) -> Void) {
        completion(SelahEntry(date: Date(), snapshot: SnapshotStore.load()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<SelahEntry>) -> Void) {
        let now = Date()
        let snapshot = SnapshotStore.load()
        let next = nextRefresh(after: now, snapshot: snapshot)
        completion(Timeline(entries: [SelahEntry(date: now, snapshot: snapshot)], policy: .after(next)))
    }
}

private func nextRefresh(after now: Date, snapshot: SelahSnapshot) -> Date {
    let calendar = Calendar.current
    let minutes = calendar.component(.hour, from: now) * 60 + calendar.component(.minute, from: now)
    var nextDate = now.addingTimeInterval(15 * 60)
    let boundaries = snapshot.classes.flatMap { item in [item.startMinutes, item.endMinutes] }.filter { item in item > minutes }.sorted()
    if let next = boundaries.first {
        var components = calendar.dateComponents([.year, .month, .day], from: now)
        components.hour = next / 60
        components.minute = next % 60
        if let date = calendar.date(from: components), date > now {
            nextDate = date.addingTimeInterval(20)
        }
    }
    let nowUnix = Int64(now.timeIntervalSince1970)
    if let due = snapshot.todos.map(\.dueUnix).filter({ due in due >= nowUnix }).min() {
        let dueDate = Date(timeIntervalSince1970: TimeInterval(due)).addingTimeInterval(20)
        if dueDate > now && dueDate < nextDate {
            nextDate = dueDate
        }
    }
    return nextDate
}

struct SelahTodayWidget: Widget {
    var body: some WidgetConfiguration {
        StaticConfiguration(kind: "selah.today", provider: SelahProvider()) { entry in
            SelahWidgetView(entry: entry)
                .containerBackground(.background, for: .widget)
        }
        .configurationDisplayName("授業とTODO")
        .description("授業と締切を同時に表示します。")
        .supportedFamilies([.systemSmall, .systemMedium, .systemLarge])
        .contentMarginsDisabled()
    }
}

@main
struct SelahWidgetBundle: WidgetBundle {
    var body: some Widget {
        SelahTodayWidget()
    }
}

struct SelahWidgetView: View {
    var entry: SelahEntry
    var familyOverride: WidgetFamily? = nil
    @Environment(\.widgetFamily) private var environmentFamily
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.widgetRenderingMode) private var renderingMode

    private var family: WidgetFamily { familyOverride ?? environmentFamily }

    private var palette: WidgetPalette {
        WidgetPalette(scheme: colorScheme, renderingMode: renderingMode)
    }

    var body: some View {
        let today = weekdayNumber(entry.date)
        let now = currentMinutes(entry.date)
        let classes = classFeed(entry.snapshot.classes, today: today, now: now)
        let todos = upcomingTodos(entry.snapshot.todos, now: max(entry.date, Date()))
        let banner = contextBanner(firstDay: classes.first?.day, today: today)
        let metrics = WidgetMetrics.forFamily(family)
        let columns = family != .systemSmall

        ZStack(alignment: .bottomTrailing) {
            VStack(alignment: .leading, spacing: 0) {
                if columns {
                    SplitHeader(
                        date: entry.date,
                        banner: banner,
                        accentDay: classes.first?.day,
                        palette: palette,
                        dateSize: metrics.dateSize
                    )
                    .padding(.bottom, banner == nil ? metrics.afterHeader : metrics.afterBanner)

                    HStack(alignment: .top, spacing: 9) {
                        classColumn(classes, today: today, now: now, metrics: metrics)
                        Rectangle()
                            .fill(Color.primary.opacity(0.10))
                            .frame(width: 1)
                            .padding(.vertical, 1)
                            .padding(.bottom, metrics.logoClearance)
                        todoColumn(todos, metrics: metrics)
                            .padding(.bottom, metrics.logoClearance)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
                } else {
                    VStack(alignment: .leading, spacing: metrics.headerGap) {
                        DateHeader(date: entry.date, size: metrics.dateSize)
                        if let banner {
                            ContextCapsule(text: banner, palette: palette, compact: true, day: classes.first?.day)
                        }
                    }
                    .padding(.bottom, banner == nil ? metrics.afterHeader : metrics.afterBanner)

                    AgendaLayout(spacing: metrics.itemSpacing, sectionSpacing: metrics.sectionGap) {
                        if classes.isEmpty {
                            Text(entry.snapshot.classes.isEmpty ? entry.snapshot.summary : "授業なし")
                                .font(.system(size: 13))
                                .foregroundStyle(.secondary)
                                .lineLimit(2)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .agendaRole(.message)
                        }
                        ForEach(Array(classes.enumerated()), id: \.element.id) { index, slot in
                            ClassCard(
                                item: slot.item,
                                today: today,
                                now: now,
                                emphasized: index == 0,
                                compact: true,
                                palette: palette
                            )
                            .agendaRole(.event)
                        }
                        ForEach(Array(todos.enumerated()), id: \.element.id) { _, todo in
                            TodoRow(
                                todo: todo,
                                now: entry.date,
                                palette: palette,
                                dense: true,
                                trailingInset: metrics.logo + 10
                            )
                            .agendaRole(.todo)
                        }
                    }
                    .padding(.bottom, metrics.logoClearance)
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                }
            }
            .padding(.top, metrics.top)
            .padding(.horizontal, metrics.horizontal)
            .padding(.bottom, metrics.bottom)
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)

            logoMark(metrics)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func classColumn(_ classes: [ClassSlot], today: Int, now: Int, metrics: WidgetMetrics) -> some View {
        SectionedColumn(spacing: metrics.itemSpacing, sectionSpacing: metrics.sectionGap) {
            if classes.isEmpty {
                Text(entry.snapshot.classes.isEmpty ? entry.snapshot.summary : "授業なし")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .lineLimit(3)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .feedRole(.message)
            }
            ForEach(classes) { slot in
                if slot.showsDay {
                    HStack(spacing: 5) {
                        Circle()
                            .fill(palette.dayTint(slot.day))
                            .frame(width: 5, height: 5)
                        Text(weekdayLabel(slot.day) + "曜日")
                            .font(.system(size: 11, weight: .semibold))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .feedRole(.section)
                }
                ClassCard(
                    item: slot.item,
                    today: today,
                    now: now,
                    emphasized: false,
                    compact: metrics.compact,
                    palette: palette
                )
                .feedRole(.item)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func todoColumn(_ todos: [SelahTodo], metrics: WidgetMetrics) -> some View {
        FillColumn(spacing: metrics.itemSpacing) {
            if todos.isEmpty {
                Text("締切なし")
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            ForEach(todos) { todo in
                TodoRow(
                    todo: todo,
                    now: entry.date,
                    palette: palette,
                    dense: metrics.compact,
                    trailingInset: 0
                )
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
    }

    private func logoMark(_ metrics: WidgetMetrics) -> some View {
        WidgetLogo(side: metrics.logo)
            .padding(.trailing, metrics.logoTrailing)
            .padding(.bottom, metrics.logoGap)
    }
}

private struct ClassSlot: Identifiable {
    var item: SelahClass
    var day: Int
    var showsDay: Bool

    var id: String { item.id }
}

private struct WidgetMetrics {
    var top: CGFloat
    var horizontal: CGFloat
    var bottom: CGFloat
    var headerGap: CGFloat
    var afterHeader: CGFloat
    var afterBanner: CGFloat
    var itemSpacing: CGFloat
    var sectionGap: CGFloat
    var dateSize: CGFloat
    var logo: CGFloat
    var logoTrailing: CGFloat
    var logoGap: CGFloat
    var logoClearance: CGFloat
    var compact: Bool

    static func forFamily(_ family: WidgetFamily) -> WidgetMetrics {
        switch family {
        case .systemLarge:
            return WidgetMetrics(
                top: 14, horizontal: 16, bottom: 10,
                headerGap: 4, afterHeader: 8, afterBanner: 8,
                itemSpacing: 6, sectionGap: 9,
                dateSize: 15, logo: 30, logoTrailing: 14, logoGap: 10, logoClearance: 32,
                compact: false
            )
        case .systemSmall:
            return WidgetMetrics(
                top: 10, horizontal: 12, bottom: 8,
                headerGap: 3, afterHeader: 4, afterBanner: 4,
                itemSpacing: 3, sectionGap: 4,
                dateSize: 13, logo: 20, logoTrailing: 10, logoGap: 8, logoClearance: 16,
                compact: true
            )
        default:
            return WidgetMetrics(
                top: 8, horizontal: 14, bottom: 8,
                headerGap: 2, afterHeader: 6, afterBanner: 5,
                itemSpacing: 4, sectionGap: 6,
                dateSize: 13, logo: 24, logoTrailing: 14, logoGap: 8, logoClearance: 26,
                compact: true
            )
        }
    }
}

private struct WidgetPalette {
    var scheme: ColorScheme
    var renderingMode: WidgetRenderingMode

    var accented: Bool { renderingMode == .accented }

    func dayTint(_ day: Int) -> Color {
        if accented { return .primary }
        let light: Color
        let dark: Color
        switch day {
        case 1:
            light = Color(red: 0.05, green: 0.36, blue: 0.82)
            dark = Color(red: 0.47, green: 0.73, blue: 1.00)
        case 2:
            light = Color(red: 0.80, green: 0.36, blue: 0.04)
            dark = Color(red: 1.00, green: 0.64, blue: 0.28)
        case 3:
            light = Color(red: 0.06, green: 0.50, blue: 0.30)
            dark = Color(red: 0.40, green: 0.84, blue: 0.52)
        case 4:
            light = Color(red: 0.46, green: 0.26, blue: 0.76)
            dark = Color(red: 0.76, green: 0.58, blue: 0.98)
        case 5:
            light = Color(red: 0.00, green: 0.45, blue: 0.50)
            dark = Color(red: 0.32, green: 0.80, blue: 0.78)
        case 6:
            light = Color(red: 0.76, green: 0.22, blue: 0.40)
            dark = Color(red: 1.00, green: 0.52, blue: 0.64)
        default:
            light = Color(red: 0.62, green: 0.42, blue: 0.02)
            dark = Color(red: 0.98, green: 0.80, blue: 0.34)
        }
        return scheme == .dark ? dark : light
    }

    func dayWash(_ day: Int) -> Color {
        if accented { return Color.primary.opacity(0.12) }
        return dayTint(day).opacity(scheme == .dark ? 0.22 : 0.14)
    }

    func dueColor(daysLeft: Int) -> Color {
        if accented { return .primary }
        switch daysLeft {
        case ..<0:
            return scheme == .dark
                ? Color(red: 1.00, green: 0.40, blue: 0.36)
                : Color(red: 0.84, green: 0.16, blue: 0.14)
        case 0:
            return scheme == .dark
                ? Color(red: 1.00, green: 0.55, blue: 0.20)
                : Color(red: 0.88, green: 0.34, blue: 0.02)
        case 1...3:
            return scheme == .dark
                ? Color(red: 1.00, green: 0.76, blue: 0.28)
                : Color(red: 0.72, green: 0.46, blue: 0.00)
        case 4...7:
            return scheme == .dark
                ? Color(red: 0.45, green: 0.70, blue: 1.00)
                : Color(red: 0.10, green: 0.40, blue: 0.80)
        default:
            return Color.secondary
        }
    }
}

private struct DateHeader: View {
    var date: Date
    var size: CGFloat

    var body: some View {
        Text(appleDateTitle(date))
            .font(.system(size: size, weight: .semibold))
            .foregroundStyle(.secondary)
            .lineLimit(1)
            .minimumScaleFactor(0.8)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct SplitHeader: View {
    var date: Date
    var banner: String?
    var accentDay: Int?
    var palette: WidgetPalette
    var dateSize: CGFloat

    var body: some View {
        HStack(alignment: .center, spacing: 8) {
            Text(appleDateTitle(date))
                .font(.system(size: dateSize, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .minimumScaleFactor(0.7)
            if let banner {
                ContextCapsule(text: banner, palette: palette, compact: true, hugs: true, day: accentDay)
                    .layoutPriority(1)
            }
            Spacer(minLength: 8)
            Text("締切")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
    }
}

private struct ContextCapsule: View {
    var text: String
    var palette: WidgetPalette
    var compact: Bool
    var hugs: Bool = false
    var day: Int? = nil

    var body: some View {
        HStack(spacing: hugs ? 5 : 7) {
            Image(systemName: "calendar")
                .font(.system(size: compact ? 11 : 13, weight: .semibold))
                .foregroundStyle(iconTint)
            Text(text)
                .font(.system(size: compact ? 12 : 14, weight: .semibold))
                .foregroundStyle(Color.primary)
                .lineLimit(1)
            if !hugs {
                Spacer(minLength: 0)
            }
        }
        .padding(.leading, hugs ? 8 : 12)
        .padding(.trailing, hugs ? 10 : 14)
        .padding(.vertical, compact ? 3 : 6)
        .frame(maxWidth: hugs ? nil : .infinity, alignment: .leading)
        .background(fill, in: Capsule())
    }

    private var iconTint: Color {
        if let day { return palette.dayTint(day) }
        return palette.accented ? .primary : Color.secondary
    }

    private var fill: Color {
        if let day { return palette.dayWash(day) }
        return palette.accented ? Color.primary.opacity(0.12) : Color.primary.opacity(0.08)
    }
}

private struct ClassCard: View {
    var item: SelahClass
    var today: Int
    var now: Int
    var emphasized: Bool
    var compact: Bool
    var palette: WidgetPalette

    var body: some View {
        HStack(alignment: .center, spacing: compact ? 6 : 8) {
            Capsule()
                .fill(tint)
                .frame(width: emphasized ? 3.5 : 3, height: barHeight)
            VStack(alignment: .leading, spacing: compact ? -1 : 0) {
                Text(item.name)
                    .font(.system(size: titleSize, weight: .semibold))
                    .foregroundStyle(Color.primary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.62)
                HStack(spacing: 3) {
                    Text(timeRange(item))
                        .font(.system(size: timeSize))
                        .monospacedDigit()
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .fixedSize(horizontal: true, vertical: false)
                        .layoutPriority(1)
                    if !roomLabel.isEmpty {
                        Text("·")
                            .font(.system(size: max(9, timeSize - 1), weight: .semibold))
                            .foregroundStyle(.tertiary)
                            .fixedSize(horizontal: true, vertical: false)
                        Text(roomLabel)
                            .font(.system(size: timeSize))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                    }
                }
            }
            Spacer(minLength: 0)
        }
        .opacity(isPast ? 0.5 : 1)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(accessibilityText)
    }

    private var roomLabel: String { halfwidth(item.room) }

    private var accessibilityText: String {
        roomLabel.isEmpty
            ? item.name + " " + timeRange(item)
            : item.name + " " + timeRange(item) + " " + roomLabel
    }

    private var isToday: Bool { item.day == today }
    private var isPast: Bool { isToday && now >= item.endMinutes }
    private var tint: Color { palette.dayTint(item.day).opacity(isPast ? 0.85 : 1) }
    private var titleSize: CGFloat { compact ? 12 : (emphasized ? 15 : 13) }
    private var timeSize: CGFloat { compact ? 10 : 11 }
    private var barHeight: CGFloat { compact ? 20 : 26 }
}

private struct TodoRow: View {
    var todo: SelahTodo
    var now: Date
    var palette: WidgetPalette
    var dense: Bool = false
    var trailingInset: CGFloat = 0

    var body: some View {
        HStack(alignment: .top, spacing: 5) {
            DueProgressRing(progress: progress, color: mark, side: dense ? 12 : 14)
                .padding(.top, 1)
            Text(todo.title)
                .font(.system(size: dense ? 12 : 13))
                .foregroundStyle(titleColor)
                .lineLimit(2)
                .lineSpacing(dense ? -1 : 0)
                .multilineTextAlignment(.leading)
                .frame(maxWidth: .infinity, alignment: .topLeading)
            Text(label)
                .font(.system(size: dense ? 10 : 11, weight: .semibold))
                .foregroundStyle(mark)
                .lineLimit(1)
                .fixedSize(horizontal: true, vertical: false)
                .padding(.top, 1)
        }
        .padding(.trailing, trailingInset)
        .accessibilityElement(children: .combine)
    }

    private var mark: Color { palette.dueColor(daysLeft: daysLeft) }
    private var titleColor: Color { daysLeft <= 3 ? mark : Color.primary }
    private var progress: CGFloat {
        if daysLeft <= 0 { return 1 }
        let span = 7.0
        let remaining = min(Double(daysLeft), span)
        return CGFloat(max(0.25, 1 - remaining / span))
    }

    private var daysLeft: Int {
        let due = Date(timeIntervalSince1970: TimeInterval(todo.dueUnix))
        return Calendar.current.dateComponents(
            [.day],
            from: Calendar.current.startOfDay(for: now),
            to: Calendar.current.startOfDay(for: due)
        ).day ?? 0
    }

    private var label: String {
        if daysLeft < 0 { return "\(-daysLeft)日超過" }
        if daysLeft == 0 { return "本日" }
        return "\(daysLeft)日後"
    }
}

private struct DueProgressRing: View {
    var progress: CGFloat
    var color: Color
    var side: CGFloat

    private var lineWidth: CGFloat { max(1.7, side * 0.16) }

    var body: some View {
        let value = min(max(progress, 0), 1)
        ZStack {
            Circle()
                .stroke(color.opacity(0.16), style: StrokeStyle(lineWidth: lineWidth * 0.72, lineCap: .round))
            Circle()
                .trim(from: 0, to: value)
                .stroke(
                    color,
                    style: StrokeStyle(lineWidth: lineWidth, lineCap: value > 0.98 ? .butt : .round)
                )
                .rotationEffect(.degrees(-90))
        }
        .frame(width: side, height: side)
    }
}

private enum AgendaRole {
    case event
    case todo
    case message
}

private struct AgendaRoleKey: LayoutValueKey {
    static let defaultValue: AgendaRole = .event
}

private extension View {
    func agendaRole(_ role: AgendaRole) -> some View {
        layoutValue(key: AgendaRoleKey.self, value: role)
    }
}

private struct AgendaLayout: Layout {
    var spacing: CGFloat
    var sectionSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) -> CGSize {
        let width = proposal.width ?? 0
        if let limit = proposal.height, limit.isFinite {
            return CGSize(width: width, height: limit)
        }
        let used = arrange(width: width, limit: .greatestFiniteMagnitude, subviews: subviews).total
        return CGSize(width: width, height: used)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) {
        let plan = arrange(width: bounds.width, limit: bounds.height, subviews: subviews)
        let placed = Set(plan.items.map(\.index))
        for item in plan.items {
            subviews[item.index].place(
                at: CGPoint(x: bounds.minX, y: bounds.minY + item.y),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: bounds.width, height: item.height)
            )
        }
        for index in subviews.indices where !placed.contains(index) {
            hide(subviews[index], in: bounds)
        }
    }

    private struct Item {
        var index: Int
        var y: CGFloat
        var height: CGFloat
    }

    private struct Plan {
        var items: [Item]
        var total: CGFloat
    }

    private func arrange(width: CGFloat, limit: CGFloat, subviews: Subviews) -> Plan {
        let measured: [(index: Int, role: AgendaRole, height: CGFloat)] = subviews.indices.map { index in
            let size = subviews[index].sizeThatFits(ProposedViewSize(width: width, height: nil))
            return (index, subviews[index][AgendaRoleKey.self], size.height)
        }
        let events = measured.filter { $0.role == .event && $0.height > 0.5 }
        let todos = measured.filter { $0.role == .todo && $0.height > 0.5 }
        let messages = measured.filter { $0.role == .message && $0.height > 0.5 }
        var items: [Item] = []
        var y: CGFloat = 0

        func gap(before role: AgendaRole) -> CGFloat {
            guard let last = items.last else { return 0 }
            let previous = measured[last.index].role
            if previous != role && (previous == .event || role == .event) {
                return sectionSpacing
            }
            return spacing
        }

        func place(_ entry: (index: Int, role: AgendaRole, height: CGFloat)) -> Bool {
            let top = y + gap(before: entry.role)
            if top + entry.height > limit + 0.5 { return false }
            items.append(Item(index: entry.index, y: top, height: entry.height))
            y = top + entry.height
            return true
        }

        for message in messages {
            if !place(message) { break }
        }

        let reserve = (!events.isEmpty && !todos.isEmpty) ? todos[0].height + sectionSpacing : 0
        let classLimit = max(0, limit - reserve)
        if let hero = events.first, hero.height <= limit + 0.5 {
            _ = place(hero)
        }
        for event in events.dropFirst() {
            let top = y + gap(before: event.role)
            if top + event.height <= classLimit + 0.5 {
                _ = place(event)
            } else {
                break
            }
        }
        for todo in todos {
            if !place(todo) { break }
        }
        return Plan(items: items, total: y)
    }

    private func hide(_ subview: LayoutSubview, in bounds: CGRect) {
        subview.place(
            at: CGPoint(x: bounds.minX, y: bounds.maxY + 4000),
            anchor: .topLeading,
            proposal: ProposedViewSize(width: 0, height: 0)
        )
    }
}

private enum FeedRole {
    case item
    case section
    case message
}

private struct FeedRoleKey: LayoutValueKey {
    static let defaultValue: FeedRole = .item
}

private extension View {
    func feedRole(_ role: FeedRole) -> some View {
        layoutValue(key: FeedRoleKey.self, value: role)
    }
}

private struct SectionedColumn: Layout {
    var spacing: CGFloat
    var sectionSpacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) -> CGSize {
        let width = proposal.width ?? 0
        if let limit = proposal.height, limit.isFinite {
            return CGSize(width: width, height: limit)
        }
        return CGSize(width: width, height: arrange(width: width, limit: .greatestFiniteMagnitude, subviews: subviews).total)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) {
        let plan = arrange(width: bounds.width, limit: bounds.height, subviews: subviews)
        let placed = Set(plan.items.map(\.index))
        for item in plan.items {
            subviews[item.index].place(
                at: CGPoint(x: bounds.minX, y: bounds.minY + item.y),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: bounds.width, height: item.height)
            )
        }
        for index in subviews.indices where !placed.contains(index) {
            subviews[index].place(
                at: CGPoint(x: bounds.minX, y: bounds.maxY + 4000),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: 0, height: 0)
            )
        }
    }

    private struct Item {
        var index: Int
        var y: CGFloat
        var height: CGFloat
    }

    private struct Plan {
        var items: [Item]
        var total: CGFloat
    }

    private func arrange(width: CGFloat, limit: CGFloat, subviews: Subviews) -> Plan {
        let measured: [(index: Int, role: FeedRole, height: CGFloat)] = subviews.indices.map { index in
            let size = subviews[index].sizeThatFits(ProposedViewSize(width: width, height: nil))
            return (index, subviews[index][FeedRoleKey.self], size.height)
        }
        var items: [Item] = []
        var index = 0
        var y: CGFloat = 0
        while index < measured.count {
            let entry = measured[index]
            if entry.height <= 0.5 {
                index += 1
                continue
            }
            if entry.role == .section {
                let next = measured[(index + 1)...].first { $0.role == .item && $0.height > 0.5 }
                guard let next else { break }
                let sectionTop = y + gap(before: .section, measured: measured, items: items)
                let itemTop = sectionTop + entry.height + spacing
                if itemTop + next.height > limit + 0.5 { break }
                items.append(Item(index: entry.index, y: sectionTop, height: entry.height))
                items.append(Item(index: next.index, y: itemTop, height: next.height))
                y = itemTop + next.height
                index = next.index + 1
                continue
            }
            let top = y + gap(before: entry.role, measured: measured, items: items)
            if top + entry.height > limit + 0.5 { break }
            items.append(Item(index: entry.index, y: top, height: entry.height))
            y = top + entry.height
            index += 1
        }
        return Plan(items: items, total: y)
    }

    private func gap(before role: FeedRole, measured: [(index: Int, role: FeedRole, height: CGFloat)], items: [Item]) -> CGFloat {
        guard let last = items.last else { return 0 }
        let previous = measured[last.index].role
        if role == .section || previous == .section {
            return sectionSpacing
        }
        return spacing
    }
}

private struct FillColumn: Layout {
    var spacing: CGFloat = 6

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) -> CGSize {
        let width = proposal.width ?? 0
        if let limit = proposal.height, limit.isFinite {
            return CGSize(width: width, height: limit)
        }
        var height: CGFloat = 0
        var used = false
        for subview in subviews {
            let size = subview.sizeThatFits(ProposedViewSize(width: width, height: nil))
            if size.height <= 0.5 { continue }
            height += (used ? spacing : 0) + size.height
            used = true
        }
        return CGSize(width: width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Void) {
        var y = bounds.minY
        var used = false
        var stopped = false
        for subview in subviews {
            if stopped {
                hide(subview, in: bounds)
                continue
            }
            let size = subview.sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
            if size.height <= 0.5 {
                hide(subview, in: bounds)
                continue
            }
            let top = used ? y + spacing : y
            if top + size.height > bounds.maxY + 0.5 {
                hide(subview, in: bounds)
                stopped = true
                continue
            }
            subview.place(
                at: CGPoint(x: bounds.minX, y: top),
                anchor: .topLeading,
                proposal: ProposedViewSize(width: bounds.width, height: size.height)
            )
            y = top + size.height
            used = true
        }
    }

    private func hide(_ subview: LayoutSubview, in bounds: CGRect) {
        subview.place(
            at: CGPoint(x: bounds.minX, y: bounds.maxY + 4000),
            anchor: .topLeading,
            proposal: ProposedViewSize(width: 0, height: 0)
        )
    }
}

private struct WidgetLogo: View {
    var side: CGFloat

    var body: some View {
        if let image = Self.mark {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .antialiased(true)
                .scaledToFit()
                .frame(width: side, height: side)
                .accessibilityLabel("Selah")
        }
    }

    private static let mark: NSImage? = {
        guard let url = Bundle.main.url(forResource: "logo", withExtension: "png") else { return nil }
        return NSImage(contentsOf: url)
    }()
}

private func contextBanner(firstDay: Int?, today: Int) -> String? {
    guard let day = firstDay, day != today else { return nil }
    let label = weekdayLabel(day)
    guard !label.isEmpty else { return nil }
    return label + "曜日の授業"
}

private func appleDateTitle(_ date: Date) -> String {
    let formatter = DateFormatter()
    formatter.locale = Locale(identifier: "ja_JP")
    formatter.calendar = Calendar(identifier: .gregorian)
    formatter.dateFormat = "M月d日 EEEE"
    return formatter.string(from: date)
}

private func halfwidth(_ text: String) -> String {
    var result = String.UnicodeScalarView()
    result.reserveCapacity(text.unicodeScalars.count)
    for scalar in text.unicodeScalars {
        switch scalar.value {
        case 0xFF01...0xFF5E:
            if let narrow = UnicodeScalar(scalar.value - 0xFEE0) {
                result.append(narrow)
            }
        case 0x3000:
            result.append(" ")
        default:
            result.append(scalar)
        }
    }
    return String(result)
}

private func timeRange(_ item: SelahClass) -> String {
    clock(item.startMinutes) + "–" + clock(item.endMinutes)
}

private func classFeed(_ classes: [SelahClass], today: Int, now: Int) -> [ClassSlot] {
    guard !classes.isEmpty else { return [] }
    let start: Int
    if classes.contains(where: { $0.day == today }) {
        start = today
    } else if let next = nextSchoolDay(in: classes, after: today) {
        start = next
    } else {
        return []
    }
    var slots: [ClassSlot] = []
    for offset in 0..<7 {
        let day = ((start - 1 + offset) % 7) + 1
        let items = classes.filter { $0.day == day }.sorted { $0.period < $1.period }
        let visible: [SelahClass]
        if day == today {
            let remaining = items.filter { $0.endMinutes > now }
            visible = remaining.isEmpty ? items : remaining
        } else {
            visible = items
        }
        for (index, item) in visible.enumerated() {
            slots.append(ClassSlot(item: item, day: day, showsDay: index == 0 && day != start))
        }
    }
    return slots
}

private func upcomingTodos(_ todos: [SelahTodo], now: Date) -> [SelahTodo] {
    let cutoff = Int64(now.timeIntervalSince1970)
    return todos
        .filter { todo in todo.dueUnix >= cutoff }
        .sorted { lhs, rhs in lhs.dueUnix < rhs.dueUnix }
}

private func clock(_ minutes: Int) -> String {
    String(format: "%d:%02d", minutes / 60, minutes % 60)
}

private func currentMinutes(_ date: Date) -> Int {
    let calendar = Calendar.current
    return calendar.component(.hour, from: date) * 60 + calendar.component(.minute, from: date)
}

private func weekdayNumber(_ date: Date) -> Int {
    let weekday = Calendar.current.component(.weekday, from: date)
    return weekday == 1 ? 7 : weekday - 1
}

private func nextSchoolDay(in classes: [SelahClass], after today: Int) -> Int? {
    let days = Set(classes.map(\.day))
    for offset in 1...7 {
        let day = ((today - 1 + offset) % 7) + 1
        if days.contains(day) { return day }
    }
    return nil
}

private func weekdayLabel(_ day: Int) -> String {
    let labels = ["", "月", "火", "水", "木", "金", "土", "日"]
    guard day >= 0, day < labels.count else { return "" }
    return labels[day]
}
