//! Dragging the window by Crow's own titlebar.
//!
//! Crow draws the titlebar, so AppKit does not drag the window itself: the press asks
//! AppKit to start the drag and WindowServer performs it. macOS 27.2's WindowServer
//! intermittently refuses that handoff. The request returns, nothing raises an error,
//! the pointer moves and the window stays where it is. Native titlebars are unaffected
//! because WindowServer starts those drags itself, which is why Finder and Safari keep
//! working while every custom-chrome app (Crow, Electron, Warp) fails at once.
//!
//! The drag is therefore watched. If the pointer travels and the window does not, the
//! window is moved from a local event monitor until the button comes up. AppKit is
//! always asked first: its drag keeps edge snapping, window tiling and dragging into
//! another Space, which a hand-placed window cannot do.

use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSApplication, NSEvent, NSEventMask, NSEventType, NSScreen, NSWindow};
use objc2_foundation::NSPoint;
use raw_window_handle::HasWindowHandle;

use crate::drag::ns_view;
use crate::input_diag::{InputSnapshot, input_snapshot};
use crate::main_thread;

/// How often a drag is checked for a window that is not following. One frame: a titlebar
/// flick can be over in 100 ms, and two checks must fit inside it.
const STEP: Duration = Duration::from_millis(16);
/// A press held longer than this is not a titlebar drag any more; stop watching.
const LIMIT: Duration = Duration::from_secs(10);
/// Pointer travel (points) below which a press is a click, not a drag.
const SLOP: f64 = 3.;
/// Pointer travel after which a window that has not moved is being ignored.
const IGNORED_AFTER: f64 = 6.;
/// How much of the titlebar stays on screen when the window is moved by hand.
const KEEP_ON_SCREEN: f64 = 24.;

thread_local! {
    /// The mouse monitor of the drag being moved by hand, until its mouse-up.
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

/// One titlebar press, from the AppKit request to the mouse-up.
pub struct TitlebarDrag {
    window: Retained<NSWindow>,
    start_origin: NSPoint,
    height: f64,
    start_pointer: NSPoint,
    span: Option<(f64, f64)>,
    snapshot: InputSnapshot,
    begun: Instant,
}

impl TitlebarDrag {
    /// Take the press and the window's frame; call just before asking the window to move.
    pub fn begin(window: &impl HasWindowHandle) -> Option<Self> {
        let snapshot = input_snapshot()?;
        let ns_window = ns_view(window)?.window()?;
        let frame = ns_window.frame();
        Some(Self {
            window: ns_window,
            start_origin: frame.origin,
            height: frame.size.height,
            start_pointer: NSEvent::mouseLocation(),
            span: screen_span(),
            snapshot,
            begun: Instant::now(),
        })
    }

    /// The window was asked to move; watch it and take over if it stays put.
    pub fn started(self) {
        let call_ms = self.begun.elapsed().as_secs_f64() * 1000.;
        poll(Watch {
            window_number: self.window.windowNumber(),
            start_origin: self.start_origin,
            height: self.height,
            start_pointer: self.start_pointer,
            span: self.span,
            snapshot: self.snapshot,
            begun: self.begun,
            call_ms,
            max_travel: 0.,
            ignored_once: false,
            timed_out: false,
        });
    }
}

/// What the press ended as, for the log line.
enum Outcome {
    Moved,
    Clicked,
    Late,
    Ignored,
}

/// Plain values, so it can cross the dispatch queue and be looked up again on the main thread.
struct Watch {
    window_number: isize,
    start_origin: NSPoint,
    height: f64,
    start_pointer: NSPoint,
    span: Option<(f64, f64)>,
    snapshot: InputSnapshot,
    begun: Instant,
    call_ms: f64,
    max_travel: f64,
    /// The pointer has already been seen moving while the window stayed put.
    ignored_once: bool,
    timed_out: bool,
}

fn poll(mut watch: Watch) {
    let Some(mtm) = main_thread() else { return };
    let Some(window) = NSApplication::sharedApplication(mtm).windowWithWindowNumber(watch.window_number) else {
        // The window closed mid-drag; nothing left to watch.
        return;
    };
    let pointer = NSEvent::mouseLocation();
    watch.max_travel = watch.max_travel.max(travel(pointer, watch.start_pointer));
    let elapsed = watch.begun.elapsed();
    let held = NSEvent::pressedMouseButtons() & 1 != 0;
    if held && elapsed < LIMIT {
        let window_travel = travel(window.frame().origin, watch.start_origin);
        let ignored = watch.max_travel >= IGNORED_AFTER && window_travel < 1.;
        // Two checks in a row: WindowServer moves the window a frame after the pointer,
        // and one slow poll must not turn a working native drag into a manual one.
        if ignored && watch.ignored_once && take_over(&watch, window) {
            return;
        }
        watch.ignored_once = ignored;
        schedule(move || poll(watch));
        return;
    }
    watch.timed_out = held && elapsed >= LIMIT;
    watch.finish(outcome(&watch, &window), &window);
}

fn outcome(watch: &Watch, window: &NSWindow) -> Outcome {
    let window_travel = travel(window.frame().origin, watch.start_origin);
    if window_travel >= 1. {
        Outcome::Moved
    } else if watch.snapshot.buttons & 1 == 0 {
        Outcome::Late
    } else if watch.max_travel >= SLOP {
        Outcome::Ignored
    } else {
        Outcome::Clicked
    }
}

impl Watch {
    fn finish(&self, outcome: Outcome, window: &NSWindow) {
        let window_travel = travel(window.frame().origin, self.start_origin);
        let (verdict, warn) = match outcome {
            Outcome::Moved => ("moved", false),
            Outcome::Clicked => ("click without a drag", false),
            Outcome::Late => ("FAILED: button already up when the move began", true),
            Outcome::Ignored => ("FAILED: pointer dragged but the window stayed", true),
        };
        let line = format!(
            "window move: {verdict}; held_ms={:.0} call_ms={:.1} pointer_travel={:.0} window_travel={window_travel:.0} timed_out={} at start: {}",
            self.begun.elapsed().as_secs_f64() * 1000.,
            self.call_ms,
            self.max_travel,
            self.timed_out,
            self.snapshot,
        );
        if warn {
            tracing::warn!(target: "input", "{line}");
        } else {
            tracing::info!(target: "input", "{line}");
        }
    }
}

fn schedule(next: impl FnOnce() + Send + 'static) {
    let after = dispatch2::DispatchTime::try_from(STEP).unwrap_or(dispatch2::DispatchTime::NOW);
    let _ = dispatch2::DispatchQueue::main().after(after, next);
}

/// Move the window by hand until the button comes up; false if no monitor was installed.
fn take_over(watch: &Watch, window: Retained<NSWindow>) -> bool {
    let start_origin = watch.start_origin;
    let height = watch.height;
    let span = watch.span;
    let start_pointer = watch.start_pointer;
    let begun = watch.begun;
    let call_ms = watch.call_ms;
    let snapshot = watch.snapshot.clone();
    let max_travel = Cell::new(travel(NSEvent::mouseLocation(), start_pointer));
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands this block a live event for the duration of the call.
        let kind = unsafe { event.as_ref() }.r#type();
        match kind {
            NSEventType::LeftMouseUp => {
                remove_monitor();
                let window_travel = travel(window.frame().origin, start_origin);
                let verdict = if window_travel < 1. {
                    "FAILED: moved by hand but the window stayed"
                } else {
                    "moved manually (AppKit ignored the drag)"
                };
                let line = format!(
                    "window move: {verdict}; held_ms={:.0} call_ms={call_ms:.1} pointer_travel={:.0} window_travel={window_travel:.0} timed_out=false at start: {snapshot}",
                    begun.elapsed().as_secs_f64() * 1000.,
                    max_travel.get(),
                );
                tracing::warn!(target: "input", "{line}");
            }
            NSEventType::LeftMouseDown => {
                // A new press means this drag ended without its mouse-up.
                remove_monitor();
            }
            _ => {
                let pointer = NSEvent::mouseLocation();
                max_travel.set(max_travel.get().max(travel(pointer, start_pointer)));
                let mut origin = NSPoint::new(
                    start_origin.x + pointer.x - start_pointer.x,
                    start_origin.y + pointer.y - start_pointer.y,
                );
                if let Some(span) = span {
                    origin.y = clamp_y(origin.y, height, span);
                }
                window.setFrameOrigin(origin);
            }
        }
        event.as_ptr()
    });
    // SAFETY: the mask names only mouse events; the block returns the event unchanged.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp | NSEventMask::LeftMouseDown,
            &block,
        )
    };
    let Some(monitor) = monitor else { return false };
    remove_monitor();
    MONITOR.with(|slot| *slot.borrow_mut() = Some(monitor));
    tracing::warn!(
        target: "input",
        "window move: AppKit did not start the drag; moving the window from its mouse events"
    );
    true
}

fn remove_monitor() {
    if let Some(monitor) = MONITOR.with(|slot| slot.borrow_mut().take()) {
        // SAFETY: the token came from addLocalMonitorForEventsMatchingMask:handler:.
        unsafe { NSEvent::removeMonitor(&monitor) };
    }
}

fn travel(a: NSPoint, b: NSPoint) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The vertical range of every screen's visible frame, below the menu bar and above the Dock.
fn screen_span() -> Option<(f64, f64)> {
    let mtm = main_thread()?;
    NSScreen::screens(mtm).iter().map(|screen| screen.visibleFrame()).fold(None, |span, frame| {
        let (bottom, top) = (frame.origin.y, frame.origin.y + frame.size.height);
        Some(span.map_or((bottom, top), |(low, high): (f64, f64)| (low.min(bottom), high.max(top))))
    })
}

/// Keep a hand-moved window under the menu bar and above the bottom of the screens.
fn clamp_y(origin_y: f64, height: f64, span: (f64, f64)) -> f64 {
    let (bottom, top) = span;
    origin_y.min(top - height.max(KEEP_ON_SCREEN)).max(bottom - height + KEEP_ON_SCREEN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hand_moved_window_stays_below_the_menu_bar_and_above_the_bottom() {
        let span = (0., 900.);
        assert_eq!(clamp_y(1400., 600., span), 300.);
        assert_eq!(clamp_y(-1000., 600., span), -576.);
        assert_eq!(clamp_y(100., 600., span), 100.);
        // Taller than the screen: the top wins, so the titlebar stays reachable.
        assert_eq!(clamp_y(100., 1000., span), -100.);
    }

    #[test]
    fn travel_is_the_distance_between_points() {
        assert_eq!(travel(NSPoint::new(0., 0.), NSPoint::new(3., 4.)), 5.);
        assert_eq!(travel(NSPoint::new(7., 2.), NSPoint::new(7., 2.)), 0.);
    }
}
