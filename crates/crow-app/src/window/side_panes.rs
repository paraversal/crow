//! Per-window widths and divider gestures for the sidebar and Preview/Info.

use gpui::{
    Context, CursorStyle, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Role, div, prelude::*, px,
};

use crate::style::color;

use super::CrowWindow;

#[derive(Clone, Copy)]
pub(crate) enum Side {
    Left,
    Right,
}

impl Side {
    fn direction(self) -> f32 {
        match self {
            Self::Left => 1.,
            Self::Right => -1.,
        }
    }
}

struct Drag {
    side: Side,
    origin: Pixels,
    width: Pixels,
}

pub(super) struct SidePanes {
    pub(super) left_focus: FocusHandle,
    preferred_left: Option<Pixels>,
    left: Pixels,
    right: Pixels,
    drag: Option<Drag>,
}

impl SidePanes {
    pub(super) fn new(cx: &mut Context<CrowWindow>) -> Self {
        Self { left_focus: cx.focus_handle(), preferred_left: None, left: px(210.), right: px(300.), drag: None }
    }

    pub(super) fn layout(&mut self, viewport: Pixels, preferred_right: Option<Pixels>) {
        self.left = self.preferred_left.unwrap_or(px(210.)).min((viewport - px(290.)).max(px(140.)));
        self.right = preferred_right.unwrap_or(px(300.)).min((viewport - self.left - px(130.)).max(px(160.)));
    }

    pub(super) fn left_width(&self) -> Pixels {
        self.left
    }

    pub(super) fn right_width(&self) -> Pixels {
        self.right
    }

    pub(super) fn drag_to(&self, x: Pixels) -> Option<(Side, Pixels)> {
        let drag = self.drag.as_ref()?;
        Some((drag.side, drag.width + (x - drag.origin) * drag.side.direction()))
    }

    pub(super) fn finish(&mut self) {
        self.drag = None;
    }
}

impl CrowWindow {
    pub(super) fn set_side_pane_width(&mut self, side: Side, width: Option<Pixels>, cx: &mut Context<Self>) {
        match side {
            Side::Left => self.side_panes.preferred_left = width.map(|width| width.clamp(px(140.), px(480.))),
            Side::Right => {
                self.info.update(cx, |info, cx| {
                    info.preferred_width = width.map(|width| width.clamp(px(160.), px(640.)));
                    cx.notify();
                });
            }
        }
        cx.notify();
    }
}

pub(crate) fn splitter(side: Side, focus: &FocusHandle, theme: &crow_theme::Theme) -> impl IntoElement + use<> {
    let (id, label) = match side {
        Side::Left => ("sidebar-resize", "Resize Sidebar"),
        Side::Right => ("preview-resize", "Resize Preview and Info"),
    };
    let focus = focus.clone();
    let line = color(theme.role("accent"));
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Splitter)
        .aria_label(label)
        .track_focus(&focus)
        .tab_stop(true)
        .absolute()
        .top_0()
        .bottom_0()
        .w(px(8.))
        .cursor(CursorStyle::ResizeLeftRight)
        .when(matches!(side, Side::Left), |handle| handle.right_0())
        .when(matches!(side, Side::Right), |handle| handle.left_0())
        .hover(move |handle| handle.bg(line.opacity(0.25)))
        .focus(|style| style.bg(line.opacity(0.25)))
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
            let Some(handle) = window.root::<CrowWindow>().flatten() else { return };
            handle.update(cx, |this, cx| {
                window.focus(&focus, cx);
                if event.click_count > 1 {
                    this.side_panes.finish();
                    this.set_side_pane_width(side, None, cx);
                } else {
                    let width = match side {
                        Side::Left => this.side_panes.left,
                        Side::Right
                            if this.search.is_none() && this.pane().read(cx).mode() == crow_core::ViewMode::Columns =>
                        {
                            (window.viewport_size().width - event.position.x).max(px(160.))
                        }
                        Side::Right => this.side_panes.right,
                    };
                    this.side_panes.drag = Some(Drag { side, origin: event.position.x, width });
                }
            });
            cx.stop_propagation();
        })
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if event.keystroke.modifiers.modified() {
                return;
            }
            let delta = match event.keystroke.key.as_str() {
                "left" => Some(-16.),
                "right" => Some(16.),
                "enter" => None,
                _ => return,
            };
            let Some(handle) = window.root::<CrowWindow>().flatten() else { return };
            handle.update(cx, |this, cx| {
                let width = match side {
                    Side::Left => this.side_panes.left,
                    Side::Right => this.side_panes.right,
                };
                this.set_side_pane_width(side, delta.map(|delta| width + px(delta * side.direction())), cx);
            });
            cx.stop_propagation();
        })
}
