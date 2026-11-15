// Render Crow's app icons from their SVG sources to 1024 px PNGs, using AppKit's own
// SVG support (macOS 13+), so no extra tools are needed.
//
//   swift scripts/render-app-icons.swift
//
// Reads crates/crow-app/assets/app-icon/*.svg and writes a .png next to each. The PNGs
// are committed: the app embeds them (Dock icon) and bundle.sh turns the default one
// into AppIcon.icns. Run this again after editing an SVG.
import AppKit

let dir = URL(fileURLWithPath: CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : "crates/crow-app/assets/app-icon")
let size = 1024

let svgs = try FileManager.default.contentsOfDirectory(at: dir, includingPropertiesForKeys: nil)
    .filter { $0.pathExtension == "svg" }
    .sorted { $0.lastPathComponent < $1.lastPathComponent }
guard !svgs.isEmpty else {
    FileHandle.standardError.write("no .svg files in \(dir.path)\n".data(using: .utf8)!)
    exit(1)
}

for svg in svgs {
    guard let image = NSImage(contentsOf: svg) else {
        FileHandle.standardError.write("can't read \(svg.path)\n".data(using: .utf8)!)
        exit(1)
    }
    let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size, bitsPerSample: 8,
        samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
        bytesPerRow: 0, bitsPerPixel: 0)!
    rep.size = NSSize(width: size, height: size)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current?.imageInterpolation = .high
    image.draw(in: NSRect(x: 0, y: 0, width: size, height: size))
    NSGraphicsContext.restoreGraphicsState()
    let png = svg.deletingPathExtension().appendingPathExtension("png")
    try rep.representation(using: .png, properties: [:])!.write(to: png)
    print("wrote \(png.path)")
}
