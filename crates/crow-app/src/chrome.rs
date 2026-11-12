//! Shared UI parts: keycaps, tooltips and sheets. Later U plans build their
//! surfaces from these, so a keycap, a tooltip and an overlay look the same
//! everywhere.

use gpui::{
    App, Context, Div, IntoElement, KeyBinding, Render, RenderOnce, Role, SharedString, Stateful,
    StatefulInteractiveElement, Window, div, prelude::*, px,
};

use crate::keys::{self, KeyScope};
use crate::style::{color, ui_font};
use crate::ui::Ui;

/// How a keycap draws: an outlined chord, one filled cap per key, or plain text.
// U1 only uses Text; U3–U7 draw Outline and Key.
#[allow(dead_code)]
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
}

// `from_binding` and `for_action` are the builders U3–U7 use.
#[allow(dead_code)]
impl Keycap {
    /// A literal shortcut, e.g. `"esc"` (`Key` splits it on nothing: use
    /// `from_binding` for a chord).
    pub fn text(text: impl Into<SharedString>, weight: KeycapWeight) -> Self {
        Self { caps: vec![text.into()], weight }
    }

    pub fn from_binding(binding: &KeyBinding, weight: KeycapWeight) -> Self {
        let caps = match weight {
            KeycapWeight::Key => keys::caps(binding),
            KeycapWeight::Outline | KeycapWeight::Text => vec![keys::chord_text(binding)],
        };
        Self { caps, weight }
    }

    pub fn for_action(action: &dyn gpui::Action, scope: KeyScope, weight: KeycapWeight, cx: &App) -> Option<Self> {
        keys::binding_for(action, scope, cx).as_ref().map(|binding| Self::from_binding(binding, weight))
    }
}

fn cap(text: SharedString, theme: &crow_theme::Theme, key: bool) -> Div {
    div()
        .h(px(if key { 20. } else { 18. }))
        .min_w(px(if key { 20. } else { 18. }))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(color(theme.role("surface-raised")))
        .when(key, |cap| cap.border_b_2().bg(color(theme.role("bg"))))
        .text_size(px(if key { 11.5 } else { 11. }))
        .text_color(color(if key { theme.role("fg") } else { theme.role("fg-muted") }))
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
            KeycapWeight::Text => {
                div().text_color(color(theme.role("fg-muted"))).child(self.caps.concat()).into_any_element()
            }
            KeycapWeight::Outline => cap(self.caps.concat().into(), &theme, false).into_any_element(),
            KeycapWeight::Key => div()
                .flex()
                .items_center()
                .gap(px(3.))
                .children(self.caps.into_iter().map(|cap_text| cap(cap_text, &theme, true)))
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
        div()
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
            .children(keys.map(|keys| Keycap::text(keys, KeycapWeight::Text)))
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

/// Attach a tooltip with an optional displayed key. The element keeps its aria
/// label; the tooltip's own label is `label` plus the key.
pub fn with_tooltip<E: StatefulInteractiveElement>(
    element: E,
    label: impl Into<SharedString>,
    keys: Option<SharedString>,
) -> E {
    let label = label.into();
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
    let label = label.into();
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
}

impl SheetWidth {
    fn px(self) -> f32 {
        match self {
            SheetWidth::Compact => 360.,
            SheetWidth::Narrow => 520.,
            SheetWidth::Wide => 620.,
        }
    }
}

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

/// A full-window scrim with a centered panel. A left mouse-down outside the
/// panel runs the overlay's own cancel path (`on_dismiss`), exactly like Esc.
pub fn sheet(
    panel: impl IntoElement,
    theme: &crow_theme::Theme,
    on_dismiss: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id("sheet-scrim")
        .debug_selector(|| "sheet-scrim".into())
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .bg(crate::style::scrim(theme))
        .occlude()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(52.))
        .on_mouse_down(gpui::MouseButton::Left, on_dismiss)
        .child(panel)
}
