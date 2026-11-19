//! The List column chooser, shared by the header and View menu.

use std::path::PathBuf;

use gpui::{Context, FocusHandle, FontWeight, IntoElement, Role, Toggled, Window, div, prelude::*, px};

use crate::actions::{ChooseListColumns, SheetCancel, SheetConfirm};
use crate::chrome::{self, SheetWidth};
use crate::extensions::Ext;
use crate::style::{active_selection, color};
use crate::ui::{self, Ui};

use super::CrowWindow;

pub(super) struct ColumnPicker {
    pub(super) focus: FocusHandle,
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
        if self.search.is_some() || self.pane().read(cx).mode() != crow_core::ViewMode::List {
            return;
        }
        let folder = self.pane().read(cx).current_dir(cx);
        let columns = cx.global::<Ui>().views.resolve(&folder).columns.0;
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        self.column_picker = Some(ColumnPicker { focus, folder, columns, cursor: 0 });
        cx.notify();
    }

    fn close_column_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.column_picker = None;
        self.focus_center(window, cx);
        cx.notify();
    }

    fn save_columns(&mut self, defaults: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = self.column_picker.take() else { return };
        if defaults {
            ui::save_default_columns(picker.folder, picker.columns, cx);
        } else {
            ui::save_view_change(picker.folder, crow_core::ViewChange::Columns(picker.columns), cx);
        }
        self.close_column_picker(window, cx);
    }

    pub(super) fn render_column_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
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
                    .role(Role::CheckBox)
                    .aria_label(title.clone())
                    .aria_toggled(if checked { Toggled::True } else { Toggled::False })
                    .px_2()
                    .py_1()
                    .rounded_sm()
                    .flex()
                    .gap_2()
                    .when(picker.cursor == index, |row| row.bg(active_selection(&theme)))
                    .child(div().w(px(17.)).child(if checked { "✓" } else { "" }))
                    .child(title)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(picker) = &mut this.column_picker {
                            picker.cursor = index;
                            toggle(&mut picker.columns, &id);
                            cx.notify();
                        }
                    }))
            })
            .collect::<Vec<_>>();
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .role(Role::Button)
                .aria_label(label)
                .rounded_md()
                .px_2()
                .py_1()
                .bg(color(theme.role("surface-raised")))
                .child(label)
        };
        Some(chrome::sheet(
            chrome::sheet_panel("list-columns-picker", SheetWidth::Compact, &theme)
                .aria_label("List Columns")
                .key_context("Sheet")
                .track_focus(&picker.focus)
                .on_action(cx.listener(|this, _: &SheetCancel, window, cx| this.close_column_picker(window, cx)))
                .on_action(cx.listener(|this, _: &SheetConfirm, window, cx| this.save_columns(false, window, cx)))
                .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                    let Some(picker) = &mut this.column_picker else { return };
                    match event.keystroke.key.as_str() {
                        "up" => picker.cursor = picker.cursor.saturating_sub(1),
                        "down" => picker.cursor = (picker.cursor + 1).min(limit),
                        "space" => {
                            if let Some(id) = &selected {
                                toggle(&mut picker.columns, id);
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
                .child(div().font_weight(FontWeight::SEMIBOLD).child("List Columns"))
                .child(
                    div()
                        .id("column-choices")
                        .role(Role::Group)
                        .aria_label("Available columns")
                        .max_h(px(218.))
                        .overflow_y_scroll()
                        .children(options),
                )
                .child(
                    div()
                        .flex()
                        .gap_2()
                        .child(
                            button("columns-default", "Save as Default")
                                .on_click(cx.listener(|this, _, window, cx| this.save_columns(true, window, cx))),
                        )
                        .child(div().flex_1())
                        .child(
                            button("columns-cancel", "Cancel")
                                .on_click(cx.listener(|this, _, window, cx| this.close_column_picker(window, cx))),
                        )
                        .child(
                            button("columns-done", "Done")
                                .on_click(cx.listener(|this, _, window, cx| this.save_columns(false, window, cx))),
                        ),
                ),
            &theme,
            mode,
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
