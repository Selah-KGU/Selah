#!/bin/sh
# Execute the production Swift feed functions with synthetic weekly schedules.
# Does not install/reload a widget or touch the user's snapshot.
set -eu
widget_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
widget_temp=$(mktemp -d /tmp/selah-widget-class-feed.XXXXXX)
trap 'rm -rf "$widget_temp"' EXIT HUP INT TERM
python3 - "$widget_root" "$widget_temp" <<'PY'
import sys
from pathlib import Path
source = (Path(sys.argv[1]) / 'src-tauri/swift/widget/SelahWidget.swift').read_text()
def block(marker):
    start = source.index(marker)
    opening = source.index('{', start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == '{') - (source[end] == '}')
        end += 1
    return source[start:end]
parts = [block(marker) for marker in [
    'struct SelahClass:', 'private struct ClassSlot:',
    'private func classFeed(', 'private func nextSchoolDay(',
    'private func contextBanner(', 'private func weekdayLabel(',
]]
harness = r'''
@main
struct WidgetFeedChecks {
    static func main() {
        var checks = 0
        func require(_ ok: Bool, _ message: String) {
            if !ok { print("FAIL: \(message)"); exit(1) }
            checks += 1
        }
        func course(_ day: Int, _ period: Int, _ start: Int, _ end: Int) -> SelahClass {
            SelahClass(day: day, period: period, name: "曜日\(day)・\(period)限", room: "B201", startMinutes: start, endMinutes: end)
        }
        let friday = [course(5, 1, 540, 630), course(5, 2, 640, 730)]
        let monday = course(1, 3, 780, 870)
        require(classFeed(friday, today: 5, now: 730).isEmpty, "the final class must disappear exactly at its end; never restore today's whole schedule")
        require(classFeed(friday, today: 5, now: 1439).isEmpty, "completed classes must stay absent for the rest of today")
        require(classFeed([], today: 5, now: 730).isEmpty, "empty timetable remains empty")
        require(classFeed(friday, today: 5, now: 729).map(\.item.period) == [2], "ongoing final class remains until its end")
        require(classFeed(friday, today: 5, now: 630).map(\.item.period) == [2], "completed first class leaves during the break")
        let afterFriday = classFeed([monday] + friday, today: 5, now: 730)
        require(afterFriday.map(\.day) == [1], "Friday evening advances across the weekend to Monday")
        require(afterFriday.first?.showsDay == false, "first future day uses the banner rather than a duplicate section header")
        require(contextBanner(firstDay: afterFriday.first?.day, today: 5) == "月曜日の授業", "banner advances with the actual first remaining day")
        require(contextBanner(firstDay: nil, today: 5) == nil, "no stale banner after all classes leave")
        let week = (1...7).flatMap { day in [course(day, 1, 540, 630), course(day, 2, 640, 730), course(day, 3, 780, 870)] }
        for today in 1...7 {
            for minute in 0..<1440 {
                let feed = classFeed(Array(week.reversed()), today: today, now: minute)
                require(feed.allSatisfy { $0.day != today || $0.item.endMinutes > minute }, "ended class resurfaced: day \(today), minute \(minute)")
                require(Set(feed.map(\.id)).count == feed.count, "duplicate class after weekly wrap")
                let expectedToday = week.filter { $0.day == today && $0.endMinutes > minute }.map(\.period)
                require(feed.filter { $0.day == today }.map(\.item.period) == expectedToday, "remaining today classes missing or out of order")
                require(feed.filter { $0.day != today }.count == 18, "future weekdays unexpectedly lost classes")
                let expectedFirstDay = expectedToday.isEmpty ? today % 7 + 1 : today
                require(feed.first?.day == expectedFirstDay, "feed must start at the next visible teaching day")
                for (index, slot) in feed.enumerated() {
                    let startsGroup = index > 0 && feed[index - 1].day != slot.day
                    require(slot.showsDay == startsGroup, "missing or duplicate day section")
                    if index > 0 && feed[index - 1].day == slot.day {
                        require(feed[index - 1].item.period < slot.item.period, "periods must remain sorted within weekdays")
                    }
                }
            }
        }
        require(classFeed(friday + [monday], today: 6, now: 1400).map(\.day) == [1, 5, 5], "a day without lessons still starts at the next school day")
        let saturday = course(6, 1, 540, 630)
        let laterDays = classFeed(friday + [saturday, monday], today: 5, now: 730)
        require(laterDays.map(\.day) == [6, 1], "completed today must not return at the end of a future-days feed")
        print("Widget class feed: \(checks) checks passed across all 7 weekdays and 1,440 minute boundaries")
    }
}
'''
(Path(sys.argv[2]) / 'WidgetFeedChecks.swift').write_text('import Foundation\n' + '\n\n'.join(parts) + '\n' + harness)
PY
xcrun swiftc -parse-as-library -swift-version 6 -O "$widget_temp/WidgetFeedChecks.swift" -o "$widget_temp/widget-feed-checks"
"$widget_temp/widget-feed-checks"
