//! Shared UI parts: keycaps, tooltips and sheets. Later U plans build their
//! surfaces from these, so a keycap, a tooltip and an overlay look the same
//! everywhere.

use std::time::Duration;

use crow_core::motion::{self, Effective};
use gpui::{
    Animation, AnimationExt, App, Context, Div, IntoElement, KeyBinding, Render, RenderOnce, Role, SharedString,
    Stateful, StatefulInteractiveElement, Window, div, prelude::*, px,
};

use crate::keys::{self, KeyScope};
use crate::style::{BAND_HEIGHT, color, ui_font};
use crate::ui::Ui;

/// How a keycap draws: an outlined chord, one filled cap per key, or plain text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeycapWeight {
    Outline,
    Key,
    Text,
}

/// A keycap drawn from the live keymap. Decorative: the enclosing control
/// carries the key in words in its aria label.
#[derive(IntoElement)]
pub struct Keycap {
    caps: Vec<SharedString>,
    weight: KeycapWeight,
    /// Outline border role. Defaults to `surface-raised`; a control whose fill is
    /// also `surface-raised` passes a contrasting role so the cap stays visible.
    border: Option<&'static str>,
    /// Text role override, for a cap whose surface needs dimmer or brighter text.
    text_role: Option<&'static str>,
}

impl Keycap {
    /// A literal shortcut, e.g. `"esc"` (`Key` splits it on nothing: use
    /// `from_binding` for a chord).
    pub fn text(text: impl Into<SharedString>, weight: KeycapWeight) -> Self {
        Self { caps: vec![text.into()], weight, border: None, text_role: None }
    }

    pub fn from_binding(binding: &KeyBinding, weight: KeycapWeight) -> Self {
        let caps = match weight {
            KeycapWeight::Key => keys::caps(binding),
            KeycapWeight::Outline | KeycapWeight::Text => vec![keys::chord_text(binding)],
        };
        Self { caps, weight, border: None, text_role: None }
    }

    pub fn for_action(action: &dyn gpui::Action, scope: KeyScope, weight: KeycapWeight, cx: &App) -> Option<Self> {
        keys::binding_for(action, scope, cx).as_ref().map(|binding| Self::from_binding(binding, weight))
    }

    /// The Outline border's theme role, when the surface behind it needs contrast.
    pub fn border(mut self, role: &'static str) -> Self {
        self.border = Some(role);
        self
    }

    /// The key text's theme role. Focus hints read `fg`; idle ones `fg-subtle`.
    pub fn text_role(mut self, role: &'static str) -> Self {
        self.text_role = Some(role);
        self
    }
}

fn cap(text: SharedString, theme: &crow_theme::Theme, key: bool, border: Option<&str>, text_role: Option<&str>) -> Div {
    let border_role = border.unwrap_or("surface-raised");
    div()
        .h(px(if key { 20. } else { 18. }))
        .min_w(px(if key { 20. } else { 18. }))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(color(theme.role(border_role)))
        .when(key, |cap| cap.border_b_2().bg(color(theme.role("bg"))))
        .text_size(px(if key { 11.5 } else { 11. }))
        .text_color(color(theme.role(text_role.unwrap_or(if key { "fg" } else { "fg-muted" }))))
        .flex_none()
        .whitespace_nowrap()
        .child(text)
}

impl RenderOnce for Keycap {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.global::<Ui>().theme.clone();
        if self.caps.is_empty() {
            return div().into_any_element();
        }
        match self.weight {
            KeycapWeight::Text => div()
                .text_color(color(theme.role(self.text_role.unwrap_or("fg-muted"))))
                .child(self.caps.concat())
                .into_any_element(),
            KeycapWeight::Outline => {
                cap(self.caps.concat().into(), &theme, false, self.border, self.text_role).into_any_element()
            }
            KeycapWeight::Key => div()
                .flex()
                .items_center()
                .gap(px(3.))
                .children(
                    self.caps.into_iter().map(|cap_text| cap(cap_text, &theme, true, self.border, self.text_role)),
                )
                .into_any_element(),
        }
    }
}

/// "Back ⌘[" — the label plus the key when there is one.
pub fn tooltip_text(label: &str, keys: Option<&str>) -> String {
    match keys {
        Some(keys) if !keys.is_empty() => format!("{label} {keys}"),
        _ => label.to_owned(),
    }
}

struct Tooltip {
    label: SharedString,
    keys: Option<SharedString>,
    spoken: String,
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ui = cx.global::<Ui>();
        let theme = &ui.theme;
        let label = self.label.clone();
        let keys = self.keys.clone();
        let tip = div()
            .id("tooltip")
            .debug_selector(|| "tooltip".into())
            .role(Role::Tooltip)
            .aria_label(self.spoken.clone())
            .px(px(8.))
            .py(px(5.))
            .rounded(px(6.))
            .bg(color(theme.role("surface-raised")))
            .shadow_lg()
            .font_family(ui_font(&ui.settings.font_family))
            .text_size(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().text_color(color(theme.role("fg"))).child(label))
            .children(keys.map(|keys| Keycap::text(keys, KeycapWeight::Text)));
        // Fades in; a tooltip is a fresh view each time, so the clock starts as it shows.
        // It goes at once. Opacity only, so Low and Full are alike.
        match ui.feedback.mode {
            Effective::Off => tip.into_any_element(),
            _ => tip
                .with_animation("tooltip-in", Animation::new(Duration::from_millis(motion::ms::TOOLTIP)), |el, t| {
                    el.opacity(motion::ease_out(t))
                })
                .into_any_element(),
        }
    }
}

fn tooltip<E: StatefulInteractiveElement>(
    element: E,
    label: SharedString,
    keys: Option<SharedString>,
    spoken: String,
) -> E {
    element.tooltip(move |_window, cx| {
        cx.new(|_| Tooltip { label: label.clone(), keys: keys.clone(), spoken: spoken.clone() }).into()
    })
}

/// The small chevron at a sidebar's edge that hides it (or, in the toolbar, brings it back):
/// it points the way the column goes, stays quiet until hovered, and runs `action` (the same command as the key and
/// the menu item). `label` is the tooltip and the aria label.
pub fn collapse_chevron(
    id: &'static str,
    label: &'static str,
    glyph: crate::icons::Glyph,
    action: &dyn gpui::Action,
    theme: &crow_theme::Theme,
    cx: &App,
) -> Stateful<Div> {
    let hover_icon = color(theme.role("fg"));
    let icon = crate::icons::glyph(glyph, 14., "fg-subtle", theme)
        .id("icon")
        .group_hover(id, move |s| s.text_color(hover_icon));
    let button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .group(id)
        .flex_none()
        .w(px(24.))
        .h(px(24.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .hover(|s| s.bg(color(theme.role("surface"))))
        .active(|s| s.bg(color(theme.role("surface-raised"))))
        // The traffic lights' strip drags the window: a press here must not start that.
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(icon);
    let dispatch = action.boxed_clone();
    with_action_tooltip(button, label, action, KeyScope::Window, cx)
        .on_click(move |_, window, cx| window.dispatch_action(dispatch.boxed_clone(), cx))
}

/// Tooltip text is often a file name; control characters in it must not reach text
/// shaping, which rejects newlines.
fn tooltip_label(label: SharedString) -> SharedString {
    match crate::style::display_text(&label) {
        std::borrow::Cow::Borrowed(_) => label,
        std::borrow::Cow::Owned(clean) => clean.into(),
    }
}

/// Attach a tooltip with an optional displayed key. The element keeps its aria
/// label; the tooltip's own label is `label` plus the key.
pub fn with_tooltip<E: StatefulInteractiveElement>(
    element: E,
    label: impl Into<SharedString>,
    keys: Option<SharedString>,
) -> E {
    let label = tooltip_label(label.into());
    let spoken = tooltip_text(&label, keys.as_deref());
    tooltip(element, label, keys, spoken)
}

/// Attach a tooltip whose key comes from the live keymap; its aria label spells
/// the key out ("Back to Desktop Command [").
pub fn with_action_tooltip<E: StatefulInteractiveElement>(
    element: E,
    label: impl Into<SharedString>,
    action: &dyn gpui::Action,
    scope: KeyScope,
    cx: &App,
) -> E {
    let label = tooltip_label(label.into());
    let keys = keys::text(action, scope, cx);
    let spoken = match keys::spoken_for(action, scope, cx) {
        Some(spoken) => tooltip_text(&label, Some(&spoken)),
        None => label.to_string(),
    };
    tooltip(element, label, keys, spoken)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::MoveUp;
    use gpui::{TestAppContext, VisualTestContext, point, size};

    #[test]
    fn tooltip_labels_lose_control_characters() {
        assert_eq!(tooltip_label("a\nb".into()), "a b");
        assert_eq!(tooltip_label("plain".into()), "plain");
    }

    #[test]
    fn tooltip_text_joins_label_and_key() {
        assert_eq!(tooltip_text("Back to Desktop", Some("⌘[")), "Back to Desktop ⌘[");
        assert_eq!(tooltip_text("Back", None), "Back");
        assert_eq!(tooltip_text("Back", Some("")), "Back");
    }

    #[test]
    fn keycap_splits_a_chord_into_keys() {
        let binding = KeyBinding::new("cmd-shift-a", MoveUp, None);
        let key = Keycap::from_binding(&binding, KeycapWeight::Key);
        assert_eq!(key.caps, vec!["⇧", "⌘", "A"]);
        let outline = Keycap::from_binding(&binding, KeycapWeight::Outline);
        assert_eq!(outline.caps, vec!["⇧⌘A"]);
        assert_eq!(Keycap::text("esc", KeycapWeight::Text).caps, vec!["esc"]);
    }

    struct Showcase;

    impl Render for Showcase {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

    #[gpui::test]
    fn keycaps_render_in_all_weights(cx: &mut TestAppContext) {
        cx.update(crate::ui::init_for_tests);
        let window = cx
            .update(|cx| cx.open_window(Default::default(), |_, cx| cx.new(|_| Showcase)))
            .expect("open a test window");
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        let binding = KeyBinding::new("cmd-shift-a", MoveUp, None);
        cx.draw(point(px(0.), px(0.)), size(px(320.), px(80.)), |_, _| {
            div()
                .flex()
                .gap(px(6.))
                .child(Keycap::text("esc", KeycapWeight::Outline))
                .child(Keycap::from_binding(&binding, KeycapWeight::Key))
                .child(Keycap::text("esc", KeycapWeight::Text))
        });
    }
}

/// Width of a sheet panel, as the boards show them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetWidth {
    Compact,
    Narrow,
    Wide,
    /// A width in px chosen by the sheet from its content.
    Fit(u16),
}

impl SheetWidth {
    fn px(self) -> f32 {
        match self {
            SheetWidth::Compact => 360.,
            SheetWidth::Narrow => 520.,
            SheetWidth::Wide => 620.,
            SheetWidth::Fit(width) => f32::from(width),
        }
    }
}

/// How far a leaving sheet's panel sinks in Full, in px.
const SHEET_SINK: f32 = 4.0;

/// The panel inside a sheet: 12 px radius, raised border, deep shadow. Callers
/// add their padding, context, actions and children.
pub fn sheet_panel(id: impl Into<gpui::ElementId>, width: SheetWidth, theme: &crow_theme::Theme) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(|| "sheet-panel".into())
        .role(Role::Dialog)
        .w(px(width.px()))
        .rounded(px(12.))
        .bg(color(theme.role("surface")))
        .border_1()
        .border_color(color(theme.role("surface-raised")))
        .shadow_2xl()
        .overflow_hidden()
        // A click inside never reaches the scrim's dismiss handler.
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// A sheet's push button: raised, or accent-filled when `primary`. Callers add
/// focus tracking, a disabled look and the click.
pub fn sheet_button(
    id: &'static str,
    label: impl Into<SharedString>,
    primary: bool,
    theme: &crow_theme::Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label.clone())
        .px_3()
        .py_1()
        .rounded_md()
        .bg(color(if primary { theme.role("accent") } else { theme.role("surface-raised") }))
        .text_color(color(if primary { theme.role("on-accent") } else { theme.role("fg") }))
        .child(label)
}

/// Move focus to the next (or previous) control, wrapping at the ends; from
/// outside the list, to its first (or last).
pub fn cycle_focus(controls: &[gpui::FocusHandle], forward: bool, window: &mut Window, cx: &mut App) {
    if controls.is_empty() {
        return;
    }
    let current = controls.iter().position(|control| control.is_focused(window));
    let next = match (current, forward) {
        (Some(index), true) => (index + 1) % controls.len(),
        (Some(index), false) => (index + controls.len() - 1) % controls.len(),
        (None, true) => 0,
        (None, false) => controls.len() - 1,
    };
    window.focus(&controls[next], cx);
}

/// A full-window scrim with a centered panel. A left mouse-down outside the
/// panel runs the overlay's own cancel path (`on_dismiss`), exactly like Esc.
///
/// `leaving` is `Some(opacity)` (1 → 0) while a closed sheet's last picture
/// fades out (`motion_ui::Leaving`): it takes no clicks, then, and in Full the
/// panel sinks 4 px. The scrim fades in on open and out on close in Low and Full.
pub fn sheet(
    panel: impl IntoElement + gpui::Styled + 'static,
    theme: &crow_theme::Theme,
    mode: Effective,
    leaving: Option<f32>,
    on_dismiss: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let panel: gpui::AnyElement = match (mode, leaving) {
        (Effective::Off, _) => panel.into_any_element(),
        (Effective::Low, Some(o)) => panel.opacity(o).into_any_element(),
        (Effective::Full, Some(o)) => panel.relative().top(px(SHEET_SINK * (1.0 - o))).opacity(o).into_any_element(),
        (Effective::Low, None) => panel
            .with_animation("sheet-open", Animation::new(Duration::from_millis(motion::ms::SHEET_IN_LOW)), |el, t| {
                el.opacity(motion::ease_out(t))
            })
            .into_any_element(),
        (Effective::Full, None) => panel
            .with_animation("sheet-open", Animation::new(Duration::from_millis(motion::ms::SHEET_IN)), |el, t| {
                el.opacity(motion::ease_out(t))
            })
            .into_any_element(),
    };
    let scrim = div().absolute().top_0().left_0().right_0().bottom_0().bg(crate::style::scrim(theme));
    let scrim = match (mode, leaving) {
        (Effective::Off, _) => scrim.into_any_element(),
        (_, Some(o)) => scrim.opacity(o).into_any_element(),
        (_, None) => scrim
            .with_animation("sheet-scrim", Animation::new(Duration::from_millis(motion::ms::SCRIM)), |el, t| {
                el.opacity(motion::ease_out(t))
            })
            .into_any_element(),
    };
    let root = div()
        .id("sheet-scrim")
        .debug_selector(|| "sheet-scrim".into())
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(BAND_HEIGHT))
        .child(scrim)
        .child(panel);
    if leaving.is_some() {
        // Its focus and state moved on already: nothing here takes input.
        root
    } else {
        root.occlude().on_mouse_down(gpui::MouseButton::Left, on_dismiss)
    }
}
