//! Shared settings controls: a switch, segmented choices, image tiles, a numeric
//! stepper and a type-to-select list. Every control draws with theme roles only,
//! carries a visible label that is also its aria label, and draws the sidebar's
//! inset cursor ring when focused (so focus never resizes it).

use std::rc::Rc;

use crow_theme::Theme;
use gpui::{
    App, Div, Entity, FocusHandle, FontWeight, Image, IntoElement, Role, ScrollHandle, SharedString, Toggled, Window,
    div, img, prelude::*, px,
};

use crate::style::{active_selection, color, cursor_ring, inactive_selection};
use crate::text_input::TextInput;

/// The label, description and enabled state every control row shares.
#[derive(Clone)]
pub struct ControlMeta {
    pub id: &'static str,
    pub label: &'static str,
    pub description: Option<SharedString>,
    pub enabled: bool,
}

/// The focus handle, focus state and theme every control draws with.
#[derive(Clone, Copy)]
pub struct ControlStyle<'a> {
    pub focus: &'a FocusHandle,
    pub focused: bool,
    pub theme: &'a Theme,
}

pub type Activate = Rc<dyn Fn(&mut Window, &mut App)>;
pub type Select = Rc<dyn Fn(usize, &mut Window, &mut App)>;

fn row(meta: ControlMeta, theme: &Theme, control: impl IntoElement) -> gpui::Stateful<Div> {
    let id = meta.id;
    div()
        .id((id, 0u64))
        .debug_selector(move || format!("{id}-row"))
        .flex()
        .flex_col()
        .gap(px(5.))
        .w_full()
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color(if meta.enabled { theme.role("fg") } else { theme.role("fg-muted") }))
                .child(meta.label),
        )
        .children(
            meta.description.map(|text| div().text_size(px(11.)).text_color(color(theme.role("fg-muted"))).child(text)),
        )
        .child(control)
}

/// A pill switch. Space toggles it; the caller binds the key.
pub fn switch(meta: ControlMeta, on: bool, style: ControlStyle, toggle: Activate) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let accent = color(theme.role("accent"));
    let raised = color(theme.role("surface-raised"));
    let knob = color(theme.role("bg"));
    let control_id = meta.id;
    let mut pill = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::Switch)
        .aria_label(meta.label)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .track_focus(focus)
        .h(px(24.))
        .w(px(44.))
        .flex_none()
        .rounded(px(12.))
        .flex()
        .items_center()
        .bg(if on { accent } else { raised })
        .when(focused, |pill| pill.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |pill| pill.opacity(0.45))
        .child(div().size(px(18.)).flex_none().rounded_full().bg(knob).ml(px(if on { 23. } else { 3. })));
    if meta.enabled {
        pill = pill.cursor_pointer().on_click(move |_, window, cx| toggle(window, cx));
    }
    row(meta, theme, pill)
}

/// A row of radio-button segments. ← → move and select; Space/Return select the
/// current one. The caller binds the keys and owns the value.
pub fn segmented(
    meta: ControlMeta,
    options: &[&'static str],
    selected: usize,
    style: ControlStyle,
    select: Select,
    trailing: Option<gpui::AnyElement>,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let control_id = meta.id;
    let group = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::RadioGroup)
        .aria_label(meta.label)
        .track_focus(focus)
        .flex()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(color(theme.role("surface")))
        .when(focused, |group| group.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |group| group.opacity(0.45))
        .children(options.iter().enumerate().map(|(index, label)| {
            let select = select.clone();
            let mut option = div()
                .id((meta.id, index))
                .role(Role::RadioButton)
                .aria_label(*label)
                .aria_selected(index == selected)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(options.len())
                .px(px(10.))
                .h(px(24.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .when(index == selected, |option| {
                    option.bg(color(theme.role("surface-raised"))).text_color(color(theme.role("fg")))
                })
                .when(index != selected, |option| option.text_color(color(theme.role("fg-muted"))))
                .child(*label);
            if meta.enabled {
                option = option.cursor_pointer().on_click(move |_, window, cx| select(index, window, cx));
            }
            option
        }));
    let control = match trailing {
        Some(trailing) => div().flex().items_center().gap(px(10.)).child(group).child(trailing),
        None => div().child(group),
    };
    row(meta, theme, control)
}

/// One image-and-label choice.
pub struct Tile {
    pub label: &'static str,
    pub image: std::sync::Arc<Image>,
}

/// Image tiles with radio semantics. ← → move and select.
pub fn tile_choice(
    meta: ControlMeta,
    tiles: &[Tile],
    selected: usize,
    style: ControlStyle,
    select: Select,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let control_id = meta.id;
    let group = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::RadioGroup)
        .aria_label(meta.label)
        .track_focus(focus)
        .flex()
        .items_start()
        .gap(px(8.))
        .p(px(2.))
        .rounded(px(8.))
        .when(focused, |group| group.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |group| group.opacity(0.45))
        .children(tiles.iter().enumerate().map(|(index, tile)| {
            let select = select.clone();
            let mut option = div()
                .id((meta.id, index))
                .role(Role::RadioButton)
                .aria_label(tile.label)
                .aria_selected(index == selected)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(tiles.len())
                .w(px(84.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(4.))
                .p(px(6.))
                .rounded(px(7.))
                .border_1()
                .border_color(color(if index == selected { theme.role("accent") } else { theme.role("border") }))
                .when(index != selected, |option| option.hover(|style| style.bg(color(theme.role("surface")))))
                .child(img(tile.image.clone()).size(px(64.)))
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(color(if index == selected { theme.role("fg") } else { theme.role("fg-muted") }))
                        .child(tile.label),
                );
            if meta.enabled {
                option = option.cursor_pointer().on_click(move |_, window, cx| select(index, window, cx));
            }
            option
        }));
    row(meta, theme, group)
}

/// A numeric field with a spin-button role. The caller owns the `TextInput` and
/// binds ↑ ↓ (and ⇧ for ±10); the field itself takes typed digits.
pub fn stepper(meta: ControlMeta, input: &Entity<TextInput>, style: ControlStyle) -> gpui::Stateful<Div> {
    let ControlStyle { focused, theme, .. } = style;
    let control_id = meta.id;
    let field = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .w(px(104.))
        .p(px(2.))
        .rounded_md()
        .when(focused, |field| field.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |field| field.opacity(0.45))
        .child(input.clone());
    row(meta, theme, field)
}

/// A scrollable list of named choices. ↑ ↓ move; typing selects by prefix. The
/// caller binds the keys, owns the cursor and decides when to preview or commit.
pub fn choice_list(
    meta: ControlMeta,
    options: &[String],
    selected: usize,
    style: ControlStyle,
    scroll: &ScrollHandle,
    select: Select,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let control_id = meta.id;
    let list = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::ListBox)
        .aria_label(meta.label)
        .track_focus(focus)
        .track_scroll(scroll)
        .max_h(px(160.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(1.))
        .p(px(2.))
        .rounded_md()
        .border_1()
        .border_color(color(theme.role("border")))
        .when(focused, |list| list.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |list| list.opacity(0.45))
        .children(options.iter().enumerate().map(|(index, label)| {
            let select = select.clone();
            let mut option = div()
                .id((meta.id, index))
                .role(Role::ListBoxOption)
                .aria_label(label.clone())
                .aria_selected(index == selected)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(options.len())
                .px_2()
                .py(px(3.))
                .rounded_md()
                .text_size(px(12.))
                .text_color(color(theme.role("fg")))
                .when(index == selected, |option| {
                    option.bg(if focused { active_selection(theme) } else { inactive_selection(theme) })
                })
                .child(label.clone());
            if meta.enabled {
                option = option.cursor_pointer().on_click(move |_, window, cx| select(index, window, cx));
            }
            option
        }));
    row(meta, theme, list)
}

/// A standard button with a focus ring. Space/Return activate it through the
/// caller's key handler; a click does the same.
pub fn button(
    id: &'static str,
    label: &'static str,
    primary: bool,
    enabled: bool,
    style: ControlStyle,
    activate: Activate,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let mut button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .track_focus(focus)
        .px(px(10.))
        .h(px(26.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(color(if primary { theme.role("accent") } else { theme.role("border") }))
        .text_size(px(12.))
        .text_color(color(if primary { theme.role("on-accent") } else { theme.role("fg") }))
        .when(primary, |button| button.bg(color(theme.role("accent"))))
        .when(!primary, |button| button.bg(color(theme.role("surface"))))
        .when(focused, |button| button.shadow(cursor_ring(theme)))
        .when(!enabled, |button| button.opacity(0.45))
        .child(label);
    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(color(if primary { theme.role("accent") } else { theme.role("surface-raised") })))
            .on_click(move |_, window, cx| activate(window, cx));
    }
    button
}

/// A button that reads as a link, for opening one of the other config files.
pub fn link_button(
    id: &'static str,
    label: &'static str,
    enabled: bool,
    style: ControlStyle,
    activate: Activate,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme } = style;
    let mut button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(SharedString::from(format!("Open {label}")))
        .track_focus(focus)
        .px(px(2.))
        .py(px(1.))
        .rounded(px(3.))
        .text_size(px(11.))
        .text_color(color(theme.role("accent")))
        .when(focused, |button| button.shadow(cursor_ring(theme)))
        .when(!enabled, |button| button.opacity(0.45))
        .child(label);
    if enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.text_color(color(theme.role("fg"))))
            .on_click(move |_, window, cx| activate(window, cx));
    }
    button
}
