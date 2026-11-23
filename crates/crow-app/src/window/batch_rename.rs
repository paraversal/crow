//! The batch-rename sheet: one rule at a time, a live old → new preview, and
//! submission of an immutable approved mapping. The pure rule lives in
//! `crow_core::batch_rename`; filesystem checks and the commit live in
//! `crow_fs::ops` off the main thread.

use super::*;
use crow_core::batch_rename::{
    AddPosition, FormField, Preview, RenameForm, RenameInput, RenameKind, RowState, preview,
};
use crow_fs::ops::{BatchRenameEntry, BatchRenameInput, BatchRowState, BatchValidation, validate_batch_rename};
use gpui::{AnyElement, Pixels, UniformListScrollHandle, uniform_list};

pub(crate) struct BatchRenameSheet {
    pub(crate) focus: FocusHandle,
    pub(crate) find: Entity<TextInput>,
    pub(crate) replace: Entity<TextInput>,
    pub(crate) add: Entity<TextInput>,
    pub(crate) base: Entity<TextInput>,
    pub(crate) start: Entity<TextInput>,
    pub(crate) padding: Entity<TextInput>,
    pub(crate) kind: RenameKind,
    /// Index of the tab the ink bar slides from after a mode change.
    tab_from: usize,
    /// Longest original name in characters; the sheet widens for long names.
    longest: usize,
    /// Rows (first `ENTER_ROWS`) whose After text changed with the last mode
    /// change or opening, and when that happened. Only these rows animate, and
    /// only briefly, so scrolling a row back into view never replays it.
    enter_rows: Vec<bool>,
    enter_started: Option<std::time::Instant>,
    mode_changed: bool,
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

/// Preview revisions are unique across sheets, so a canceled validation from a
/// closed sheet can never match the next sheet's revision after it reopens.
static NEXT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Selections above this build their preview off the main thread.
const LARGE_SELECTION: usize = 500;
/// Keystrokes within this window share one preview rebuild on a large selection.
const PREVIEW_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(50);
const PREVIEW_ARROW_WIDTH: Pixels = px(16.);

fn batch_inputs(preview: &Preview) -> Vec<BatchRenameInput> {
    preview
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

enum BatchRenameActivation {
    Kind(RenameKind),
    Side(AddPosition),
    Extension,
    Cancel,
    Rename,
}

/// A group that appears (the new rule's sentence, the error strip) fades in; in
/// Full it also rises 4 px, drawn only, so nothing around it moves. Off draws
/// it at once. The animation restarts whenever `id` changes or the element is
/// mounted again.
fn enter<E: IntoElement + gpui::Styled + 'static>(
    element: E,
    id: impl Into<gpui::ElementId>,
    mode: crow_core::motion::Effective,
    ms: u64,
) -> AnyElement {
    enter_after(element, id, mode, ms, 0, (0., 4.))
}

/// `enter` that waits `delay` ms first (invisible meanwhile), so rows of a table
/// can arrive one after another.
fn enter_after<E: IntoElement + gpui::Styled + 'static>(
    element: E,
    id: impl Into<gpui::ElementId>,
    mode: crow_core::motion::Effective,
    ms: u64,
    delay: u64,
    from: (f32, f32),
) -> AnyElement {
    use crow_core::motion::{Effective, ease_out};
    use gpui::{Animation, AnimationExt as _};
    if mode == Effective::Off {
        return element.into_any_element();
    }
    let total = (ms + delay) as f32;
    element
        .relative()
        .with_animation(id, Animation::new(std::time::Duration::from_millis(ms + delay)), move |el, t| {
            let t = ease_out(((t * total - delay as f32) / ms as f32).clamp(0., 1.));
            let el = el.opacity(t);
            if mode == Effective::Full { el.left(px(from.0 * (1. - t))).top(px(from.1 * (1. - t))) } else { el }
        })
        .into_any_element()
}

/// Width of one mode tab; the ink bar slides in steps of this.
const TAB_WIDTH: f32 = 104.;
/// Gap between rows' arrival when the mode changes, and how many rows stagger.
const ROW_STAGGER_MS: u64 = 35;
const ROW_STAGGER_ROWS: usize = 6;
/// Rows tracked for the mode-change animation: more than fit on screen at once.
const ENTER_ROWS: usize = 40;
/// How long after a mode change a row may still start its entrance.
const ENTER_WINDOW: std::time::Duration = std::time::Duration::from_millis(600);

fn field(value: String, placeholder: &'static str, aria: &'static str, cx: &mut App) -> Entity<TextInput> {
    cx.new(|cx| TextInput::new(value, placeholder, cx).bare().with_aria_label(aria))
}

/// What the After column shows for a row, to tell whether a new preview changes it.
fn after_key(preview: &Preview, index: usize) -> Option<&str> {
    let row = preview.rows.get(index)?;
    Some(if preview.error.is_some() {
        "\u{2014}"
    } else if row.state == RowState::Unchanged {
        "unchanged"
    } else {
        row.destination_name.as_str()
    })
}

impl BatchRenameSheet {
    /// Installs a new preview. After a mode change it records which of the first
    /// rows now show different text, so only those animate.
    fn adopt(&mut self, built: Preview) {
        if std::mem::take(&mut self.mode_changed) {
            self.enter_rows = (0..ENTER_ROWS.min(built.rows.len()))
                .map(|i| after_key(&self.preview, i) != after_key(&built, i))
                .collect();
            self.enter_started = Some(std::time::Instant::now());
        }
        self.preview = built;
    }

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
            tab_from: 0,
            longest: snapshot.iter().map(|input| input.name.chars().count()).max().unwrap_or(0),
            enter_rows: vec![true; ENTER_ROWS.min(snapshot.len())],
            enter_started: Some(std::time::Instant::now()),
            mode_changed: false,
            add_position: AddPosition::After,
            preserve_extension: true,
            preview: preview(&snapshot, &RenameForm::default()),
            checks: None,
            validation_error: None,
            validating: false,
            revision: NEXT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
        batch_inputs(&self.preview)
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
        // The opening preview is the untouched form's, which `new` already built.
        cx.notify();
    }

    pub(super) fn refresh_batch_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
        let form = sheet.form(cx);
        sheet.revision = sheet.revision.wrapping_add(1);
        let revision = sheet.revision;
        if let Some(cancel) = sheet.validation_cancel.take() {
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        sheet.checks = None;
        sheet.validation_error = None;
        sheet.submit = None;
        // A big selection builds its preview (one row per item) off the main
        // thread after a short debounce; the rows on screen stay until it lands.
        let large = sheet.snapshot.len() > LARGE_SELECTION;
        if !large {
            let built = preview(&sheet.snapshot, &form);
            sheet.adopt(built);
            if !sheet.preview.complete() {
                sheet.validating = false;
                cx.notify();
                return;
            }
        }
        sheet.validating = true;
        let snapshot = sheet.snapshot.clone();
        let ready = (!large).then(|| sheet.inputs());
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        sheet.validation_cancel = Some(cancel.clone());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            if large {
                cx.background_executor().timer(PREVIEW_DEBOUNCE).await;
                if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
            }
            let (built, checked) = cx
                .background_spawn(async move {
                    match ready {
                        Some(inputs) => (None, Some(validate_batch_rename(&inputs, &cancel))),
                        None => {
                            let built = preview(&snapshot, &form);
                            let checked =
                                built.complete().then(|| validate_batch_rename(&batch_inputs(&built), &cancel));
                            (Some(built), checked)
                        }
                    }
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let Some(Sheet::BatchRename(sheet)) = &mut this.sheet else { return };
                if sheet.revision != revision {
                    return;
                }
                if let Some(built) = built {
                    sheet.adopt(built);
                }
                sheet.validating = false;
                match checked {
                    Some(Ok(checked)) => apply_validation(sheet, checked),
                    Some(Err(error)) => {
                        sheet.validation_error = Some(error.to_string().into());
                        sheet.checks = None;
                        sheet.submit = None;
                    }
                    None => {}
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
            let field = sheet.preview.error_field.map(|field| match field {
                FormField::Find => sheet.find.clone(),
                FormField::Add => sheet.add.clone(),
                FormField::Base => sheet.base.clone(),
                FormField::Start => sheet.start.clone(),
                FormField::Padding => sheet.padding.clone(),
            });
            match &field {
                Some(field) => crate::feedback::refuse(crate::feedback::RefuseTarget::Field(field), window, cx),
                None => crate::feedback::refuse(crate::feedback::RefuseTarget::None, window, cx),
            }
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
        retire!(self, cx, sheet);
        self.focus_center(window, cx);
        cx.global_mut::<Operations>().batch_cursor = batch_cursor;
        self.run_request(Request::BatchRename { moves }, window, cx);
    }

    pub(super) fn batch_rename_kind(&mut self, kind: RenameKind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &mut self.sheet else { return };
        if sheet.kind != kind {
            sheet.tab_from = sheet.kind as usize;
        }
        sheet.kind = kind;
        sheet.mode_changed = true;
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

    fn batch_rename_activation(&self, window: &Window) -> Option<BatchRenameActivation> {
        let Sheet::BatchRename(sheet) = self.sheet.as_ref()? else { return None };
        if sheet.rules[0].is_focused(window) {
            Some(BatchRenameActivation::Kind(RenameKind::Replace))
        } else if sheet.rules[1].is_focused(window) {
            Some(BatchRenameActivation::Kind(RenameKind::AddText))
        } else if sheet.rules[2].is_focused(window) {
            Some(BatchRenameActivation::Kind(RenameKind::Number))
        } else if sheet.sides[0].is_focused(window) {
            Some(BatchRenameActivation::Side(AddPosition::After))
        } else if sheet.sides[1].is_focused(window) {
            Some(BatchRenameActivation::Side(AddPosition::Before))
        } else if sheet.extension.is_focused(window) {
            Some(BatchRenameActivation::Extension)
        } else if sheet.cancel.is_focused(window) {
            Some(BatchRenameActivation::Cancel)
        } else if sheet.rename.is_focused(window) {
            Some(BatchRenameActivation::Rename)
        } else {
            None
        }
    }

    fn activate_batch_rename_control(
        &mut self,
        activation: BatchRenameActivation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match activation {
            BatchRenameActivation::Kind(kind) => self.batch_rename_kind(kind, window, cx),
            BatchRenameActivation::Side(side) => self.batch_rename_side(side, window, cx),
            BatchRenameActivation::Extension => self.batch_rename_extension(window, cx),
            BatchRenameActivation::Cancel => self.close_sheet(window, cx),
            BatchRenameActivation::Rename => self.submit_batch_rename(window, cx),
        }
    }

    /// Space activates the focused radio, toggle or button. Enter arrives as
    /// `SheetConfirm` (GPUI dispatches key bindings before key listeners), so
    /// `batch_rename_confirm` handles the same focus before submitting.
    pub(super) fn batch_rename_key_down(
        &mut self,
        event: &gpui::KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "space" {
            return;
        }
        let Some(activation) = self.batch_rename_activation(window) else { return };
        cx.stop_propagation();
        self.activate_batch_rename_control(activation, window, cx);
    }

    pub(super) fn batch_rename_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(activation) = self.batch_rename_activation(window) {
            self.activate_batch_rename_control(activation, window, cx);
        } else {
            self.submit_batch_rename(window, cx);
        }
    }

    pub(super) fn batch_rename_tab(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return };
        let order = sheet.tab_order(cx);
        crate::chrome::cycle_focus(&order, forward, window, cx);
        cx.notify();
    }

    pub(super) fn render_batch_rename(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(Sheet::BatchRename(sheet)) = &self.sheet else { return div().into_any_element() };
        let theme = cx.global::<Ui>().theme.clone();
        let (fg, muted, accent, danger, border) = (
            color(theme.role("fg")),
            color(theme.role("fg-muted")),
            color(theme.role("accent")),
            color(theme.role("error")),
            color(theme.role("border")),
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
        let segmented = || div().flex().flex_none().gap_1().p(px(2.)).rounded_md().bg(color(theme.role("bg-inset")));
        let word = |text: &'static str| div().flex_none().text_size(px(13.)).text_color(muted).child(text);
        let bad_field = sheet.preview.error_field;
        let slot = |input: &Entity<TextInput>, id: &'static str, width: f32, field: Option<FormField>| {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            let edge = if field.is_some() && bad_field == field {
                danger
            } else if focused {
                accent
            } else {
                border
            };
            // Inline text with an underline, not a box: the sentence reads as one line.
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .w(px(width))
                .px_1()
                .py_1()
                .text_size(px(13.))
                .border_b_1()
                .border_color(edge)
                .child(input.clone())
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
            .flex_none()
            .items_center()
            .gap_1()
            .px_2()
            .py_1()
            .rounded_md()
            .cursor_pointer()
            .text_size(px(12.))
            .text_color(if sheet.preserve_extension { fg } else { muted })
            .on_click(cx.listener(|this, _, window, cx| this.batch_rename_extension(window, cx)))
            .child(div().w(px(10.)).text_color(accent).child(if sheet.preserve_extension { "✓" } else { "" }))
            .child("Keep extension");
        let mode = cx.global::<Ui>().feedback.mode;
        let tab = |id: &'static str, label: &'static str, selected: bool, focus: &FocusHandle| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(selected)
                .track_focus(focus)
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                .w(px(TAB_WIDTH))
                .flex_none()
                .py_2()
                .flex()
                .justify_center()
                .cursor_pointer()
                .text_size(px(13.))
                .when(selected, |el| el.text_color(fg).font_weight(FontWeight::MEDIUM))
                .when(!selected, |el| el.text_color(muted).hover(|el| el.text_color(fg)))
                .child(label)
        };
        // The ink bar slides from the tab that was active; Off puts it there at once.
        let (from, to) = (sheet.tab_from as f32 * TAB_WIDTH, sheet.kind as usize as f32 * TAB_WIDTH);
        let ink = div().absolute().bottom(px(-1.)).w(px(TAB_WIDTH)).h(px(2.)).bg(accent);
        let ink = if mode == crow_core::motion::Effective::Off {
            ink.left(px(to)).into_any_element()
        } else {
            use gpui::{Animation, AnimationExt as _};
            ink.left(px(from))
                .with_animation(
                    ("batch-ink", sheet.kind as usize),
                    Animation::new(std::time::Duration::from_millis(crow_core::motion::ms::FIELD)),
                    move |el, t| el.left(px(from + (to - from) * crow_core::motion::ease_out(t))),
                )
                .into_any_element()
        };
        let rules =
            div()
                .flex()
                .flex_none()
                .relative()
                .child(
                    tab("batch-rule-replace", "Find & Replace", sheet.kind == RenameKind::Replace, &sheet.rules[0])
                        .on_click(
                            cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::Replace, window, cx)),
                        ),
                )
                .child(tab("batch-rule-add", "Add Text", sheet.kind == RenameKind::AddText, &sheet.rules[1]).on_click(
                    cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::AddText, window, cx)),
                ))
                .child(
                    tab("batch-rule-number", "Numbering", sheet.kind == RenameKind::Number, &sheet.rules[2]).on_click(
                        cx.listener(|this, _, window, cx| this.batch_rename_kind(RenameKind::Number, window, cx)),
                    ),
                )
                .child(ink);
        let sentence = div().flex().flex_wrap().items_center().gap_2();
        let sentence =
            match sheet.kind {
                RenameKind::Replace => sentence
                    .child(word("Replace"))
                    .child(slot(&sheet.find, "batch-field-find", 130., Some(FormField::Find)))
                    .child(word("with"))
                    .child(slot(&sheet.replace, "batch-field-replace", 130., None)),
                RenameKind::AddText => sentence
                    .child(word("Add"))
                    .child(slot(&sheet.add, "batch-field-add", 160., Some(FormField::Add)))
                    .child(
                        segmented()
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
                    ),
                RenameKind::Number => sentence
                    .child(word("Name them"))
                    .child(slot(&sheet.base, "batch-field-base", 120., Some(FormField::Base)))
                    .child(word("counting from"))
                    .child(slot(&sheet.start, "batch-field-start", 52., Some(FormField::Start)))
                    .child(word("with"))
                    .child(slot(&sheet.padding, "batch-field-padding", 40., Some(FormField::Padding)))
                    .child(word("digits")),
            };
        // The new sentence slides in from the side of its tab: from the right when
        // moving to a later tab, from the left when moving back.
        let from_x = if sheet.kind as usize >= sheet.tab_from { 16. } else { -16. };
        let sentence = enter_after(
            sentence,
            ("batch-sentence", sheet.kind as usize),
            mode,
            crow_core::motion::ms::FIELD,
            0,
            (from_x, 0.),
        );
        let bar = div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(border)
                    .child(rules)
                    .child(extension),
            )
            .child(div().min_h(px(28.)).child(sentence));
        let header = div()
            .flex()
            .gap_2()
            .px_3()
            .py_1()
            .text_size(px(11.))
            .text_color(muted)
            .bg(color(theme.role("bg-inset")))
            .child(div().flex_1().min_w_0().child("Before"))
            .child(div().w(PREVIEW_ARROW_WIDTH).flex_none())
            .child(div().debug_selector(|| "batch-preview-new-name-heading".into()).flex_1().min_w_0().child("After"));
        let rows = sheet.preview.rows.len();
        // The list grows with the selection and the window, down to a short
        // minimum; beyond that it scrolls.
        let viewport = window.viewport_size();
        let room = (f32::from(viewport.height) - 330.).max(180.);
        let list_height = (rows as f32 * 24.).clamp(96., room);
        // Long names widen the sheet (about 7 px a character, two columns).
        let sheet_width = ((sheet.longest as f32 * 7.5 * 2.) + 140.)
            .clamp(620., (f32::from(viewport.width) - 48.).clamp(620., 1100.));
        let list = uniform_list("batch-preview", rows, cx.processor(Self::render_batch_preview_rows))
            .track_scroll(&sheet.preview_scroll)
            .h(px(list_height))
            .w_full();
        let table = div()
            .rounded_md()
            .border_1()
            .border_color(border)
            .overflow_hidden()
            .flex()
            .flex_col()
            .child(header)
            .child(list);
        let message: Option<SharedString> =
            sheet.validation_error.clone().or_else(|| sheet.preview.error.clone().map(Into::into));
        let strip = message.map(|message| {
            let strip = div()
                .id("batch-error")
                .debug_selector(|| "batch-error".into())
                .flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_1()
                .rounded_md()
                .bg(crate::style::error_soft(&theme))
                .text_size(px(12.))
                .text_color(danger)
                .child(crate::icons::glyph(crate::icons::Glyph::AlertCircle, 13., "error", &theme))
                .child(message);
            enter(strip, "batch-error-in", mode, crow_core::motion::ms::FILTER_ROW)
        });
        let invalid = if sheet.preview.error.is_some() {
            0
        } else {
            (0..rows)
                .filter(|&index| match sheet.row_state(index) {
                    Some(BatchRowState::Invalid(_)) => true,
                    _ => matches!(sheet.preview.rows[index].state, RowState::Invalid(_)),
                })
                .count()
        };
        let changed = sheet.preview.changed();
        let status = if sheet.preview.error.is_some() {
            div().text_color(danger).child("Fix the highlighted field to preview")
        } else if sheet.validation_error.is_some() {
            div().text_color(danger).child("Couldn't check these names")
        } else if sheet.validating {
            div().text_color(muted).child("Checking…")
        } else if invalid > 0 {
            div()
                .text_color(danger)
                .child(format!("{invalid} row{} can't be renamed", if invalid == 1 { "" } else { "s" }))
        } else if changed == 0 {
            div().text_color(muted).child("No names would change")
        } else {
            div().text_color(muted).child(format!("{changed} of {rows} will change"))
        };
        let can_submit = sheet.can_submit();
        let rename_label = if can_submit && changed > 0 { format!("Rename {changed}") } else { "Rename".to_owned() };
        let button = |id: &'static str, label: SharedString, primary: bool, focus: &FocusHandle| {
            let dead = primary && !can_submit;
            crate::chrome::sheet_button(id, label, primary && can_submit, &theme)
                .aria_disabled(dead)
                .track_focus(focus)
                .focus_visible(|style| style.shadow(cursor_ring(&theme)))
                .when(dead, |el| el.opacity(0.5))
        };
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().id("batch-status").flex_1().min_w_0().text_size(px(12.)).child(status))
            .child(
                button("batch-cancel", "Cancel".into(), false, &sheet.cancel)
                    .on_click(cx.listener(|this, _, window, cx| this.close_sheet(window, cx))),
            )
            .child(
                button("batch-rename-submit", rename_label.into(), true, &sheet.rename)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_batch_rename(window, cx))),
            );
        let body = div()
            .id("batch-rename")
            .debug_selector(|| "batch-rename".into())
            .flex()
            .flex_col()
            .gap_3()
            .child(bar)
            .children(strip)
            .child(table)
            .child(footer);
        let tab = cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
            if event.keystroke.key != "tab" {
                return;
            }
            this.batch_rename_tab(!event.keystroke.modifiers.shift, window, cx);
            cx.stop_propagation();
        });
        chrome::sheet(
            chrome::sheet_panel("sheet", SheetWidth::Fit(sheet_width as u16), &theme)
                .aria_label("Batch Rename")
                .key_context("Sheet")
                .track_focus(&sheet.focus)
                .on_action(cx.listener(|this, _: &crate::actions::SheetConfirm, window, cx| {
                    this.batch_rename_confirm(window, cx)
                }))
                .on_action(
                    cx.listener(|this, _: &crate::actions::SheetCancel, window, cx| this.close_sheet(window, cx)),
                )
                .on_key_down(tab)
                .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                    this.batch_rename_key_down(event, window, cx)
                }))
                .p_4()
                .flex()
                .flex_col()
                .gap_3()
                .child(div().font_weight(FontWeight::SEMIBOLD).child("Batch Rename"))
                .child(body),
            &theme,
            cx.global::<Ui>().feedback.mode,
            self.exit_fade.get(),
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
        let fg = color(theme.role("fg"));
        let muted = color(theme.role("fg-muted"));
        let error = color(theme.role("error"));
        // A blank field is a form error, shown once above the table. Repeating
        // it on every preview row would bury the names the rows are there to show.
        let form_error = sheet.preview.error.is_some();
        let motion = cx.global::<Ui>().feedback.mode;
        let kind = sheet.kind as usize;
        // Rows only animate shortly after opening or a mode change, never when
        // scrolled back into view.
        let animate = sheet.enter_started.is_some_and(|at| at.elapsed() < ENTER_WINDOW);
        range
            .filter_map(|index| {
                let row = sheet.preview.rows.get(index)?;
                let reason = if form_error {
                    None
                } else {
                    match sheet.row_state(index) {
                        Some(BatchRowState::Invalid(reason)) => Some(reason.as_str()),
                        _ => match &row.state {
                            RowState::Invalid(reason) => Some(reason.as_str()),
                            _ => None,
                        },
                    }
                };
                let unchanged = !form_error && reason.is_none() && row.state == RowState::Unchanged;
                let line = reason.unwrap_or("");
                let detail = if let Some(reason) = reason {
                    format!(", {reason}")
                } else if unchanged {
                    ", unchanged".to_owned()
                } else {
                    String::new()
                };
                let aria = format!("{} becomes {}{detail}", row.original_name, row.destination_name);
                let name = |text: &str, span: std::ops::Range<usize>, tone: crow_theme::Rgb| {
                    crate::style::change_text(text.to_owned(), span, tone)
                };
                let before = if form_error || unchanged || reason.is_some() {
                    div().child(row.original_name.clone())
                } else {
                    div().child(name(&row.original_name, row.removed.clone(), theme.role("error")))
                };
                let after = if form_error {
                    div().text_color(muted).child("—")
                } else if unchanged {
                    div().text_color(muted).child("unchanged")
                } else if reason.is_some() {
                    div().text_color(error).child(row.destination_name.clone())
                } else {
                    div().child(name(&row.destination_name, row.inserted.clone(), theme.role("accent")))
                };
                let marker = if reason.is_some() {
                    crate::icons::glyph(crate::icons::Glyph::AlertCircle, 13., "error", &theme).into_any_element()
                } else {
                    div().text_color(muted).child(if form_error || unchanged { "–" } else { "→" }).into_any_element()
                };
                let cell = div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        after
                            .debug_selector(move || format!("batch-preview-new-name-{index}"))
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis(),
                    )
                    .when(!line.is_empty(), |destination| {
                        destination.child(
                            div()
                                .max_w(gpui::relative(0.8))
                                .flex_none()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .text_size(px(11.))
                                .text_color(error)
                                .child(line.to_owned()),
                        )
                    });
                // A mode change brings the new names in one row after another; the
                // keystroke-driven rebuilds keep the same id and do not replay it.
                let delay = ROW_STAGGER_MS * index.min(ROW_STAGGER_ROWS) as u64;
                let cell = if animate && sheet.enter_rows.get(index).copied().unwrap_or(false) {
                    enter_after(
                        cell,
                        ("batch-after", index * 3 + kind),
                        motion,
                        crow_core::motion::ms::FIELD,
                        delay,
                        (0., 4.),
                    )
                } else {
                    cell.into_any_element()
                };
                Some(
                    div()
                        .id(("batch-preview-row", index))
                        .debug_selector(move || format!("batch-preview-{index}"))
                        .role(Role::ListItem)
                        .aria_label(aria)
                        .h(px(24.))
                        .w_full()
                        .px_3()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(px(12.))
                        .text_color(if unchanged || form_error { muted } else { fg })
                        .when(reason.is_some(), |row| row.bg(crate::style::error_soft(&theme)))
                        .child(before.flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis())
                        .child(div().w(PREVIEW_ARROW_WIDTH).flex_none().flex().justify_center().child(marker))
                        .child(cell)
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
