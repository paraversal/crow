//! Layout and divider gestures for the sidebar and Preview/Info.

use gpui::{
    Context, CursorStyle, FocusHandle, KeyDownEvent, MouseButton, MouseDownEvent, Pixels, Role, div, prelude::*, px,
};

use crate::{
    style::color,
    ui::{self, SettingChange},
};

use super::CrowWindow;

const LEFT_MIN: u32 = 140;
const LEFT_MAX: u32 = 480;

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
    position: Pixels,
}

impl Drag {
    fn current_width(&self) -> Pixels {
        self.width + (self.position - self.origin) * self.side.direction()
    }

    fn resized_left(&self) -> Option<u32> {
        (matches!(self.side, Side::Left) && self.position != self.origin)
            .then(|| left_width_setting(self.current_width()))
    }
}

fn left_width_setting(width: Pixels) -> u32 {
    (f32::from(width).round() as u32).clamp(LEFT_MIN, LEFT_MAX)
}

pub(super) struct SidePanes {
    pub(super) left_focus: FocusHandle,
    left: Pixels,
    right: Pixels,
    drag: Option<Drag>,
}

impl SidePanes {
    pub(super) fn new(cx: &mut Context<CrowWindow>) -> Self {
        Self { left_focus: cx.focus_handle(), left: px(210.), right: px(300.), drag: None }
    }

    pub(super) fn layout(
        &mut self,
        viewport: Pixels,
        preferred_left: u32,
        preferred_right: Option<Pixels>,
        gallery_with_info: bool,
    ) {
        let center_min = if gallery_with_info { px(320.) } else { px(130.) };
        let right_min = px(160.);
        let left = self.drag.as_ref().and_then(Drag::resized_left).unwrap_or(preferred_left).clamp(LEFT_MIN, LEFT_MAX);
        self.left = px(left as f32).min((viewport - center_min - right_min).max(px(LEFT_MIN as f32)));
        self.right = preferred_right.unwrap_or(px(300.)).min((viewport - self.left - center_min).max(right_min));
    }

    pub(super) fn left_width(&self) -> Pixels {
        self.left
    }

    pub(super) fn right_width(&self) -> Pixels {
        self.right
    }

    fn drag_to(&mut self, x: Pixels) -> Option<(Side, Pixels)> {
        let drag = self.drag.as_mut()?;
        drag.position = x;
        Some((drag.side, drag.current_width()))
    }
}

impl CrowWindow {
    pub(super) fn drag_side_pane(&mut self, x: Pixels, cx: &mut Context<Self>) {
        match self.side_panes.drag_to(x) {
            Some((Side::Left, _)) => cx.notify(),
            Some((Side::Right, width)) => self.set_side_pane_width(Side::Right, Some(width), cx),
            None => {}
        }
    }

    pub(super) fn finish_side_pane_resize(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.side_panes.drag.take() else { return };
        if let Some(width) = drag.resized_left() {
            ui::set_setting(SettingChange::SidebarWidth(width), cx);
        }
        cx.notify();
    }

    fn set_side_pane_width(&mut self, side: Side, width: Option<Pixels>, cx: &mut Context<Self>) {
        match side {
            Side::Left => {
                let width =
                    width.map(left_width_setting).unwrap_or_else(|| crow_core::WindowDefaults::default().sidebar_width);
                ui::set_setting(SettingChange::SidebarWidth(width), cx);
            }
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
        // The splitter moves under the pointer as it resizes. Refresh when the
        // hover listener sees the new hitbox, even if no mouse move hit that box.
        .on_hover(|_, window, _| window.refresh())
        .focus_visible(|style| style.bg(line.opacity(0.25)))
        .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, window, cx| {
            let Some(handle) = window.root::<CrowWindow>().flatten() else { return };
            handle.update(cx, |this, cx| {
                window.focus(&focus, cx);
                if event.click_count > 1 {
                    this.side_panes.drag = None;
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
                    let origin = event.position.x;
                    this.side_panes.drag = Some(Drag { side, origin, width, position: origin });
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
