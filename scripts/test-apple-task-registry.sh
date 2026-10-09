#!/bin/sh
# Verify the production task registry without loading Foundation Models or AI.
set -eu
registry_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
registry_temp=$(mktemp -d /tmp/selah-apple-task-registry.XXXXXX)
trap 'rm -rf "$registry_temp"' EXIT HUP INT TERM
python3 - "$registry_root" "$registry_temp" <<'PY'
import sys
from pathlib import Path
source = (Path(sys.argv[1]) / 'src-tauri/swift/AppleIntelligenceBridge.swift').read_text()
start = source.index('private final class TaskRegistry:')
end = source.index('private let registry = TaskRegistry()', start)
registry = source[start:end]
harness = r'''
@main
struct RegistryChecks {
    static func sleeper() -> Task<Void, Never> {
        Task { try? await Task.sleep(nanoseconds: 300_000_000_000) }
    }
    static func main() async {
        let registry = TaskRegistry()
        registry.cancel("before-register")
        let early = sleeper()
        registry.insert("before-register", early)
        precondition(early.isCancelled, "cancel before registration was lost")
        await early.value
        registry.remove("before-register")
        registry.clear("before-register")
        precondition(!registry.isCancelled("before-register"))

        let live = sleeper()
        registry.insert("registered", live)
        precondition(!live.isCancelled)
        registry.cancel("registered")
        precondition(live.isCancelled)
        await live.value
        registry.remove("registered")
        registry.clear("registered")

        let fresh = sleeper()
        registry.insert("registered", fresh)
        precondition(!fresh.isCancelled, "old cancellation reached a new request")
        registry.cancel("registered")
        await fresh.value
        registry.remove("registered")
        registry.clear("registered")

        for index in 0..<200 {
            let id = "race-\(index)"
            let task = sleeper()
            await withTaskGroup(of: Void.self) { group in
                group.addTask { registry.insert(id, task) }
                group.addTask { registry.cancel(id) }
                await group.waitForAll()
            }
            precondition(task.isCancelled, "concurrent cancel/register was lost")
            precondition(registry.isCancelled(id))
            await task.value
            registry.remove(id)
            registry.clear(id)
            precondition(!registry.isCancelled(id))
        }
        print("Apple task registry: cancel before/after registration, reuse and 200 concurrent races passed")
    }
}
'''
(Path(sys.argv[2]) / 'RegistryChecks.swift').write_text('import Foundation\n' + registry + harness)
PY
xcrun swiftc -parse-as-library -swift-version 6 -O "$registry_temp/RegistryChecks.swift" -o "$registry_temp/registry-checks"
"$registry_temp/registry-checks"
