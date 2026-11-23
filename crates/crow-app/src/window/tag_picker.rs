//! The `/` filter's tag picker: a compact multi-select sheet. Every checked tag
//! must be present on a row; the name filter still applies on top.

use gpui::{Context, FocusHandle, FontWeight, IntoElement, Role, Toggled, Window, div, prelude::*, px};

use crate::actions::{SheetCancel, SheetConfirm};
use crate::chrome::{self, SheetWidth};
use crate::icons::{Glyph, glyph};
use crate::style::{active_selection, color};
use crate::ui::Ui;

use super::CrowWindow;

impl TagPicker {
    pub(super) fn buttons(&self) -> &[FocusHandle] {
        &self.buttons
    }
}

pub(super) struct TagPicker {
    pub(super) focus: FocusHandle,
    selected: Vec<String>,
    cursor: usize,
    /// Clear, Cancel and Done, in Tab order.
    pub(super) buttons: [FocusHandle; 3],
}

impl CrowWindow {
    pub(super) fn open_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() || self.overlay_open() {
            return;
        }
        crate::tags::refresh_if_stale(cx);
        let selected = self.pane().read(cx).folder_tag_filter(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.tag_picker = Some(TagPicker {
            focus,
            selected,
            cursor: 0,
            buttons: [cx.focus_handle(), cx.focus_handle(), cx.focus_handle()],
        });
        cx.notify();
    }

    fn close_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        retire!(self, cx, tag_picker);
        if let Some(focus) = self.pane().read(cx).filter_focus(cx) {
            window.focus(&focus, cx);
        } else {
            self.focus_center(window, cx);
        }
        cx.notify();
    }

    /// Return or Space: the focused button, else Done.
    fn press_tag_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.tag_picker else { return };
        match picker.buttons.iter().position(|button| button.is_focused(window)) {
            Some(0) => {
                picker.selected.clear();
                self.apply_tag_picker(window, cx);
            }
            Some(1) => self.close_tag_picker(window, cx),
            _ => self.apply_tag_picker(window, cx),
        }
    }

    fn apply_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &self.tag_picker else { return };
        let selected = picker.selected.clone();
        retire!(self, cx, tag_picker);
        self.pane().update(cx, |pane, cx| pane.set_folder_tag_filter(selected, cx));
        if let Some(focus) = self.pane().read(cx).filter_focus(cx) {
            window.focus(&focus, cx);
        } else {
            self.focus_center(window, cx);
        }
        cx.notify();
    }

    pub(super) fn render_tag_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let picker = self.tag_picker.as_ref()?;
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let mode = ui.feedback.mode;
        let mut choices = ui.tags.all();
        for selected in &picker.selected {
            if !choices.iter().any(|choice| crow_core::normalize_tag(choice) == *selected) {
                choices.push(selected.clone());
            }
        }
        let limit = choices.len().saturating_sub(1);
        let selected = choices.get(picker.cursor).cloned();
        let options = choices
            .iter()
            .enumerate()
            .map(|(index, tag)| {
                let tag = tag.clone();
                let checked = picker.selected.contains(&crow_core::normalize_tag(&tag));
                div()
                    .id(("tag-toggle", index))
                    .debug_selector(move || format!("tag-toggle-{index}"))
                    .role(Role::CheckBox)
                    .aria_label(tag.clone())
                    .aria_toggled(if checked { Toggled::True } else { Toggled::False })
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(choices.len())
                    .when(picker.cursor == index, |row| row.aria_active_descendant())
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(picker.cursor == index, |row| row.bg(active_selection(&theme)))
                    .child(glyph(Glyph::Tag, 14., if checked { "accent" } else { "fg-muted" }, &theme))
                    .child(div().flex_1().min_w_0().child(tag.clone()))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(color(theme.role("accent")))
                            .child(if checked { "✓" } else { "" }),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(picker) = &mut this.tag_picker {
                            picker.cursor = index;
                            toggle(&mut picker.selected, &tag);
                            cx.notify();
                        }
                    }))
            })
            .collect::<Vec<_>>();
        let button = |id: &'static str, label: &'static str, focus: &FocusHandle| {
            crate::chrome::sheet_button(id, label, false, &theme)
                .px_2()
                .track_focus(focus)
                .tab_stop(true)
                .focus_visible(|style| style.shadow(crate::style::cursor_ring(&theme)))
        };
        let empty = choices.is_empty().then(|| {
            div()
                .p_2()
                .text_size(px(12.))
                .text_color(color(theme.role("fg-muted")))
                .child("No indexed tags yet. Add pinned_tags in sidebar.toml.")
        });
        Some(chrome::sheet(
            chrome::sheet_panel("tag-picker", SheetWidth::Compact, &theme)
                .aria_label("Filter by Tag")
                .key_context("Sheet")
                .track_focus(&picker.focus)
                .on_action(cx.listener(|this, _: &SheetCancel, window, cx| this.close_tag_picker(window, cx)))
                .on_action(cx.listener(|this, _: &SheetConfirm, window, cx| this.press_tag_button(window, cx)))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    let Some(picker) = &mut this.tag_picker else { return };
                    let focused = picker.buttons.iter().position(|button| button.is_focused(window));
                    match event.keystroke.key.as_str() {
                        "tab" => {
                            let forward = !event.keystroke.modifiers.shift;
                            chrome::cycle_focus(&picker.buttons, forward, window, cx);
                        }
                        "up" => picker.cursor = picker.cursor.saturating_sub(1),
                        "down" => picker.cursor = (picker.cursor + 1).min(limit),
                        // Space presses a focused button; otherwise it toggles the row under the cursor.
                        "space" if focused.is_some() => {
                            this.press_tag_button(window, cx);
                        }
                        "space" => {
                            if let Some(tag) = &selected {
                                toggle(&mut picker.selected, tag);
                            }
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    cx.notify();
                }))
                .p_4()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Filter by Tag"))
                .child(
                    div()
                        .id("tag-choices")
                        .role(Role::Group)
                        .aria_label("Finder tags")
                        .max_h(px(218.))
                        .overflow_y_scroll()
                        .children(empty)
                        .children(options),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(button("tags-clear", "Clear", &picker.buttons[0]).on_click(cx.listener(
                            |this, _, window, cx| {
                                if let Some(picker) = &mut this.tag_picker {
                                    picker.selected.clear();
                                    this.apply_tag_picker(window, cx);
                                }
                            },
                        )))
                        .child(div().flex_1())
                        .child(
                            button("tags-cancel", "Cancel", &picker.buttons[1])
                                .on_click(cx.listener(|this, _, window, cx| this.close_tag_picker(window, cx))),
                        )
                        .child(
                            button("tags-done", "Done", &picker.buttons[2])
                                .on_click(cx.listener(|this, _, window, cx| this.apply_tag_picker(window, cx))),
                        ),
                ),
            &theme,
            mode,
            self.exit_fade.get(),
            cx.listener(|this, _, window, cx| this.close_tag_picker(window, cx)),
        ))
    }
}

fn toggle(selected: &mut Vec<String>, tag: &str) {
    let tag = crow_core::normalize_tag(tag);
    if let Some(index) = selected.iter().position(|existing| existing == &tag) {
        selected.remove(index);
    } else {
        selected.push(tag);
    }
}
