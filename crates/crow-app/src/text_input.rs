//! A single-line text field, adapted from GPUI's `examples/input.rs`
//! (Zed Industries, Apache-2.0). Changes: theme colors, accessibility,
//! and Return/Escape/Tab reported as events. Used by the palette, filter, search and rename fields.

use std::ops::Range;
use std::time::{Duration, Instant};

use gpui::{
    App, Bounds, ClipboardItem, ContentMask, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, Pixels, Point, Role, ShapedLine, SharedString, Style,
    TextRun, UTF16Selection, UnderlineStyle, Window, actions, div, fill, point, prelude::*, px, relative, size,
};
use unicode_segmentation::*;

use crate::actions::{
    CopyFiles as CrowCopyFiles, CutFiles as CrowCutFiles, PasteFiles as CrowPasteFiles, Redo as CrowRedo,
    SelectAll as CrowSelectAll, Undo as CrowUndo,
};
use crate::style::{color, color_alpha, effect_tint};
use crate::ui::Ui;
use crow_core::motion::{self, Effective, Fx};

actions!(
    text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        WordLeft,
        WordRight,
        SelectWordLeft,
        SelectWordRight,
        DeleteWordLeft,
        DeleteWordRight,
        SelectAll,
        Home,
        End,
        ShowCharacterPalette,
        Paste,
        Cut,
        Copy,
        Confirm,
        Cancel,
        Complete
    ]
);

pub enum TextInputEvent {
    /// The text changed (typing, deleting, pasting, or `set_text`).
    Changed,
    Confirmed,
    Cancelled,
    CompletionRequested,
    /// Backspace with the caret at the start and nothing selected. Only emitted
    /// when the input asked for it; otherwise the field rings the bell.
    BackspaceAtStart,
}

pub fn keymap() -> Vec<KeyBinding> {
    let c = Some("TextInput");
    vec![
        KeyBinding::new("backspace", Backspace, c),
        KeyBinding::new("delete", Delete, c),
        KeyBinding::new("left", Left, c),
        KeyBinding::new("right", Right, c),
        KeyBinding::new("shift-left", SelectLeft, c),
        KeyBinding::new("shift-right", SelectRight, c),
        KeyBinding::new("alt-left", WordLeft, c),
        KeyBinding::new("alt-right", WordRight, c),
        KeyBinding::new("alt-shift-left", SelectWordLeft, c),
        KeyBinding::new("alt-shift-right", SelectWordRight, c),
        KeyBinding::new("alt-backspace", DeleteWordLeft, c),
        KeyBinding::new("alt-delete", DeleteWordRight, c),
        KeyBinding::new("cmd-a", SelectAll, c),
        KeyBinding::new("home", Home, c),
        KeyBinding::new("cmd-left", Home, c),
        KeyBinding::new("end", End, c),
        KeyBinding::new("cmd-right", End, c),
        KeyBinding::new("ctrl-cmd-space", ShowCharacterPalette, c),
        KeyBinding::new("cmd-v", Paste, c),
        KeyBinding::new("cmd-x", Cut, c),
        KeyBinding::new("cmd-c", Copy, c),
        KeyBinding::new("enter", Confirm, c),
        KeyBinding::new("escape", Cancel, c),
        KeyBinding::new("tab", Complete, c),
    ]
}

/// Room between a field's left edge and its text, so the caret sits before the
/// first glyph instead of on it. The caret stays at the insertion point; the text
/// is shifted right by this much.
const TEXT_INSET: f32 = 2.;

/// How long the caret stays on or off while the field has focus.
const CARET_BLINK_INTERVAL: Duration = Duration::from_millis(500);

pub struct TextInput {
    id: ElementId,
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    /// A stepper's value and range, for VoiceOver's numeric announcement.
    numeric: Option<Numeric>,
    /// Spoken name when it differs from the placeholder.
    aria_label: Option<SharedString>,
    /// Whether to draw the field box. Bare inputs sit inside a field the parent draws.
    chrome: bool,
    /// Report a backspace at the start instead of ringing the bell.
    report_backspace_at_start: bool,
    role: Role,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    /// Horizontal scroll that keeps the caret inside the field. Every field scrolls;
    /// only fields too narrow to show their whole text ever have a non-zero value.
    scroll_x: Pixels,
    is_selecting: bool,
    history: History,
    /// The caret is drawn while this is true; a timer flips it every
    /// `CARET_BLINK_INTERVAL` while the field has focus.
    caret_visible: bool,
    /// When the caret next changes phase. Edits and caret moves push it out and
    /// turn the caret on, so it is solid while typing.
    blink_due: Instant,
    /// The focus state paint last saw, so gaining focus can turn the caret on at once.
    was_focused: bool,
    /// The blink timer runs for the field's whole life; it notifies only while focused.
    blink_loop: bool,
    /// A refused key: the field shakes (Full) or its ring tints red (Low) until this passes.
    refuse: Option<(u64, Instant)>,
    refuse_generation: u64,
}

/// The field's own undo history. Inside a text field ⌘Z undoes typing, never a file operation.
#[derive(Default)]
struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last edit and where the cursor ended, so a run of typing or deleting undoes as one step.
    last: Option<(EditKind, usize)>,
}

struct Snapshot {
    content: SharedString,
    selected_range: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Typing,
    Deleting,
    /// Paste, cut and completion: always their own step.
    Other,
}

const HISTORY_LIMIT: usize = 100;

/// A stepper field's value and range, announced as a spin button.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Numeric {
    pub value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl TextInput {
    pub fn new(content: impl Into<SharedString>, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        let content = content.into();
        let end = content.len();
        Self {
            id: "text-input".into(),
            focus_handle: cx.focus_handle(),
            content,
            placeholder: placeholder.into(),
            numeric: None,
            aria_label: None,
            chrome: true,
            report_backspace_at_start: false,
            role: Role::TextInput,
            selected_range: end..end,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            scroll_x: px(0.),
            is_selecting: false,
            history: History::default(),
            caret_visible: true,
            blink_due: cx.background_executor().now() + CARET_BLINK_INTERVAL,
            was_focused: false,
            blink_loop: false,
            refuse: None,
            refuse_generation: 0,
        }
    }

    pub fn text(&self) -> &str {
        &self.content
    }

    /// A unique element id, for several fields in one window.
    pub fn with_id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = id.into();
        self
    }

    /// No fill, border or padding: the parent draws the field around it.
    pub fn bare(mut self) -> Self {
        self.chrome = false;
        self
    }

    /// Refuse a key: one alert beep, then the shake (Full) or red ring (Low).
    pub fn refuse(&mut self, cx: &mut Context<Self>) {
        crate::feedback::play(crow_core::motion::Event::Refuse, cx);
        self.refuse_visual(cx);
    }

    /// The visual half of `refuse`, for callers that already beeped.
    pub fn refuse_visual(&mut self, cx: &mut Context<Self>) {
        let mode = cx.global::<Ui>().feedback.mode;
        if mode == crow_core::motion::Effective::Off {
            return;
        }
        self.refuse_generation += 1;
        let generation = self.refuse_generation;
        self.refuse = Some((generation, Instant::now()));
        cx.notify();
        let total: Duration = crow_core::motion::phases(mode, crow_core::motion::Fx::Refuse).iter().sum();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(total + Duration::from_millis(250)).await;
            this.update(cx, |this, cx| {
                if this.refuse_generation == generation {
                    this.refuse = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn with_role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The spoken name, when the placeholder is not the right one.
    pub fn with_aria_label(mut self, label: impl Into<SharedString>) -> Self {
        self.aria_label = Some(label.into());
        self
    }

    /// Announce this field as a spin button with `value` in `min..=max`.
    pub fn with_numeric_range(mut self, value: f64, min: f64, max: f64, step: f64) -> Self {
        self.numeric = Some(Numeric { value, min, max, step });
        self
    }

    /// Keep the announced value in step with the setting.
    pub fn set_numeric_value(&mut self, value: f64, cx: &mut Context<Self>) {
        if let Some(numeric) = &mut self.numeric
            && numeric.value != value
        {
            numeric.value = value;
            cx.notify();
        }
    }

    pub fn set_aria_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.aria_label = Some(label.into());
        cx.notify();
    }

    /// Emit `BackspaceAtStart` at the start instead of ringing the bell.
    pub fn report_backspace_at_start(mut self) -> Self {
        self.report_backspace_at_start = true;
        self
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        let placeholder = placeholder.into();
        if self.placeholder == placeholder {
            return;
        }
        self.placeholder = placeholder;
        cx.notify();
    }

    /// Replace the whole text and put the cursor at its end.
    pub fn set_text(&mut self, text: impl Into<SharedString>, window: &mut Window, cx: &mut Context<Self>) {
        self.record(EditKind::Other, &(0..self.content.len()));
        self.history.last = None;
        self.content = text.into();
        self.selected_range = self.content.len()..self.content.len();
        self.selection_reversed = false;
        self.marked_range = None;
        self.touch_caret(cx);
        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    /// Select `range` (byte offsets, clamped to the text).
    pub fn select_range(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let end = range.end.min(self.content.len());
        self.selected_range = range.start.min(end)..end;
        self.selection_reversed = false;
        self.touch_caret(cx);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Confirmed);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::Cancelled);
    }

    fn complete(&mut self, _: &Complete, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(TextInputEvent::CompletionRequested);
    }

    fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), window, cx);
        } else {
            self.move_to(self.selected_range.start, window, cx)
        }
    }

    fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), window, cx);
        } else {
            self.move_to(self.selected_range.end, window, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), window, cx);
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), window, cx);
    }

    /// ⌥← as in every Mac text field: to the start of the word before the caret.
    fn word_left(&mut self, _: &WordLeft, window: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selected_range.is_empty() { self.cursor_offset() } else { self.selected_range.start };
        self.move_to(previous_word_start(&self.content, from), window, cx);
    }

    /// ⌥→: to the end of the word after the caret.
    fn word_right(&mut self, _: &WordRight, window: &mut Window, cx: &mut Context<Self>) {
        let from = if self.selected_range.is_empty() { self.cursor_offset() } else { self.selected_range.end };
        self.move_to(next_word_end(&self.content, from), window, cx);
    }

    fn select_word_left(&mut self, _: &SelectWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(previous_word_start(&self.content, self.cursor_offset()), window, cx);
    }

    fn select_word_right(&mut self, _: &SelectWordRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(next_word_end(&self.content, self.cursor_offset()), window, cx);
    }

    /// ⌥⌫: delete back to the start of the word (or the selection).
    fn delete_word_left(&mut self, _: &DeleteWordLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let start = previous_word_start(&self.content, self.cursor_offset());
            if start == self.cursor_offset() {
                self.refuse(cx);
                return;
            }
            self.select_to(start, window, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// ⌥⌦: delete forward to the end of the word (or the selection).
    fn delete_word_right(&mut self, _: &DeleteWordRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let end = next_word_end(&self.content, self.cursor_offset());
            if end == self.cursor_offset() {
                self.refuse(cx);
                return;
            }
            self.select_to(end, window, cx);
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
        self.select_to(self.content.len(), window, cx)
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                if self.report_backspace_at_start {
                    cx.emit(TextInputEvent::BackspaceAtStart);
                } else {
                    self.refuse(cx);
                }
                return;
            }
            self.select_to(prev, window, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                self.refuse(cx);
                return;
            }
            self.select_to(next, window, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        window.request_virtual_keyboard();

        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), window, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        }
    }

    fn show_character_palette(&mut self, _: &ShowCharacterPalette, window: &mut Window, _: &mut Context<Self>) {
        window.show_character_palette();
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(self.selected_range.clone(), &text.replace("\n", " "), EditKind::Other, window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
        }
    }
    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(self.content[self.selected_range.clone()].to_string()));
            self.edit(self.selected_range.clone(), "", EditKind::Other, window, cx)
        }
    }

    // Crow's menu items dispatch the file actions. Inside a text field they mean the
    // field's own text operations, as in Finder.
    fn copy_files(&mut self, _: &CrowCopyFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.copy(&Copy, window, cx);
    }

    fn cut_files(&mut self, _: &CrowCutFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.cut(&Cut, window, cx);
    }

    fn paste_files(&mut self, _: &CrowPasteFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.paste(&Paste, window, cx);
    }

    fn crow_select_all(&mut self, _: &CrowSelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.select_all(&SelectAll, window, cx);
    }

    /// Replace `range` with `new_text` as one user edit, recorded for undo.
    fn edit(
        &mut self,
        range: Range<usize>,
        new_text: &str,
        kind: EditKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Committing a composition finishes the step its marked text started.
        if self.marked_range.is_none() {
            self.record(kind, &range);
        }
        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        let cursor = range.start + new_text.len();
        self.selected_range = cursor..cursor;
        self.selection_reversed = false;
        self.marked_range.take();
        self.history.last = Some((kind, cursor));
        self.touch_caret(cx);
        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    /// Save the state before an edit of `kind` over `range`, unless the edit continues the
    /// current run: typing at the cursor after typing, or deleting next to it after deleting.
    fn record(&mut self, kind: EditKind, range: &Range<usize>) {
        let continues = match (self.history.last, kind) {
            (Some((EditKind::Typing, cursor)), EditKind::Typing) => range.is_empty() && range.start == cursor,
            (Some((EditKind::Deleting, cursor)), EditKind::Deleting) => range.end == cursor || range.start == cursor,
            _ => false,
        };
        if !continues {
            if self.history.undo.len() == HISTORY_LIMIT {
                self.history.undo.remove(0);
            }
            self.history.undo.push(self.snapshot());
        }
        self.history.redo.clear();
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { content: self.content.clone(), selected_range: self.selected_range.clone() }
    }

    fn restore(&mut self, snapshot: Snapshot, window: &mut Window, cx: &mut Context<Self>) {
        self.content = snapshot.content;
        let end = self.content.len();
        self.selected_range = snapshot.selected_range.start.min(end)..snapshot.selected_range.end.min(end);
        self.selection_reversed = false;
        self.marked_range = None;
        self.history.last = None;
        self.touch_caret(cx);
        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    // Handled here even with nothing to undo, so ⌘Z in a field never reaches the file Undo.
    fn undo(&mut self, _: &CrowUndo, window: &mut Window, cx: &mut Context<Self>) {
        match self.history.undo.pop() {
            Some(snapshot) => {
                self.history.redo.push(self.snapshot());
                self.restore(snapshot, window, cx);
            }
            None => self.refuse(cx),
        }
    }

    fn redo(&mut self, _: &CrowRedo, window: &mut Window, cx: &mut Context<Self>) {
        match self.history.redo.pop() {
            Some(snapshot) => {
                self.history.undo.push(self.snapshot());
                self.restore(snapshot, window, cx);
            }
            None => self.refuse(cx),
        }
    }

    /// Start the field's blink timer. It lives as long as the input and only
    /// notifies while the field has focus.
    fn start_blink_loop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.blink_loop {
            return;
        }
        self.blink_loop = true;
        cx.spawn_in(window, async move |this, cx| {
            loop {
                let delay = this
                    .update(cx, |input, cx| {
                        let delay = input.blink_due.saturating_duration_since(cx.background_executor().now());
                        if delay.is_zero() { Duration::from_millis(8) } else { delay }
                    })
                    .unwrap_or(CARET_BLINK_INTERVAL);
                cx.background_executor().timer(delay).await;
                let keep_going = this
                    .update_in(cx, |input, window, cx| {
                        let now = cx.background_executor().now();
                        if !input.focus_handle.is_focused(window) {
                            // Not drawn anyway; keep the state honest for the next focus.
                            input.caret_visible = false;
                            input.blink_due = now + CARET_BLINK_INTERVAL;
                            return true;
                        }
                        if input.marked_range.is_some() {
                            input.caret_visible = true;
                            input.blink_due = now + CARET_BLINK_INTERVAL;
                            return true;
                        }
                        if now < input.blink_due {
                            return true;
                        }
                        input.caret_visible = !input.caret_visible;
                        input.blink_due = now + CARET_BLINK_INTERVAL;
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_going {
                    break;
                }
            }
        })
        .detach();
    }

    /// Whether the caret is currently in its "on" phase. Tests read this.
    #[cfg(test)]
    pub(crate) fn caret_visible(&self) -> bool {
        self.caret_visible
    }

    /// Every edit and caret move turns the caret back on and pushes the next
    /// blink phase out, so the caret stays solid while typing.
    fn touch_caret(&mut self, cx: &App) {
        self.caret_visible = true;
        self.blink_due = cx.background_executor().now() + CARET_BLINK_INTERVAL;
    }

    fn move_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        self.touch_caret(cx);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed { self.selected_range.start } else { self.selected_range.end }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        // The line is painted inset and shifted left by `scroll_x`; undo both before hit-testing.
        line.closest_index_for_x(position.x - bounds.left() - px(TEXT_INSET) + self.scroll_x)
    }

    fn select_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        self.touch_caret(cx);
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content.grapheme_indices(true).rev().find_map(|(idx, _)| (idx < offset).then_some(idx)).unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.range_to_utf16(&self.selected_range), reversed: self.selection_reversed })
    }

    fn set_selected_text_range(&mut self, range_utf16: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let kind = if new_text.is_empty() { EditKind::Deleting } else { EditKind::Typing };
        self.edit(range, new_text, kind, window, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        // A composition (IME, dead keys) is one undo step from its first marked text to its commit.
        if self.marked_range.is_none() {
            self.record(EditKind::Typing, &range);
        }
        self.content = (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        let insertion_start_utf16 = self.offset_to_utf16(range.start);
        let inserted_length_utf16 = new_text.encode_utf16().count();
        self.selected_range = new_selected_range_utf16
            .map(|selected| {
                self.range_from_utf16(
                    &(insertion_start_utf16 + selected.start.min(inserted_length_utf16)
                        ..insertion_start_utf16 + selected.end.min(inserted_length_utf16)),
                )
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        self.touch_caret(cx);

        window.notify_text_input_changed(&self.focus_handle);
        cx.emit(TextInputEvent::Changed);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        let shown = crate::style::display_text(&self.content);
        if last_layout.text.as_ref() != shown.as_ref() {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        Some(Bounds::from_corners(
            point(bounds.left() + px(TEXT_INSET) + last_layout.x_for_index(range.start) - self.scroll_x, bounds.top()),
            point(bounds.left() + px(TEXT_INSET) + last_layout.x_for_index(range.end) - self.scroll_x, bounds.bottom()),
        ))
    }

    fn selection_bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Bounds<Pixels>> {
        self.bounds_for_range(range_utf16, bounds, window, cx).into_iter().collect()
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;

        let shown = crate::style::display_text(&self.content);
        if last_layout.text.as_ref() != shown.as_ref() {
            return None;
        }
        let utf8_index = last_layout.closest_index_for_x(line_point.x + self.scroll_x - px(TEXT_INSET));
        Some(self.offset_to_utf16(utf8_index))
    }
}

struct TextElement {
    input: Entity<TextInput>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    /// The line's vertical offset inside the field's bounds (see `prepaint`).
    text_top: Pixels,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
    scroll_x: Pixels,
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let style = window.text_style();
        let theme = &cx.global::<Ui>().theme;
        let placeholder_role = if input.chrome { "fg-subtle" } else { "fg-muted" };
        let (placeholder_color, cursor_color, selection_color) =
            (color(theme.role(placeholder_role)), color(theme.role("accent")), color_alpha(theme.role("accent"), 0x40));

        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), gpui::Hsla::from(placeholder_color))
        } else {
            (crate::style::display_text(&content).into_owned().into(), style.color)
        };

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = input.marked_range.as_ref() {
            vec![
                TextRun { len: marked_range.start, ..run.clone() },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(UnderlineStyle { color: Some(run.color), thickness: px(1.0), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: display_text.len() - marked_range.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(display_text, font_size, &runs, None);

        // Center the lowercase band, not ascent+descent: the ascent reserves room
        // for accents, which leaves the text reading a little low in the field.
        let x_height =
            line.runs.first().map_or(px(0.), |run| window.text_system().x_height(run.font_id, line.font_size));
        let text_top = (x_height - (line.ascent - line.descent)) / 2.;

        let cursor_pos = line.x_for_index(cursor);
        let text_width = bounds.size.width - px(TEXT_INSET);
        let scroll_x = scroll_for_caret(cursor_pos, line.width, text_width, input.scroll_x);
        // The caret sits at the insertion point; the text is inset, so the caret
        // lands just before the first glyph instead of on it.
        let caret_x = |index| bounds.left() + line.x_for_index(index) - scroll_x;
        let text_x = bounds.left() + px(TEXT_INSET) - scroll_x;
        let (selection, cursor) = if selected_range.is_empty() {
            (
                None,
                Some(fill(
                    Bounds::new(point(caret_x(cursor), bounds.top()), size(px(2.), bounds.bottom() - bounds.top())),
                    cursor_color,
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(text_x + line.x_for_index(selected_range.start), bounds.top()),
                        point(text_x + line.x_for_index(selected_range.end), bounds.bottom()),
                    ),
                    selection_color,
                )),
                None,
            )
        };
        PrepaintState { line: Some(line), text_top, scroll_x, selection, cursor }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let scroll_x = prepaint.scroll_x;
        let text_top = prepaint.text_top;
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepaint.selection.take() {
                window.paint_quad(selection)
            }
            let Some(line) = prepaint.line.take() else { return };
            let origin = point(bounds.left() + px(TEXT_INSET) - scroll_x, bounds.top() + text_top);
            if let Err(e) = line.paint(origin, window.line_height(), gpui::TextAlign::Left, None, window, cx) {
                tracing::warn!("text input paint: {e}");
            }
            let focused = focus_handle.is_focused(window);
            let caret_shown = focused && {
                let input = self.input.read(cx);
                // A frame that first sees focus shows the caret even before the loop's next tick.
                input.caret_visible || input.marked_range.is_some() || !input.was_focused
            };
            if caret_shown && let Some(cursor) = prepaint.cursor.take() {
                window.paint_quad(cursor);
            }
            self.input.update(cx, |input, ctx| {
                if input.was_focused != focused {
                    input.was_focused = focused;
                    input.caret_visible = focused;
                    input.blink_due = ctx.background_executor().now() + CARET_BLINK_INTERVAL;
                }
                input.last_layout = Some(line);
                input.last_bounds = Some(bounds);
                input.scroll_x = scroll_x;
            });
        });
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.start_blink_loop(window, cx);
        // The refuse shake is drawn from the clock: keep frames coming while it runs.
        if self.refuse.is_some() {
            window.request_animation_frame();
        }
        let theme = cx.global::<Ui>().theme.clone();
        let chrome = self.chrome;
        let numeric = self.numeric;
        let mode = cx.global::<Ui>().feedback.mode;
        let now = Instant::now();
        let refuse_look = self.refuse.and_then(|(_, started)| {
            if mode == Effective::Off {
                return None;
            }
            let phases = motion::phases(mode, Fx::Refuse);
            Some(motion::look_in(&phases, mode, Fx::Refuse, started, now))
        });
        let tint = refuse_look.and_then(|look| effect_tint(look, &theme));
        let border = match refuse_look {
            Some(look) if look.tint > 0.0 => color(theme.role("error")),
            _ => color(theme.role("accent")),
        };
        div()
            .id(self.id.clone())
            .role(self.role)
            .aria_label(self.aria_label.clone().unwrap_or_else(|| self.placeholder.clone()))
            .aria_placeholder(self.placeholder.clone())
            .aria_value(self.content.clone())
            .when_some(numeric, |field, numeric| {
                field
                    .aria_numeric_value(numeric.value)
                    .aria_min_numeric_value(numeric.min)
                    .aria_max_numeric_value(numeric.max)
                    .aria_numeric_value_step(numeric.step)
            })
            .flex()
            .relative()
            .when_some(refuse_look, |field, look| field.left(px(look.dx)))
            // keys::context_stack adds TextInput for Filter, Search and Palette.
            .key_context("TextInput")
            .track_focus(&self.focus_handle(cx))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::delete_word_left))
            .on_action(cx.listener(Self::delete_word_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::show_character_palette))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::copy_files))
            .on_action(cx.listener(Self::cut_files))
            .on_action(cx.listener(Self::paste_files))
            .on_action(cx.listener(Self::crow_select_all))
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::redo))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::complete))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .w_full()
            .when(chrome, |field| {
                field.px_2().py_1().rounded_md().bg(color(theme.role("bg"))).border_1().border_color(border)
            })
            .when_some(tint, |field, tint| field.child(div().absolute().inset_0().rounded_md().bg(tint)))
            .child(TextElement { input: cx.entity() })
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// How far a single-line field scrolls so the caret stays inside it. `text` is the
/// shaped line's width and `width` the text area; a 2 px margin keeps the caret's own
/// 2 px quad visible. The offset moves only as far as the caret needs, and never past
/// the text's end, so deleting from the end of a long text keeps the field full and a
/// text that fits always sits at offset 0.
fn scroll_for_caret(caret: Pixels, text: Pixels, width: Pixels, current: Pixels) -> Pixels {
    let room = width - px(2.);
    if room <= px(0.) {
        return px(0.);
    }
    let mut scroll = current;
    if caret < scroll {
        scroll = caret;
    } else if caret > scroll + room {
        scroll = caret - room;
    }
    let max = (text - room).max(px(0.));
    scroll.clamp(px(0.), max)
}

/// Letters, digits and `_` make words; `m5-fix.md` is three of them, as in TextEdit.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The start of the word before `offset`, skipping any separators first.
fn previous_word_start(text: &str, offset: usize) -> usize {
    let mut chars = text[..offset].char_indices().rev().peekable();
    while chars.next_if(|(_, c)| !is_word_char(*c)).is_some() {}
    let mut start = chars.peek().map_or(0, |(i, c)| i + c.len_utf8());
    while let Some((i, _)) = chars.next_if(|(_, c)| is_word_char(*c)) {
        start = i;
    }
    start
}

/// The end of the word after `offset`, skipping any separators first.
fn next_word_end(text: &str, offset: usize) -> usize {
    let mut chars = text[offset..].char_indices().map(|(i, c)| (i + offset, c)).peekable();
    while chars.next_if(|(_, c)| !is_word_char(*c)).is_some() {}
    let mut end = chars.peek().map_or(text.len(), |(i, _)| *i);
    while let Some((i, c)) = chars.next_if(|(_, c)| is_word_char(*c)) {
        end = i + c.len_utf8();
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_boundaries_follow_mac_fields() {
        let text = "m5-fix.md  notes";
        assert_eq!(previous_word_start(text, text.len()), 11, "back over \"notes\"");
        assert_eq!(previous_word_start(text, 11), 7, "spaces and the dot are skipped");
        assert_eq!(previous_word_start(text, 7), 3);
        assert_eq!(previous_word_start(text, 3), 0);
        assert_eq!(previous_word_start(text, 0), 0);
        assert_eq!(next_word_end(text, 0), 2);
        assert_eq!(next_word_end(text, 2), 6);
        assert_eq!(next_word_end(text, 9), text.len());
        assert_eq!(next_word_end(text, text.len()), text.len());
        assert_eq!(next_word_end("über straße", 0), "über".len(), "non-ASCII letters are word characters");
        assert_eq!(previous_word_start("   ", 3), 0, "only separators: to the start");
    }

    #[test]
    fn scroll_for_caret_keeps_the_caret_visible() {
        // Caret at the end of a long text, beyond the right edge: scroll just enough.
        assert_eq!(scroll_for_caret(px(100.), px(100.), px(50.), px(0.)), px(52.));
        // Caret back at the start of a long text: scroll to it.
        assert_eq!(scroll_for_caret(px(0.), px(100.), px(50.), px(52.)), px(0.));
        // Short text never scrolls, even after a long one.
        assert_eq!(scroll_for_caret(px(10.), px(10.), px(50.), px(52.)), px(0.));
        // Caret already visible: keep the offset.
        assert_eq!(scroll_for_caret(px(60.), px(100.), px(50.), px(20.)), px(20.));
        // Caret left of the view in a long text: it lands at the left edge.
        assert_eq!(scroll_for_caret(px(30.), px(200.), px(50.), px(100.)), px(30.));
        // Deleting from the end of a long text: the view follows the end, so the field
        // stays full instead of showing empty space after the last character.
        assert_eq!(scroll_for_caret(px(190.), px(190.), px(50.), px(152.)), px(142.));
        assert_eq!(scroll_for_caret(px(60.), px(60.), px(50.), px(152.)), px(12.));
        // Zero width (before layout): no offset.
        assert_eq!(scroll_for_caret(px(100.), px(100.), px(0.), px(20.)), px(0.));
    }
}
