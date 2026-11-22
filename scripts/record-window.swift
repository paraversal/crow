// Record one window to a movie, without the pointer or anything in front of it.
//
//   swift scripts/record-window.swift <window-id> <out.mov> [fps]
//
// ScreenCaptureKit captures the window alone (screencapture -v records the screen or a
// rect, pointer included). Stops on SIGINT or SIGTERM and finishes the file. Used by
// scripts/shot.sh when CROW_SHOT_RECORD is set. Needs Screen Recording permission.
import AppKit
import AVFoundation
import Foundation
import ScreenCaptureKit

let args = CommandLine.arguments
guard args.count >= 3, let windowID = UInt32(args[1]) else {
    FileHandle.standardError.write("usage: record-window.swift <window-id> <out.mov> [fps]\n".data(using: .utf8)!)
    exit(2)
}
let url = URL(fileURLWithPath: args[2])
let fps = args.count > 3 ? Int32(args[3]) ?? 60 : 60

final class Recorder: NSObject, SCRecordingOutputDelegate {
    let windowID: UInt32
    let url: URL
    let fps: Int32
    var stream: SCStream?
    let done = DispatchSemaphore(value: 0)

    init(windowID: UInt32, url: URL, fps: Int32) {
        self.windowID = windowID
        self.url = url
        self.fps = fps
    }

    func start() async throws {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let window = content.windows.first(where: { $0.windowID == self.windowID }) else {
            throw NSError(domain: "record-window", code: 1, userInfo: [NSLocalizedDescriptionKey: "no window \(windowID)"])
        }
        // The display cropped to the window (Crow is in front while recording). A window
        // filter makes macOS swap the traffic lights for its "being shared" badge.
        guard let display = content.displays.first(where: { $0.frame.contains(window.frame.origin) }) ?? content.displays.first
        else {
            throw NSError(domain: "record-window", code: 2, userInfo: [NSLocalizedDescriptionKey: "no display"])
        }
        let filter = SCContentFilter(display: display, excludingWindows: [])
        let config = SCStreamConfiguration()
        let scale = CGFloat(filter.pointPixelScale)
        config.sourceRect = window.frame.offsetBy(dx: -display.frame.minX, dy: -display.frame.minY)
        config.width = Int(window.frame.width * scale)
        config.height = Int(window.frame.height * scale)
        config.backgroundColor = .clear
        config.minimumFrameInterval = CMTime(value: 1, timescale: fps)
        config.showsCursor = false
        config.capturesAudio = false
        let output = SCRecordingOutputConfiguration()
        output.outputURL = url
        output.outputFileType = .mov
        output.videoCodecType = .h264
        let stream = SCStream(filter: filter, configuration: config, delegate: nil)
        try stream.addRecordingOutput(SCRecordingOutput(configuration: output, delegate: self))
        try await stream.startCapture()
        self.stream = stream
    }

    func stop() {
        guard let stream else { exit(0) }
        stream.stopCapture { _ in }
    }

    func recordingOutputDidFinishRecording(_ recordingOutput: SCRecordingOutput) { done.signal() }

    func recordingOutput(_ recordingOutput: SCRecordingOutput, didFailWithError error: Error) {
        FileHandle.standardError.write("record-window: \(error.localizedDescription)\n".data(using: .utf8)!)
        done.signal()
    }
}

let recorder = Recorder(windowID: windowID, url: url, fps: fps)
Task {
    do {
        try await recorder.start()
        print("recording")
        fflush(stdout)
    } catch {
        FileHandle.standardError.write("record-window: \(error.localizedDescription)\n".data(using: .utf8)!)
        exit(1)
    }
}
var sources: [DispatchSourceSignal] = []
for sig in [SIGINT, SIGTERM] {
    signal(sig, SIG_IGN)
    let source = DispatchSource.makeSignalSource(signal: sig, queue: .main)
    source.setEventHandler {
        recorder.stop()
        DispatchQueue.global().async {
            _ = recorder.done.wait(timeout: .now() + 5)
            exit(0)
        }
    }
    source.resume()
    sources.append(source)
}
// A real main thread with an app loop: ScreenCaptureKit adds its menu-bar indicator
// on the main queue, which dispatchMain() would hand to a worker thread.
let app = NSApplication.shared
app.setActivationPolicy(.prohibited)
app.run()
