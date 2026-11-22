// Draw an app icon at the Dock's template size: artwork 824/1024 of a 1024 px canvas,
// centered, transparent margin. Art that already has a margin is only scaled to 1024.
//
//   swift scripts/pad-icon.swift <in.png> <out.png>
//
// bundle.sh runs it before making AppIcon.icns; the app does the same for the Dock
// (crates/crow-macos, set_app_icon), so one full-bleed PNG works everywhere.
import AppKit

let args = CommandLine.arguments
guard args.count == 3, let data = try? Data(contentsOf: URL(fileURLWithPath: args[1])),
      let rep = NSBitmapImageRep(data: data), let image = NSImage(data: data)
else {
    FileHandle.standardError.write(Data("usage: pad-icon.swift in.png out.png\n".utf8))
    exit(2)
}
let w = rep.pixelsWide, h = rep.pixelsHigh
func alpha(_ x: Int, _ y: Int) -> CGFloat { rep.colorAt(x: x, y: y)?.alphaComponent ?? 0 }
let dx = max(w / 50, 1), dy = max(h / 50, 1)  // a rounded icon is ~transparent exactly at the edge
let reachesEdge = [alpha(w / 2, dy), alpha(w / 2, h - 1 - dy), alpha(dx, h / 2), alpha(w - 1 - dx, h / 2)].contains { $0 > 0.1 }

let side: CGFloat = 1024
let art = reachesEdge ? side * 824 / 1024 : side
let inset = (side - art) / 2
let out = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: Int(side), pixelsHigh: Int(side), bitsPerSample: 8,
    samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
    bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
NSGraphicsContext.current?.imageInterpolation = .high
image.draw(in: NSRect(x: inset, y: inset, width: art, height: art), from: .zero, operation: .sourceOver, fraction: 1)
NSGraphicsContext.restoreGraphicsState()
try! out.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: args[2]))
