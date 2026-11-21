// Prints the macOS system colours the Finder theme is based on, in sRGB hex, for the
// current appearance. Run it in Light and in Dark: `swift scripts/sample-system-colors.swift`.
import AppKit

func hex(_ c: NSColor) -> String {
    guard let s = c.usingColorSpace(.sRGB) else { return "?" }
    func b(_ v: CGFloat) -> Int { Int((v * 255).rounded()) }
    return String(format: "#%02x%02x%02x", b(s.redComponent), b(s.greenComponent), b(s.blueComponent))
}
let colors: [(String, NSColor)] = [
    ("windowBackground", .windowBackgroundColor), ("controlBackground", .controlBackgroundColor),
    ("underPageBackground", .underPageBackgroundColor), ("separator", .separatorColor),
    ("label", .labelColor), ("secondaryLabel", .secondaryLabelColor), ("tertiaryLabel", .tertiaryLabelColor),
    ("accent", .controlAccentColor), ("selectedContentBackground", .selectedContentBackgroundColor),
    ("unemphasizedSelection", .unemphasizedSelectedContentBackgroundColor),
    ("systemRed", .systemRed), ("systemOrange", .systemOrange), ("systemGreen", .systemGreen), ("systemBlue", .systemBlue),
]
print("appearance:", NSApp?.effectiveAppearance.name.rawValue ?? NSAppearance.currentDrawing().name.rawValue)
for (name, color) in colors { print(name.padding(toLength: 28, withPad: " ", startingAt: 0), hex(color)) }
