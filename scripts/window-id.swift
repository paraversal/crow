// Print Crow's window id and size: "<id> <width>x<height>" in points.
//
//   swift scripts/window-id.swift <pid>
//
// Finds the on-screen layer-0 window owned by the pid, which is the app's
// content window (not the menu bar or a panel). Used by scripts/shot.sh so a
// screenshot captures only Crow, at the size the boards use.
import CoreGraphics
import Foundation

let args = CommandLine.arguments
guard args.count == 2, let pid = Int(args[1]) else {
    FileHandle.standardError.write("usage: window-id.swift <pid>\n".data(using: .utf8)!)
    exit(2)
}
let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
guard let windows = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else {
    exit(1)
}
for window in windows {
    guard let owner = window[kCGWindowOwnerPID as String] as? Int, owner == pid,
          let layer = window[kCGWindowLayer as String] as? Int, layer == 0,
          let number = window[kCGWindowNumber as String] as? Int,
          let bounds = window[kCGWindowBounds as String] as? [String: Any],
          let width = bounds["Width"] as? Double, let height = bounds["Height"] as? Double
    else { continue }
    print("\(number) \(Int(width.rounded()))x\(Int(height.rounded()))")
    exit(0)
}
exit(1)
