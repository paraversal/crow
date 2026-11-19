//! The batch-rename sheet: one rule at a time, a live old → new preview, and
//! submission of an immutable approved mapping. The pure rule lives in
//! `crow_core::batch_rename`; filesystem checks and the commit live in
//! `crow_fs::ops` off the main thread.

use super::*;
use crow_core::batch_rename::{AddPosition, Preview, RenameForm, RenameInput, RenameKind, RowState, preview};
use crow_fs::ops::{BatchRenameEntry, BatchRenameInput, BatchRowState, BatchValidation, validate_batch_rename};
use gpui::{AnyElement, UniformListScrollHandle, uniform_list};

pub(crate) struct BatchRenameSheet {
    pub(crate) focus: FocusHandle,
    pub(crate) find: Entity<TextInput>,
    pub(crate) replace: Entity<TextInput>,
    pub(crate) add: Entity<TextInput>,
    pub(crate) base: Entity<TextInput>,
    pub(crate) start: Entity<TextInput>,
    pub(crate) padding: Entity<TextInput>,
    pub(crate) kind: RenameKind,
    pub(crate) add_position: AddPosition,
    pub(crate) preserve_extension: bool,
    pub(crate) preview: Preview,
    pub(crate) checks: Option<Arc<[BatchRowState]>>,
    pub(crate) validation_error: Option<SharedString>,
    pub(crate) validating: bool,
    pub(crate) revision: u64,
    pub(crate) submit: Option<Arc<[BatchRenameEntry]>>,
    preview_scroll: UniformListScrollHandle,
    rules: [FocusHandle; 3],
    sides: [FocusHandle; 2],
    extension: FocusHandle,
    cancel: FocusHandle,
    rename: FocusHandle,
    snapshot: Arc<[RenameInput]>,
    pub(crate) validation_cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) subscriptions: Vec<Subscription>,
}

fn field(value: String, placeholder: &'static str, aria: &'static str, cx: &mut App) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(value, placeholder, cx).bare().with_aria_label(aria))
}

impl BatchRenameSheet {
    pub(crate) fn new(snapshot: Arc<[RenameInput]>, cx: &mut App) -> Self {
        let focus = cx.focus_handle();
        Self {
            focus,
            find: field(String::new(), "Find", "Find text", cx),
            replace: field(String::new(), "Replace with", "Replace with text", cx),
            add: field(String::new(), "Text", "Text to add", cx),
            base: field(String::new(), "Name", "Base name", cx),
            start: field("1".to_owned(), "1", "Start number", cx),
            padding: field(String::new(), "0", "Zero padding", cx),
            kind: RenameKind::Replace,
            add_position: AddPosition::After,
            preserve_extension: true,
            preview: preview(&snapshot, &RenameForm::default()),
            checks: None,
            validation_error: None,
            validating: false,
            revision: 0,
            submit: None,
            preview_scroll: UniformListScrollHandle::new(),
            rules: std::array::from_fn(|_| cx.focus_handle()),
            sides: std::array::from_fn(|_| cx.focus_handle()),
            extension: cx.focus_handle(),
            cancel: cx.focus_handle(),
            rename: cx.focus_handle(),
            snapshot,
            validation_cancel: None,
            subscriptions: Vec::new(),
        }
    }

    pub(crate) fn form(&self, cx: &App) -> RenameForm {
        RenameForm {
            kind: self.kind,
            find: self.find.read(cx).text().to_owned(),
            replace: self.replace.read(cx).text().to_owned(),
            add: self.add.read(cx).text().to_owned(),
            add_position: self.add_position,
            base: self.base.read(cx).text().to_owned(),
            start: self.start.read(cx).text().to_owned(),
            padding: self.padding.read(cx).text().to_owned(),
            preserve_extension: self.preserve_extension,
        }
    }

    pub(crate) fn inputs(&self) -> Vec<BatchRenameInput> {
        self.preview
            .rows
            .iter()
            .map(|row| BatchRenameInput {
                source: row.source.clone(),
                name: row.original_name.clone(),
                is_dir: row.is_dir,
                target_name: row.destination_name.clone(),
            })
            .collect()
    }

    pub(crate) fn can_submit(&self) -> bool {
        self.submit.is_some() && !self.validating
    }

    pub(crate) fn row_state(&self, index: usize) -> Option<&BatchRowState> {
        self.checks.as_ref().and_then(|checks| checks.get(index))
    }

    pub(crate) fn focus_surfaces(&self, cx: &App) -> Vec<FocusHandle> {
        let mut surfaces = vec![self.focus.clone()];
        surfaces.extend(self.rules.iter().cloned());
        match self.kind {
            RenameKind::Replace => {
                surfaces.push(self.find.read(cx).focus_handle(cx));
                surfaces.push(self.replace.read(cx).focus_handle(cx));
            }
            RenameKind::AddText => {
                surfaces.push(self.add.read(cx).focus_handle(cx));
                surfaces.extend(self.sides.iter().cloned());
            }
            RenameKind::Number => {
                surfaces.push(self.base.read(cx).focus_handle(cx));
                surfaces.push(self.start.read(cx).focus_handle(cx));
                surfaces.push(self.padding.read(cx).focus_handle(cx));
            }
        }
        surfaces.push(self.extension.clone());
        surfaces.push(self.cancel.clone());
        surfaces.push(self.rename.clone());
        surfaces
    }

    fn tab_order(&self, cx: &App) -> Vec<FocusHandle> {
        let mut order = self.rules.to_vec();
        match self.kind {
            RenameKind::Replace => {
                order.push(self.find.read(cx).focus_handle(cx));
                order.push(self.replace.read(cx).focus_handle(cx));
            }
            RenameKind::AddText => {
                order.push(self.add.read(cx).focus_handle(cx));
                order.extend(self.sides.iter().cloned());
            }
            RenameKind::Number => {
                order.push(self.base.read(cx).focus_handle(cx));
                order.push(self.start.read(cx).focus_handle(cx));
                order.push(self.padding.read(cx).focus_handle(cx));
            }
        }
        order.push(self.extension.clone());
        order.push(self.cancel.clone());
        order.push(self.rename.clone());
        order
    }
}

impl CrowWindow {
    pub(super) fn open_batch_rename(
        &mut self,
        snapshot: Arc<[RenameInput]>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sheet.is_some() || snapshot.len() < 2 {
            return;
        }
        let mut sheet = BatchRenameSheet::new(snapshot, cx);
        let fields = [
            sheet.find.clone(),
            sheet.replace.clone(),
            sheet.add.clone(),
            sheet.base.clone(),
            sheet.start.clone(),
            sheet.padding.clone(),
        ];
        for input in fields {
            sheet.subscriptions.push(cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                TextInputEvent::Changed => this.refresh_batch_rename(window, cx),
                TextInputEvent::Confirmed => this.submit_batch_rename(window, cx),
                TextInputEvent::Cancelled => this.close_sheet(window, cx),
                TextInputEvent::CompletionRequested => this.batch_rename_tab(true, window, cx),
                TextInputEvent::BackspaceAtStart => {}
            }));
        }
        let first = sheet.find.read(cx).focus_handle(cx);
        self.sheet = Some(Sheet::BatchRename(Box::new(sheet)));
        window.focus(&first, cx);
        self.refresh_batch_rename(window, cx);
        cx.notify();
    }

    pub(super) fn refresh_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
        let form = sheet.form(cx);
        sheet.preview = preview(&sheet.snapshot, &form);
        sheet.revision = sheet.revision.wrapping_add(1);
        let revision = sheet.revision;
        if let Some(cancel) = sheet.validation_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        sheet.checks = None;
        sheet.validation_error = None;
        sheet.submit = None;
        if !sheet.preview.complete() {
            sheet.validating = false;
            cx.notify();
            return;
        }
        sheet.validating = true;
        let inputs = sheet.inputs();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        sheet.validation_cancel = Some(cancel.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let checked = cx.background_spawn(async move { validate_batch_rename(&inputs, &cancel) }).await;
            this.update_in(cx, |this, window, cx| {
                let Some(Sheet::BatchRename(sheet)) = &mut this.sheet else { return };
                if sheet.revision != revision {
                    return;
                }
                sheet.validating = false;
                match checked {
                    Ok(checked) => apply_validation(sheet, checked),
                    Err(error) => {
                        sheet.validation_error = Some(error.to_string().into());
                        sheet.checks = None;
                        sheet.submit = None;
                    }
                }
                cx.notify();
                window.refresh();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn submit_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return };
        if !sheet.can_submit() {
            crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx);
            return;
        }
        let Some(moves) = sheet.submit.clone() else { return };
        if !cx.global::<Operations>().is_idle() {
            crate::operations::message("Another operation is still running", true, cx);
            cx.notify();
            return;
        }
        let cursor = self.pane().read(cx).selection_cursor(cx);
        let batch_cursor = cursor.and_then(|cursor| {
            moves.iter().find(|entry| entry.source == cursor).map(|entry| entry.target.clone()).or(Some(cursor))
        });
        self.sheet = None;
        self.focus_center(window, cx);
        cx.global_mut::<Operations>().batch_cursor = batch_cursor;
        self.run_request(Request::BatchRename { moves }, window, cx);
    }

    pub(super) fn batch_rename_kind(&mut self, kind: RenameKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
        sheet.kind = kind;
        let focus = match kind {
            RenameKind::Replace => sheet.find.read(cx).focus_handle(cx),
            RenameKind::AddText => sheet.add.read(cx).focus_handle(cx),
            RenameKind::Number => sheet.base.read(cx).focus_handle(cx),
        };
        window.focus(&focus, cx);
        self.refresh_batch_rename(window, cx);
    }

    pub(super) fn batch_rename_side(&mut self, side: AddPosition, window: &mut Window, cx: &mut Context<Self>) {
        let focus = {
            let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
            sheet.add_position = side;
            sheet.sides[usize::from(side == AddPosition::Before)].clone()
        };
        window.focus(&focus, cx);
        self.refresh_batch_rename(window, cx);
    }

    pub(super) fn batch_rename_extension(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = {
            let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
            sheet.preserve_extension = !sheet.preserve_extension;
            sheet.extension.clone()
        };
        window.focus(&focus, cx);
        self.refresh_batch_rename(window, cx);
    }

    pub(super) fn batch_rename_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return };
        let order = sheet.tab_order(cx);
        let current = order.iter().position(|focus| focus.is_focused(window));
        let next = match (current, forward) {
            (Some(index), true) => (index + 1) % order.len(),
            (Some(index), false) => (index + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        window.focus(&order[next], cx);
        cx.notify();
    }

    pub(super) fn render_batch_rename(&mut self, _window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return div().into_any_element() };
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent, danger) = (
            color(theme.role("fg")),
            color(theme.role("fg-muted")),
            color(theme.role("accent")),
            color(theme.role("error")),
        );
        let control = |id: &'static str, label: &'static str, selected: bool, focus: &FocusHandle| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(selected)
                .track_focus(focus)
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                .px_3()
                .py_1()
                .rounded_md()
                .text_size(px(12.))
                .when(selected, |el| el.bg(color(theme.role("surface-raised"))).text_color(fg))
                .when(!selected, |el| el.text_color(muted))
                .child(label)
        };
        let extension = div()
            .id("batch-keep-extension")
            .debug_selector(|| "batch-keep-extension".into())
            .role(Role::CheckBox)
            .aria_label("Keep extension")
            .aria_toggled(if sheet.preserve_extension { gpui::Toggled::True } else { gpui::Toggled::False })
            .track_focus(&sheet.extension)
            .focus_visible(|style| style.shadow(cursor_ring(&theme)))
            .flex()
            .items_center()
            .gap_2()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(muted)
            .on_click(cx.listener(|this, _, window, cx| this.batch_rename_extension(window, cx)))
            .child(
                div()
                    .w(px(13.))
                    .h(px(13.))
                    .rounded_sm()
                    .border_1()
                    .border_color(color(theme.role("border")))
                    .when(sheet.preserve_extension, |box_| box_.bg(accent)),
            )
            .child("Keep extension");
        let rules = div()
            .flex()
            .gap_1()
            .child(
                control("batch-rule-replace", "Find & Replace", sheet.kind == RenameKind::Replace, &sheet.rules[0])
                    .on_click(
                        cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::Replace, window, cx)),
                    ),
            )
            .child(
                control("batch-rule-add", "Add Text", sheet.kind == RenameKind::AddText, &sheet.rules[1]).on_click(
                    cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::AddText, window, cx)),
                ),
            )
            .child(
                control("batch-rule-number", "Numbering", sheet.kind == RenameKind::Number, &sheet.rules[2]).on_click(
                    cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::Number, window, cx)),
                ),
            );
        let fields =
            match sheet.kind {
                RenameKind::Replace => div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(sheet.find.clone()))
                    .child(div().flex_1().child(sheet.replace.clone()))
                    .into_any_element(),
                RenameKind::AddText => div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(sheet.add.clone()))
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .child(
                                control(
                                    "batch-add-after",
                                    "After name",
                                    sheet.add_position == AddPosition::After,
                                    &sheet.sides[0],
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.batch_rename_side(AddPosition::After, window, cx)
                                })),
                            )
                            .child(
                                control(
                                    "batch-add-before",
                                    "Before name",
                                    sheet.add_position == AddPosition::Before,
                                    &sheet.sides[1],
                                )
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.batch_rename_side(AddPosition::Before, window, cx)
                                })),
                            ),
                    )
                    .into_any_element(),
                RenameKind::Number => div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().child(sheet.base.clone()))
                    .child(div().w(px(90.)).child(sheet.start.clone()))
                    .child(div().w(px(90.)).child(sheet.padding.clone()))
                    .into_any_element(),
            };
        let header = div()
            .flex()
            .gap_2()
            .text_size(px(11.))
            .text_color(muted)
            .child(div().flex_1().child("Original"))
            .child(div().flex_1().child("New name"));
        let rows = sheet.preview.rows.len();
        let list = uniform_list("batch-preview", rows, cx.processor(Self::render_batch_preview_rows))
            .track_scroll(&sheet.preview_scroll)
            .h(px(180.))
            .w_full();
        let status = if let Some(error) =
            sheet.validation_error.clone().or_else(|| sheet.preview.error.clone().map(Into::into))
        {
            div().text_size(px(12.)).text_color(danger).child(error)
        } else if sheet.validating {
            div().text_size(px(12.)).text_color(muted).child("Checking…")
        } else if sheet.preview.changed() == 0 {
            div().text_size(px(12.)).text_color(muted).child("No names would change")
        } else if sheet.can_submit() {
            div().text_size(px(12.)).text_color(muted).child(format!(
                "{} name{} will change",
                sheet.preview.changed(),
                if sheet.preview.changed() == 1 { "" } else { "s" }
            ))
        } else {
            div().text_size(px(12.)).text_color(danger).child("Fix the highlighted rows")
        };
        let can_submit = sheet.can_submit();
        let button = |id: &'static str, label: &'static str, primary: bool, focus: &FocusHandle| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Button)
                .aria_label(label)
                .aria_disabled(primary && !can_submit)
                .track_focus(focus)
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                .px_3()
                .py_1()
                .rounded_md()
                .bg(color(if primary && can_submit { theme.role("accent") } else { theme.role("surface-raised") }))
                .text_color(if primary && can_submit { color(theme.role("on-accent")) } else { fg })
                .when(primary && !can_submit, |el| el.opacity(0.5))
                .child(label)
        };
        let body = div()
            .id("batch-rename")
            .debug_selector(|| "batch-rename".into())
            .flex()
            .flex_col()
            .gap_3()
            .child(rules)
            .child(fields)
            .child(extension)
            .child(header)
            .child(list)
            .child(status)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        button("batch-cancel", "Cancel", false, &sheet.cancel)
                            .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
                    )
                    .child(
                        button("batch-rename-submit", "Rename", true, &sheet.rename)
                            .on_click(cx.listener(|this, _, window, cx| this.submit_batch_rename(window, cx))),
                    ),
            );
        let tab = cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.key != "tab" {
                return;
            }
            this.batch_rename_tab(!event.keystroke.modifiers.shift, window, cx);
            cx.stop_propagation();
        });
        chrome::sheet(
            chrome::sheet_panel("sheet", SheetWidth::Wide, &theme)
                .aria_label("Batch Rename")
                .key_context("Sheet")
                .track_focus(&sheet.focus)
                .on_action(cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| {
                    this.submit_batch_rename(window, cx)
                }))
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)),
                )
                .on_key_down(tab)
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Batch Rename"))
                .child(body),
            &theme,
            cx.global::<Ui>().feedback.mode,
            cx.listener(|this, _, window, cx| this.close_sheet(window, cx)),
        )
        .into_any_element()
    }

    fn render_batch_preview_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return Vec::new() };
        let theme = cx.global::<Ui>().theme.clone();
        let muted = color(theme.role("fg-muted"));
        let error = color(theme.role("error"));
        range
            .filter_map(|index| {
                let row = sheet.preview.rows.get(index)?;
                let state = sheet.row_state(index);
                let (line, color_) = match state {
                    Some(BatchRowState::Invalid(reason)) => (reason.as_str(), error),
                    _ => match &row.state {
                        RowState::Invalid(reason) => (reason.as_str(), error),
                        RowState::Unchanged => ("unchanged", muted),
                        RowState::Ready => ("", muted),
                    },
                };
                let detail = if line.is_empty() { String::new() } else { format!(", {line}") };
                Some(
                    div()
                        .id(("batch-preview-row", index))
                        .debug_selector(move || format!("batch-preview-{index}"))
                        .role(Role::ListItem)
                        .aria_label(format!("{} becomes {}{detail}", row.original_name, row.destination_name))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(row.original_name.clone()),
                        )
                        .child(div().text_color(muted).child("→"))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_color(if line.is_empty() { color(theme.role("fg")) } else { color_ })
                                .child(row.destination_name.clone()),
                        )
                        .when(!line.is_empty(), |row| {
                            row.child(div().text_size(px(11.)).text_color(color_).child(line.to_owned()))
                        })
                        .into_any_element(),
                )
            })
            .collect()
    }
}

fn apply_validation(sheet: &mut BatchRenameSheet, checked: BatchValidation) {
    sheet.checks = Some(checked.rows.clone().into());
    sheet.validation_error = checked.error.map(Into::into);
    let every_row = checked.rows.iter().all(|row| !matches!(row, BatchRowState::Invalid(_)));
    sheet.submit =
        (every_row && sheet.validation_error.is_none() && !checked.moves.is_empty()).then(|| Arc::from(checked.moves));
}
