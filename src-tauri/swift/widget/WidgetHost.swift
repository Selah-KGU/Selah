import AppKit
import Foundation
import WidgetKit

private let snapshotURL: URL = {
    FileManager.default.homeDirectoryForCurrentUser
        .appendingPathComponent("Library/Application Support/com.kgu.selah", isDirectory: true)
        .appendingPathComponent("widget-snapshot.json")
}()

final class Watcher {
    private var source: DispatchSourceFileSystemObject?
    private var descriptor: Int32 = -1

    func start() {
        let directory = snapshotURL.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        descriptor = open(directory.path, O_EVTONLY)
        guard descriptor >= 0 else { return }
        let source = DispatchSource.makeFileSystemObjectSource(
            fileDescriptor: descriptor,
            eventMask: [.write, .rename, .delete],
            queue: .main
        )
        source.setEventHandler {
            WidgetCenter.shared.reloadAllTimelines()
        }
        source.setCancelHandler { [descriptor] in
            if descriptor >= 0 { close(descriptor) }
        }
        self.source = source
        source.resume()
        WidgetCenter.shared.reloadAllTimelines()
    }
}

private let widgetOpenNotification = Notification.Name("com.kgu.selah.widget-open")

final class HostDelegate: NSObject, NSApplicationDelegate {
    func application(_ application: NSApplication, open urls: [URL]) {
        if urls.contains(where: { $0.scheme?.caseInsensitiveCompare("selah") == .orderedSame }) {
            forwardWidgetClick()
        }
    }

    func application(
        _ application: NSApplication,
        willContinueUserActivityWithType userActivityType: String
    ) -> Bool {
        isWidgetActivityType(userActivityType)
    }

    func application(
        _ application: NSApplication,
        continue userActivity: NSUserActivity,
        restorationHandler: @escaping ([any NSUserActivityRestoring]) -> Void
    ) -> Bool {
        let info = userActivity.userInfo ?? [:]
        let fromWidget = isWidgetActivityType(userActivity.activityType)
            || info["WGWidgetUserInfoKeyKind"] != nil
            || info["WGWidgetUserInfoKeyFamily"] != nil
            || userActivity.webpageURL?.scheme?.caseInsensitiveCompare("selah") == .orderedSame
        if fromWidget {
            forwardWidgetClick()
            return true
        }
        return false
    }
}

private func isWidgetActivityType(_ activityType: String) -> Bool {
    let lower = activityType.lowercased()
    return lower.contains("widget") || lower.contains("selah")
}

private func forwardWidgetClick() {
    let running = NSWorkspace.shared.runningApplications.first { app in
        app.bundleIdentifier == "com.kgu.selah"
            || app.executableURL?.lastPathComponent == "selah-app"
    }
    if let running {
        NSApp.yieldActivation(to: running)
        DistributedNotificationCenter.default().postNotificationName(
            widgetOpenNotification,
            object: nil,
            userInfo: nil,
            deliverImmediately: true
        )
        running.activate(options: [.activateAllWindows])
        return
    }
    guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: "com.kgu.selah") else {
        return
    }
    let config = NSWorkspace.OpenConfiguration()
    config.activates = true
    NSWorkspace.shared.openApplication(at: url, configuration: config)
}

let app = NSApplication.shared
let hostDelegate = HostDelegate()
app.delegate = hostDelegate
app.setActivationPolicy(.accessory)
let watcher = Watcher()
watcher.start()
app.run()
