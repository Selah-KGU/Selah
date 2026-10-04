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

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let watcher = Watcher()
watcher.start()
app.run()
