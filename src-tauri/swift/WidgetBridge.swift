import Foundation
import WidgetKit

@_cdecl("selah_widget_reload")
public func selah_widget_reload() {
    WidgetCenter.shared.reloadAllTimelines()
}
