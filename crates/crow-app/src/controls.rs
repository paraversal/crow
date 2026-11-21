//! Shared settings controls: a switch, segmented choices, image tiles, a numeric
//! stepper and a type-to-select drop-down. Every control draws with theme roles only,
//! carries a visible label that is also its aria label, and draws the sidebar's
//! inset cursor ring when focused (so focus never resizes it).

use std::rc::Rc;

use std::cell::RefCell;

use crow_core::motion::Effective;
use crow_theme::Theme;
use gpui::{
    AnimationExt as _, App, Bounds, Div, Entity, FocusHandle, FontWeight, Image, IntoElement, Role, ScrollHandle,
    SharedString, Toggled, Window, anchored, deferred, div, img, prelude::*, px,
};

use crate::icons::{Glyph, glyph};
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
    pub mode: Effective,
}

pub type Activate = Rc<dyn Fn(&mut Window, &mut App)>;
pub type Select = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// A labelled row around a caller-built control, for composite settings such
/// as the config-file buttons and the sort key with its direction.
pub fn labeled(meta: ControlMeta, theme: &Theme, control: impl IntoElement) -> gpui::Stateful<Div> {
    row(meta, theme, control)
}

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
                .text_size(px(14.))
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
    let ControlStyle { focus, focused, theme, mode } = style;
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
        .aria_disabled(!meta.enabled)
        .track_focus(focus)
        .h(px(24.))
        .w(px(44.))
        .flex_none()
        .rounded(px(12.))
        .flex()
        .items_center()
        .when(focused, |pill| pill.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |pill| pill.opacity(0.45));
    // Full slides the knob and fades the accent in (a spring, so a second click
    // reverses smoothly); Low only fades the colour; Off swaps at once.
    let knob_x = if on { 23. } else { 3. };
    let knob_el = div().size(px(18.)).flex_none().rounded_full().bg(knob);
    let knob_el: gpui::AnyElement = match mode {
        Effective::Full => knob_el
            .with_spring(
                SharedString::from(format!("{control_id}-knob")),
                crate::motion_ui::glide(px(knob_x)),
                |el, x| el.ml(x),
            )
            .into_any_element(),
        _ => knob_el.ml(px(knob_x)).into_any_element(),
    };
    let tint = div().absolute().inset_0().rounded(px(12.)).bg(accent);
    let tint: gpui::AnyElement = match mode {
        Effective::Off => tint.opacity(if on { 1.0 } else { 0.0 }).into_any_element(),
        _ => tint
            .with_spring(
                SharedString::from(format!("{control_id}-tint")),
                crate::motion_ui::glide(if on { 1.0f32 } else { 0.0 }),
                |el, alpha| el.opacity(alpha),
            )
            .into_any_element(),
    };
    pill = pill.relative().bg(raised).child(tint).child(knob_el);
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
    let ControlStyle { focus, focused, theme, mode } = style;
    let control_id = meta.id;
    let glide_fill = mode == Effective::Full && meta.enabled;
    let recorded: Rc<RefCell<Vec<Bounds<gpui::Pixels>>>> = Rc::default();
    let group = div()
        .when(glide_fill, |group| {
            let recorded = recorded.clone();
            group.on_children_prepainted(move |bounds, _, _| {
                *recorded.borrow_mut() = bounds.into_iter().skip(1).collect();
            })
        })
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::RadioGroup)
        .aria_label(meta.label)
        .aria_disabled(!meta.enabled)
        .track_focus(focus)
        .flex()
        .items_center()
        .p(px(2.))
        .gap(px(2.))
        .rounded(px(7.))
        .bg(color(theme.role("surface")))
        .when(focused, |group| group.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |group| group.opacity(0.45))
        .relative()
        .children(glide_fill.then(|| {
            // Full: the selected segment's fill glides between the segments (a spring on
            // the segment index), painted under their labels.
            let fill = color(theme.role("surface-raised"));
            let recorded = recorded.clone();
            div().absolute().size(px(0.)).with_spring(
                SharedString::from(format!("{control_id}-segment")),
                crate::motion_ui::glide(selected as f32),
                move |el, position| {
                    el.child(
                        gpui::canvas(
                            |_, _, _| (),
                            move |_, (), window, _| {
                                let boxes = recorded.borrow();
                                let position = position.clamp(0.0, boxes.len().saturating_sub(1) as f32);
                                let (low, high) = (position.floor() as usize, position.ceil() as usize);
                                let (Some(from), Some(to)) = (boxes.get(low), boxes.get(high)) else { return };
                                let t = position.fract();
                                let left = from.left() + (to.left() - from.left()) * t;
                                let width = from.size.width + (to.size.width - from.size.width) * t;
                                let area =
                                    Bounds::new(gpui::point(left, from.top()), gpui::size(width, from.size.height));
                                window.paint_quad(gpui::quad(
                                    area,
                                    px(5.),
                                    fill,
                                    gpui::Edges::default(),
                                    gpui::transparent_black(),
                                    gpui::BorderStyle::default(),
                                ));
                            },
                        )
                        .absolute()
                        .inset_0(),
                    )
                },
            )
        }))
        .children(options.iter().enumerate().map(|(index, label)| {
            let select = select.clone();
            let mut option = div()
                .id((meta.id, index))
                .role(Role::RadioButton)
                .aria_label(*label)
                .aria_disabled(!meta.enabled)
                .aria_selected(index == selected)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(options.len())
                .px(px(10.))
                .h(px(24.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(12.))
                .when(index == selected && !glide_fill, |option| option.bg(color(theme.role("surface-raised"))))
                .when(index == selected, |option| option.text_color(color(theme.role("fg"))))
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
    let ControlStyle { focus, focused, theme, .. } = style;
    let control_id = meta.id;
    let group = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::RadioGroup)
        .aria_label(meta.label)
        .aria_disabled(!meta.enabled)
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
                .aria_disabled(!meta.enabled)
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

/// A single-line text field. The caller owns the `TextInput` and commits on
/// Return or when focus leaves; `width` keeps it apart from the numeric steppers.
pub fn text_field(
    meta: ControlMeta,
    input: &Entity<TextInput>,
    style: ControlStyle,
    width: f32,
) -> gpui::Stateful<Div> {
    let ControlStyle { focused, theme, .. } = style;
    let control_id = meta.id;
    let field = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .w(px(width))
        .p(px(2.))
        .rounded_md()
        .when(focused, |field| field.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |field| field.opacity(0.45))
        .child(input.clone());
    row(meta, theme, field)
}

/// An open drop-down's menu: the highlighted entry, its scroll position and
/// what a click outside the menu does.
pub struct Menu<'a> {
    pub cursor: usize,
    pub scroll: &'a ScrollHandle,
    pub dismiss: Activate,
}

/// A pop-up button showing the current choice; when `menu` is given, its list
/// opens under it. The caller binds the keys (Space, Return, ↑ ↓ open it; ↑ ↓
/// move; Return keeps; Esc reverts) and owns the value and the cursor.
pub fn dropdown(
    meta: ControlMeta,
    options: &[String],
    selected: usize,
    menu: Option<Menu>,
    style: ControlStyle,
    toggle: Activate,
    select: Select,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme, .. } = style;
    let control_id = meta.id;
    let value = options.get(selected).cloned().unwrap_or_default();
    let mut button = div()
        .id(control_id)
        .debug_selector(move || format!("{control_id}-control"))
        .role(Role::ComboBox)
        .aria_label(meta.label)
        .aria_value(value.clone())
        .aria_expanded(menu.is_some())
        .aria_disabled(!meta.enabled)
        .track_focus(focus)
        .w(px(DROPDOWN_WIDTH))
        .h(px(26.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(color(theme.role("surface")))
        .text_size(px(12.))
        .text_color(color(theme.role("fg")))
        .when(focused, |button| button.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |button| button.opacity(0.45))
        .child(div().flex_1().min_w_0().truncate().child(value))
        .child(glyph(Glyph::ChevronDown, 12., "fg-muted", theme));
    if meta.enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(color(theme.role("surface-raised"))))
            .on_click(move |_, window, cx| toggle(window, cx));
    }
    let popup = menu.filter(|_| meta.enabled).map(|menu| {
        let Menu { cursor, scroll, dismiss } = menu;
        let list = div()
            .id((control_id, u64::MAX))
            .debug_selector(move || format!("{control_id}-menu"))
            .role(Role::ListBox)
            .aria_label(meta.label)
            .occlude()
            .track_scroll(scroll)
            .overflow_y_scroll()
            .w(px(DROPDOWN_WIDTH))
            .max_h(px(280.))
            .mt(px(4.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(6.))
            .bg(color(theme.role("surface-raised")))
            .shadow_lg()
            .on_mouse_down_out(move |_, window, cx| dismiss(window, cx))
            .children(options.iter().enumerate().map(|(index, label)| {
                let select = select.clone();
                div()
                    .id((control_id, index))
                    .debug_selector(move || format!("{control_id}-option-{index}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .aria_selected(index == cursor)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(options.len())
                    // `truncate` drops the row's minimum height; without
                    // `flex_none` a long menu squashes its rows instead of scrolling.
                    .flex_none()
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg")))
                    .truncate()
                    .cursor_pointer()
                    .when(index == cursor, |option| option.bg(active_selection(theme)))
                    .when(index != cursor, |option| option.hover(|style| style.bg(inactive_selection(theme))))
                    .on_click(move |_, window, cx| select(index, window, cx))
                    .child(label.clone())
            }));
        // An absolute element sits at its parent's origin, so the anchor is an
        // empty in-flow div right under the button.
        div().h_0().child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(list)).with_priority(1))
    });
    row(meta, theme, div().flex().flex_col().child(button).children(popup))
}

const DROPDOWN_WIDTH: f32 = 260.;

/// A button that opens a list of choices, such as "Add Provider". The list uses
/// the same anchored popup as a drop-down but the closed button keeps its own
/// label instead of a selected value.
#[allow(clippy::too_many_arguments)]
pub fn menu_button(
    meta: ControlMeta,
    label: &'static str,
    options: &[String],
    menu: Option<Menu>,
    style: ControlStyle,
    toggle: Activate,
    select: Select,
) -> gpui::Stateful<Div> {
    let ControlStyle { focus, focused, theme, .. } = style;
    let control_id = meta.id;
    let open = menu.is_some();
    let mut button = div()
        .id(control_id)
        .debug_selector(move || control_id.to_owned())
        .role(Role::Button)
        .aria_label(meta.label)
        .aria_expanded(open)
        .aria_disabled(!meta.enabled)
        .track_focus(focus)
        .px(px(10.))
        .h(px(26.))
        .min_w(px(130.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .rounded(px(6.))
        .border_1()
        .border_color(color(theme.role("border")))
        .bg(color(theme.role("surface")))
        .text_size(px(12.))
        .text_color(color(theme.role("fg")))
        .when(focused, |button| button.shadow(cursor_ring(theme)))
        .when(!meta.enabled, |button| button.opacity(0.45))
        .child(label)
        .child(glyph(Glyph::ChevronDown, 12., "fg-muted", theme));
    if meta.enabled {
        button = button
            .cursor_pointer()
            .hover(|style| style.bg(color(theme.role("surface-raised"))))
            .on_click(move |_, window, cx| toggle(window, cx));
    }
    let popup = menu.map(|Menu { cursor, scroll, dismiss }| {
        let list = div()
            .id((control_id, u64::MAX))
            .debug_selector(move || format!("{control_id}-menu"))
            .role(Role::ListBox)
            .aria_label(meta.label)
            .occlude()
            .track_scroll(scroll)
            .overflow_y_scroll()
            .w(px(DROPDOWN_WIDTH))
            .max_h(px(280.))
            .mt(px(4.))
            .p(px(4.))
            .flex()
            .flex_col()
            .rounded(px(6.))
            .bg(color(theme.role("surface-raised")))
            .shadow_lg()
            .on_mouse_down_out(move |_, window, cx| dismiss(window, cx))
            .children(options.iter().enumerate().map(|(index, label)| {
                let select = select.clone();
                div()
                    .id((control_id, index))
                    .debug_selector(move || format!("{control_id}-option-{index}"))
                    .role(Role::ListBoxOption)
                    .aria_label(label.clone())
                    .aria_selected(index == cursor)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(options.len())
                    .flex_none()
                    .px_2()
                    .py(px(3.))
                    .rounded(px(4.))
                    .text_size(px(12.))
                    .text_color(color(theme.role("fg")))
                    .truncate()
                    .cursor_pointer()
                    .when(index == cursor, |option| option.bg(active_selection(theme)))
                    .when(index != cursor, |option| option.hover(|style| style.bg(inactive_selection(theme))))
                    .on_click(move |_, window, cx| select(index, window, cx))
                    .child(label.clone())
            }));
        div().h_0().child(deferred(anchored().snap_to_window_with_margin(px(8.)).child(list)).with_priority(1))
    });
    row(meta, theme, div().flex().flex_col().child(button).children(popup))
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
    let ControlStyle { focus, focused, theme, .. } = style;
    let mut button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(label)
        .aria_disabled(!enabled)
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
    let ControlStyle { focus, focused, theme, .. } = style;
    let mut button = div()
        .id(id)
        .debug_selector(move || id.into())
        .role(Role::Button)
        .aria_label(SharedString::from(format!("Open {label}")))
        .aria_disabled(!enabled)
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
