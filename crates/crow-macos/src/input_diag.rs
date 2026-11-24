//! Diagnostics for keyboard focus. A shortcut that never fires leaves no trace on its
//! own, so this records what AppKit saw: the event it handed Crow, which process posted
//! it, the mouse buttons, and which app and window were key. Lines go to crow.log under
//! the `input` target. The titlebar drag watcher in `titlebar_drag` reads the same snapshot.

use std::fmt;

use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSEvent, NSEventType, NSRunningApplication, NSWindow, NSWorkspace};
use objc2_foundation::NSProcessInfo;

use crate::main_thread;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventGetIntegerValueField(event: *const CGEventRef, field: u32) -> i64;
}

/// The opaque `CGEventRef` behind an `NSEvent`, encoded as AppKit declares it.
#[repr(C)]
struct CGEventRef {
    _private: [u8; 0],
}

// SAFETY: matches the `struct __CGEvent *` AppKit returns from `-[NSEvent CGEvent]`.
unsafe impl RefEncode for CGEventRef {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("__CGEvent", &[]));
}
/// `kCGEventSourceUnixProcessID`: the process that posted the event, 0 for hardware.
const SOURCE_UNIX_PROCESS_ID: u32 = 41;

const EVENT_NAMES: &[(NSEventType, &str)] = &[
    (NSEventType::LeftMouseDown, "LeftMouseDown"),
    (NSEventType::LeftMouseUp, "LeftMouseUp"),
    (NSEventType::LeftMouseDragged, "LeftMouseDragged"),
    (NSEventType::RightMouseDown, "RightMouseDown"),
    (NSEventType::MouseMoved, "MouseMoved"),
    (NSEventType::KeyDown, "KeyDown"),
    (NSEventType::KeyUp, "KeyUp"),
    (NSEventType::FlagsChanged, "FlagsChanged"),
    (NSEventType::AppKitDefined, "AppKitDefined"),
    (NSEventType::SystemDefined, "SystemDefined"),
    (NSEventType::ApplicationDefined, "ApplicationDefined"),
    (NSEventType::Gesture, "Gesture"),
    (NSEventType::Pressure, "Pressure"),
];

/// AppKit's view of input at one moment.
#[derive(Debug, Clone)]
pub struct InputSnapshot {
    /// The event AppKit is dispatching, e.g. `LeftMouseDown`.
    pub event: Option<String>,
    /// How long ago the event happened, by its own timestamp.
    pub event_age_ms: Option<f64>,
    /// Who posted it: `hid` for the hardware, otherwise `pid (name)`.
    pub event_source: Option<String>,
    /// `NSEvent.pressedMouseButtons`; bit 0 is the primary button.
    pub buttons: usize,
    pub app_active: bool,
    /// The app macOS considers frontmost.
    pub frontmost: Option<String>,
    /// Crow's key window, by window number.
    pub key_window: Option<isize>,
    /// The class of the key window's first responder.
    pub first_responder: Option<String>,
}

impl fmt::Display for InputSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let or_none = |value: &Option<String>| value.clone().unwrap_or_else(|| "none".into());
        write!(f, "event={}", or_none(&self.event))?;
        if let Some(age) = self.event_age_ms {
            write!(f, " age_ms={age:.0}")?;
        }
        write!(
            f,
            " source={} buttons={:#b} app_active={} frontmost={} key_window={} first_responder={}",
            or_none(&self.event_source),
            self.buttons,
            self.app_active,
            or_none(&self.frontmost),
            self.key_window.map_or_else(|| "none".into(), |n| n.to_string()),
            or_none(&self.first_responder),
        )
    }
}

fn event_name(kind: NSEventType) -> String {
    EVENT_NAMES
        .iter()
        .find(|(known, _)| *known == kind)
        .map_or_else(|| format!("type{}", kind.0), |(_, name)| (*name).to_string())
}

fn describe_pid(pid: i32) -> String {
    let name = NSRunningApplication::runningApplicationWithProcessIdentifier(pid).and_then(|app| app.localizedName());
    match name {
        Some(name) => format!("{pid} ({name})"),
        None => pid.to_string(),
    }
}

fn event_source(event: &NSEvent) -> Option<String> {
    // SAFETY: `CGEvent` returns the event's backing CGEventRef (or nil), owned by the event.
    let cg: *const CGEventRef = unsafe { msg_send![event, CGEvent] };
    if cg.is_null() {
        return None;
    }
    // SAFETY: a field read from a live CGEventRef.
    let pid = unsafe { CGEventGetIntegerValueField(cg, SOURCE_UNIX_PROCESS_ID) };
    Some(match i32::try_from(pid) {
        Ok(0) => "hid".into(),
        Ok(pid) => describe_pid(pid),
        Err(_) => pid.to_string(),
    })
}

/// A snapshot of AppKit's input state; `None` off the main thread.
pub fn input_snapshot() -> Option<InputSnapshot> {
    let mtm = main_thread()?;
    let app = NSApplication::sharedApplication(mtm);
    let event = app.currentEvent();
    let now = NSProcessInfo::processInfo().systemUptime();
    let key: Option<Retained<NSWindow>> = app.keyWindow();
    let frontmost = NSWorkspace::sharedWorkspace().frontmostApplication().map(|front| {
        let name = front.localizedName().map(|n| n.to_string()).unwrap_or_default();
        format!("{} ({name})", front.processIdentifier())
    });
    Some(InputSnapshot {
        event: event.as_ref().map(|e| event_name(e.r#type())),
        event_age_ms: event.as_ref().map(|e| (now - e.timestamp()) * 1000.),
        event_source: event.as_deref().and_then(event_source),
        buttons: NSEvent::pressedMouseButtons(),
        app_active: app.isActive(),
        frontmost,
        key_window: key.as_ref().map(|w| w.windowNumber()),
        first_responder: key
            .as_ref()
            .and_then(|w| w.firstResponder())
            .map(|r| r.class().name().to_string_lossy().into_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_event_types_have_names_and_others_their_number() {
        assert_eq!(event_name(NSEventType::LeftMouseDown), "LeftMouseDown");
        assert_eq!(event_name(NSEventType::LeftMouseUp), "LeftMouseUp");
        assert_eq!(event_name(NSEventType(999)), "type999");
    }

    #[test]
    fn a_snapshot_reads_as_key_value_pairs() {
        let snapshot = InputSnapshot {
            event: Some("LeftMouseDown".into()),
            event_age_ms: Some(12.4),
            event_source: Some("hid".into()),
            buttons: 1,
            app_active: true,
            frontmost: Some("42 (Crow)".into()),
            key_window: Some(7),
            first_responder: None,
        };
        assert_eq!(
            snapshot.to_string(),
            "event=LeftMouseDown age_ms=12 source=hid buttons=0b1 app_active=true frontmost=42 (Crow) \
             key_window=7 first_responder=none"
        );
    }
}
