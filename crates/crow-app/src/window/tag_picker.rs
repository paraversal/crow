//! The `/` filter's tag picker: a compact multi-select sheet. Every checked tag
//! must be present on a row; the name filter still applies on top.

use gpui::{Context, FocusHandle, FontWeight, IntoElement, Role, Toggled, Window, div, prelude::*, px};

use crate::actions::{SheetCancel, SheetConfirm};
use crate::chrome::{self, SheetWidth};
use crate::icons::{Glyph, glyph};
use crate::style::{active_selection, color};
use crate::ui::Ui;

use super::CrowWindow;

pub(super) struct TagPicker {
    pub(super) focus: FocusHandle,
    selected: Vec<String>,
    cursor: usize,
}

impl CrowWindow {
    pub(super) fn open_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() || self.tag_picker.is_some() {
            return;
        }
        let selected = self.pane().read(cx).folder_tag_filter(cx);
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.tag_picker = Some(TagPicker { focus, selected, cursor: 0 });
        cx.notify();
    }

    fn close_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tag_picker = None;
        if let Some(focus) = self.pane().read(cx).filter_focus(cx) {
            window.focus(&focus, cx);
        } else {
            self.focus_center(window, cx);
        }
        cx.notify();
    }

    fn apply_tag_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.tag_picker.take() else { return };
        self.pane().update(cx, |pane, cx| pane.set_folder_tag_filter(picker.selected, cx));
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
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .rounded_md()
                .px_2()
                .py_1()
                .bg(color(theme.role("surface-raised")))
                .child(label)
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
                .on_action(cx.listener(|this, _: &SheetConfirm, window, cx| this.apply_tag_picker(window, cx)))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    let Some(picker) = &mut this.tag_picker else { return };
                    match event.keystroke.key.as_str() {
                        "up" => picker.cursor = picker.cursor.saturating_sub(1),
                        "down" => picker.cursor = (picker.cursor + 1).min(limit),
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
                        .child(button("tags-clear", "Clear").on_click(cx.listener(|this, _, window, cx| {
                            if let Some(picker) = &mut this.tag_picker {
                                picker.selected.clear();
                                this.apply_tag_picker(window, cx);
                            }
                        })))
                        .child(div().flex_1())
                        .child(
                            button("tags-cancel", "Cancel")
                                .on_click(cx.listener(|this, _, window, cx| this.close_tag_picker(window, cx))),
                        )
                        .child(
                            button("tags-done", "Done")
                                .on_click(cx.listener(|this, _, window, cx| this.apply_tag_picker(window, cx))),
                        ),
                ),
            &theme,
            mode,
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
