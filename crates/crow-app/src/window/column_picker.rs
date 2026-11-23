//! The List column chooser, shared by the header and View menu.

use std::path::PathBuf;

use gpui::{Context, FocusHandle, FontWeight, IntoElement, Role, ScrollHandle, Toggled, Window, div, prelude::*, px};

use crate::actions::{ChooseListColumns, SheetCancel, SheetConfirm};
use crate::chrome::{self, SheetWidth};
use crate::extensions::Ext;
use crate::style::{active_selection, cursor_ring};
use crate::ui::{self, Ui};

use super::CrowWindow;

pub(super) struct ColumnPicker {
    pub(super) focus: FocusHandle,
    pub(super) buttons: [FocusHandle; 3],
    scroll: ScrollHandle,
    folder: PathBuf,
    columns: Vec<String>,
    cursor: usize,
}

#[cfg(test)]
impl ColumnPicker {
    pub(super) fn columns_for_test(&self) -> &[String] {
        &self.columns
    }
}

fn available(cx: &gpui::App) -> Vec<(String, String)> {
    let mut choices: Vec<_> =
        cx.global::<Ext>().0.columns().iter().map(|c| (c.id().into(), c.title().into())).collect();
    if let Some(plugins) = cx.try_global::<crate::plugins::Plugins>() {
        choices.extend(
            plugins
                .surfaces
                .snapshot()
                .of_kind(crow_core::ExtensionKind::Column)
                .filter(|(id, _)| !plugins.surfaces.is_suppressed(&id.plugin))
                .map(|(id, column)| (id.to_string(), column.descriptor.title.clone())),
        );
    }
    choices
}

impl CrowWindow {
    pub(super) fn choose_list_columns(&mut self, _: &ChooseListColumns, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() || self.overlay_open() || self.pane().read(cx).mode() != crow_core::ViewMode::List {
            return;
        }
        let Some(folder) = self.pane().read(cx).focused_path(cx) else { return };
        let columns = cx.global::<Ui>().views.resolve(&folder).columns.0;
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.column_picker = Some(ColumnPicker {
            focus,
            buttons: std::array::from_fn(|_| cx.focus_handle()),
            scroll: ScrollHandle::new(),
            folder,
            columns,
            cursor: 0,
        });
        cx.notify();
    }

    fn close_column_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        retire!(self, cx, column_picker);
        self.focus_center(window, cx);
        cx.notify();
    }

    fn save_columns(&mut self, defaults: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &self.column_picker else { return };
        let (folder, columns) = (picker.folder.clone(), picker.columns.clone());
        if defaults {
            ui::save_default_columns(folder, columns, cx);
        } else {
            ui::save_view_change(folder, crow_core::ViewChange::Columns(columns), cx);
        }
        self.close_column_picker(window, cx);
    }

    fn activate_column_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &self.column_picker else { return };
        if picker.buttons[1].is_focused(window) {
            self.close_column_picker(window, cx);
        } else {
            self.save_columns(picker.buttons[0].is_focused(window), window, cx);
        }
    }

    pub(super) fn render_column_picker(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement + use<>> {
        let picker = self.column_picker.as_ref()?;
        let choices = available(cx);
        let ui = cx.global::<Ui>();
        let theme = ui.theme.clone();
        let mode = ui.feedback.mode;
        let limit = choices.len().saturating_sub(1);
        let selected = choices.get(picker.cursor).map(|(id, _)| id.clone());
        let options = choices
            .into_iter()
            .enumerate()
            .map(|(index, (id, title))| {
                let checked = picker.columns.contains(&id);
                div()
                    .id(("column-toggle", index))
                    .debug_selector(move || format!("column-toggle-{index}"))
                    .role(Role::CheckBox)
                    .aria_label(title.clone())
                    .aria_toggled(if checked { Toggled::True } else { Toggled::False })
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .flex()
                    .gap_2()
                    .when(picker.cursor == index && picker.focus.is_focused(window), |row| {
                        row.bg(active_selection(&theme))
                    })
                    .child(div().w(px(17.)).child(if checked { "✓" } else { "" }))
                    .child(title)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(picker) = &mut this.column_picker {
                            window.focus(&picker.focus, cx);
                            picker.cursor = index;
                            toggle(&mut picker.columns, &id);
                            cx.notify();
                        }
                    }))
            })
            .collect::<Vec<_>>();
        let button = |index: usize, id: &'static str, label: &'static str| {
            crate::chrome::sheet_button(id, label, false, &theme)
                .px_2()
                .track_focus(&picker.buttons[index])
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
        };
        Some(chrome::sheet(
            chrome::sheet_panel("list-columns-picker", SheetWidth::Compact, &theme)
                .aria_label("List Columns")
                .key_context("Sheet")
                .track_focus(&picker.focus)
                .on_action(cx.listener(|this, _: &SheetCancel, window, cx| this.close_column_picker(window, cx)))
                .on_action(cx.listener(|this, _: &SheetConfirm, window, cx| this.activate_column_picker(window, cx)))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                    let Some(picker) = &mut this.column_picker else { return };
                    let modifiers = event.keystroke.modifiers;
                    match event.keystroke.key.as_str() {
                        "tab" if !modifiers.control && !modifiers.alt && !modifiers.platform => {
                            let controls = [
                                picker.focus.clone(),
                                picker.buttons[0].clone(),
                                picker.buttons[1].clone(),
                                picker.buttons[2].clone(),
                            ];
                            chrome::cycle_focus(&controls, !modifiers.shift, window, cx);
                        }
                        "space" if !modifiers.modified() && !picker.focus.is_focused(window) => {
                            this.activate_column_picker(window, cx);
                        }
                        "space" if !modifiers.modified() => {
                            if let Some(id) = &selected {
                                toggle(&mut picker.columns, id);
                            }
                        }
                        "up" | "down" if !modifiers.modified() && picker.focus.is_focused(window) => {
                            picker.cursor = if event.keystroke.key == "up" {
                                picker.cursor.saturating_sub(1)
                            } else {
                                (picker.cursor + 1).min(limit)
                            };
                            picker.scroll.scroll_to_item(picker.cursor);
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
                .child(div().font_weight(FontWeight::SEMIBOLD).child("List Columns"))
                .child(
                    div()
                        .id("column-choices")
                        .role(Role::Group)
                        .aria_label("Available columns")
                        .max_h(px(218.))
                        .overflow_y_scroll()
                        .track_scroll(&picker.scroll)
                        .children(options),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            button(0, "columns-default", "Save as Default")
                                .on_click(cx.listener(|this, _, window, cx| this.save_columns(true, window, cx))),
                        )
                        .child(div().flex_1())
                        .child(
                            button(1, "columns-cancel", "Cancel")
                                .on_click(cx.listener(|this, _, window, cx| this.close_column_picker(window, cx))),
                        )
                        .child(
                            button(2, "columns-done", "Done")
                                .on_click(cx.listener(|this, _, window, cx| this.save_columns(false, window, cx))),
                        ),
                ),
            &theme,
            mode,
            self.exit_fade.get(),
            cx.listener(|this, _, window, cx| this.close_column_picker(window, cx)),
        ))
    }
}

fn toggle(columns: &mut Vec<String>, id: &str) {
    if let Some(index) = columns.iter().position(|existing| existing == id) {
        columns.remove(index);
    } else {
        columns.push(id.to_owned());
    }
}

pub(crate) fn open(window: &mut Window, cx: &mut gpui::App) {
    if let Some(root) = window.root::<CrowWindow>().flatten() {
        root.update(cx, |this, cx| this.choose_list_columns(&ChooseListColumns, window, cx));
    }
}
