// Exit 0 when the window screenshot's close button is red (the window was key), 1 when it
// is the grey of an inactive window. shot.sh retakes a shot that fails this.
import AppKit

guard CommandLine.arguments.count == 2,
      let data = try? Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])),
      let image = NSBitmapImageRep(data: data),
      let c = image.colorAt(x: 41, y: 52)?.usingColorSpace(.sRGB)
else { exit(2) }
exit(c.redComponent > 0.7 && c.greenComponent < 0.55 ? 0 : 1)
