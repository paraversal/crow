//! Settings ▸ Keyboard: every command with its shortcut, searchable. Click a
//! shortcut (or press Return on its row), type the new keys, done. Changes go
//! to `keys.toml` and the live keymap follows.

use std::collections::HashSet;

use crow_core::keymap_edit::{self, assign_problem, normalize_sequence};

use super::*;
use crate::keymap::{self, Conflict, ShortcutRow};
use crate::palette::CommandId;

/// How long a Vim sequence waits for another key before it is taken as typed.
const SEQUENCE_PAUSE: Duration = Duration::from_millis(1000);

pub(super) struct Capture {
    command: CommandId,
    steps: Vec<String>,
    /// Counts keys, so a pause timer from an earlier key does nothing.
    generation: u64,
    _intercept: Subscription,
}

#[derive(Clone)]
pub(super) enum Prompt {
    Conflict { command: CommandId, sequence: String, conflicts: Vec<Conflict>, choice: usize },
    ResetAll { choice: usize },
}

pub(super) struct KeyboardState {
    pub search: Entity<TextInput>,
    pub layer: KeymapLayer,
    pub cursor: usize,
    pub scroll: ScrollHandle,
    row_items: Vec<usize>,
    pub capture: Option<Capture>,
    pub prompt: Option<Prompt>,
    /// The last thing that went wrong or was refused, shown under its row.
    pub notice: Option<(CommandId, SharedString)>,
    pub query: String,
}

impl KeyboardState {
    pub(super) fn new(cx: &mut Context<SettingsWindow>) -> Self {
        let search = cx.new(|cx| {
            TextInput::new("", "Search shortcuts", cx)
                .bare()
                .with_id("settings-keyboard-search")
                .with_role(Role::SearchInput)
                .with_aria_label("Search shortcuts")
        });
        Self {
            search,
            layer: cx.global::<Ui>().settings.keymap,
            cursor: 0,
            scroll: ScrollHandle::new(),
            row_items: Vec::new(),
            capture: None,
            prompt: None,
            notice: None,
            query: String::new(),
        }
    }
}

fn caps_of(sequence: &str, cx: &App) -> Vec<SharedString> {
    keymap::display(sequence, cx).map(|b| crate::keys::caps(&b)).unwrap_or_default()
}

fn text_of(sequence: &str, cx: &App) -> String {
    keymap::display(sequence, cx)
        .map(|b| crate::keys::chord_text(&b).to_string())
        .unwrap_or_else(|| sequence.to_owned())
}

fn spoken_of(sequence: &str, cx: &App) -> String {
    keymap::display(sequence, cx).map(|b| crate::keys::spoken(&b)).unwrap_or_else(|| sequence.to_owned())
}

/// The key text for one captured keystroke, `cmd-shift-g` style.
fn step_of(keystroke: &gpui::Keystroke) -> Option<String> {
    let m = keystroke.modifiers;
    if matches!(keystroke.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") {
        return None;
    }
    let mut out = String::new();
    for (held, name) in [(m.control, "ctrl-"), (m.platform, "cmd-"), (m.alt, "alt-"), (m.shift, "shift-")] {
        if held {
            out.push_str(name);
        }
    }
    out.push_str(&keystroke.key);
    Some(out)
}

impl SettingsWindow {
    pub(super) fn keyboard_rows(&self, cx: &App) -> Vec<ShortcutRow> {
        let k = &self.keyboard;
        let query = k.query.to_lowercase();
        let rows = keymap::rows(k.layer, cx);
        if query.is_empty() {
            return rows;
        }
        let duplicates = self.keyboard_duplicates(cx);
        rows.into_iter()
            .filter(|row| {
                let mut haystack = format!("{} {}", row.label, row.group).to_lowercase();
                for key in &row.keys {
                    haystack.push(' ');
                    haystack.push_str(&text_of(key, cx).to_lowercase());
                    haystack.push(' ');
                    haystack.push_str(&spoken_of(key, cx).to_lowercase());
                }
                if duplicates.contains(&row.id) {
                    haystack.push_str(" conflict duplicate");
                }
                if row.keys.is_empty() {
                    haystack.push_str(" unbound none");
                }
                query.split_whitespace().all(|token| haystack.contains(token))
            })
            .collect()
    }

    fn keyboard_duplicates(&self, cx: &App) -> HashSet<CommandId> {
        keymap::duplicates(self.keyboard.layer, cx)
    }

    pub(super) fn on_keyboard_search_event(
        &mut self,
        event: &TextInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TextInputEvent::Changed => {
                let text = self.keyboard.search.read(cx).text().trim().to_owned();
                if text != self.keyboard.query {
                    self.keyboard.query = text;
                    self.keyboard.cursor = 0;
                    cx.notify();
                }
            }
            TextInputEvent::Confirmed | TextInputEvent::CompletionRequested => {
                window.focus(&self.handle(ControlId::KeyboardList), cx);
            }
            TextInputEvent::Cancelled => {
                if self.keyboard.query.is_empty() {
                    self.cancel(window, cx);
                } else {
                    self.keyboard.search.update(cx, |input, cx| input.set_text("", window, cx));
                }
            }
            TextInputEvent::BackspaceAtStart => {}
        }
    }

    pub(super) fn focus_keyboard_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.keyboard.search.clone();
        let len = input.read(cx).text().len();
        input.update(cx, |input, cx| input.select_range(0..len, window, cx));
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
        cx.notify();
    }

    pub(super) fn move_keyboard_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.keyboard.capture.is_some() {
            return;
        }
        let len = self.keyboard_rows(cx).len();
        if len == 0 {
            return;
        }
        let next = (self.keyboard.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        self.keyboard.cursor = next;
        if let Some(&item) = self.keyboard.row_items.get(next) {
            self.keyboard.scroll.scroll_to_item(item);
        }
        cx.notify();
    }

    /// ← → on the list: the layer switch, or the choice in an open question.
    pub(super) fn keyboard_horizontal(&mut self, delta: isize, cx: &mut Context<Self>) {
        match &mut self.keyboard.prompt {
            Some(Prompt::Conflict { choice, conflicts, .. }) => {
                let replaceable = conflicts.iter().all(|c| c.command.is_some());
                let options: &[usize] = if replaceable { &[0, 1, 2] } else { &[1, 2] };
                let at = options.iter().position(|o| o == choice).unwrap_or(0) as isize;
                *choice = options[(at + delta).clamp(0, options.len() as isize - 1) as usize];
            }
            Some(Prompt::ResetAll { choice }) => *choice = (*choice as isize + delta).clamp(0, 1) as usize,
            None if self.keyboard.capture.is_none() => {
                let layer = if delta < 0 { KeymapLayer::Finder } else { KeymapLayer::Vim };
                self.set_keyboard_layer(layer, cx);
            }
            None => {}
        }
        cx.notify();
    }

    pub(super) fn set_keyboard_layer(&mut self, layer: KeymapLayer, cx: &mut Context<Self>) {
        if self.keyboard.layer != layer {
            self.keyboard.layer = layer;
            self.keyboard.cursor = 0;
            self.keyboard.notice = None;
            self.keyboard.prompt = None;
            self.cancel_capture(cx);
            cx.notify();
        }
    }

    pub(super) fn keyboard_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        let busy = self.keyboard.capture.is_some() || self.keyboard.prompt.is_some() || self.keyboard.notice.is_some();
        self.keyboard.capture = None;
        self.keyboard.prompt = None;
        self.keyboard.notice = None;
        if busy {
            cx.notify();
        }
        busy
    }

    pub(super) fn cancel_capture(&mut self, cx: &mut Context<Self>) {
        if self.keyboard.capture.take().is_some() {
            cx.notify();
        }
    }

    /// Return or a click on a row: listen for the new shortcut.
    pub(super) fn begin_capture(&mut self, command: CommandId, window: &mut Window, cx: &mut Context<Self>) {
        if cx.global::<Ui>().file_error("keys.toml").is_some() {
            self.keyboard.notice =
                Some((command, "Fix keys.toml first; Settings will not overwrite a broken file.".into()));
            cx.notify();
            return;
        }
        self.keyboard.notice = None;
        self.keyboard.prompt = None;
        let weak = cx.weak_entity();
        let handle = window.window_handle();
        let intercept = cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() != handle {
                return;
            }
            let Some(this) = weak.upgrade() else { return };
            // Nothing else may see the key: not a binding, not the menu.
            cx.stop_propagation();
            let keystroke = event.keystroke.clone();
            this.update(cx, |this, cx| this.captured(&keystroke, window, cx));
        });
        window.focus(&self.handle(ControlId::KeyboardList), cx);
        self.keyboard.capture = Some(Capture { command, steps: Vec::new(), generation: 0, _intercept: intercept });
        cx.notify();
    }

    fn captured(&mut self, keystroke: &gpui::Keystroke, window: &mut Window, cx: &mut Context<Self>) {
        let Some(capture) = &mut self.keyboard.capture else { return };
        let m = keystroke.modifiers;
        let bare = !m.control && !m.platform && !m.alt && !m.function && !m.shift;
        let command = capture.command;
        if bare && keystroke.key == "escape" {
            self.cancel_capture(cx);
            return;
        }
        if bare && matches!(keystroke.key.as_str(), "backspace" | "delete") && capture.steps.is_empty() {
            self.cancel_capture(cx);
            self.write_keys(command, Vec::new(), Vec::new(), cx);
            return;
        }
        if bare && keystroke.key == "enter" && !capture.steps.is_empty() {
            self.finish_capture(cx);
            return;
        }
        let Some(step) = step_of(keystroke) else { return };
        capture.steps.push(step);
        capture.generation += 1;
        let generation = capture.generation;
        if self.keyboard.layer == KeymapLayer::Finder {
            self.finish_capture(cx);
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(SEQUENCE_PAUSE).await;
            this.update(cx, |this, cx| {
                if this.keyboard.capture.as_ref().is_some_and(|c| c.generation == generation) {
                    this.finish_capture(cx);
                }
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn finish_capture(&mut self, cx: &mut Context<Self>) {
        let Some(capture) = self.keyboard.capture.take() else { return };
        let command = capture.command;
        let layer = self.keyboard.layer;
        let sequence = match normalize_sequence(&capture.steps.join(" ")) {
            Ok(sequence) => sequence,
            Err(error) => {
                self.keyboard.notice = Some((command, error.into()));
                cx.notify();
                return;
            }
        };
        if let Some(problem) = assign_problem(layer, &sequence) {
            self.keyboard.notice = Some((command, problem.into()));
            cx.notify();
            return;
        }
        let current = keymap::rows(layer, cx).into_iter().find(|row| row.id == command);
        if current.is_some_and(|row| row.keys == [sequence.clone()]) {
            cx.notify();
            return;
        }
        let conflicts = keymap::conflicts(layer, command, &sequence, cx);
        if conflicts.is_empty() {
            self.write_keys(command, vec![sequence], Vec::new(), cx);
        } else {
            let choice = if conflicts.iter().all(|c| c.command.is_some()) { 0 } else { 1 };
            self.keyboard.prompt = Some(Prompt::Conflict { command, sequence, conflicts, choice });
            cx.notify();
        }
    }

    /// Answer the open question with its highlighted choice.
    pub(super) fn keyboard_answer(&mut self, cx: &mut Context<Self>) {
        match self.keyboard.prompt.take() {
            Some(Prompt::Conflict { command, sequence, conflicts, choice }) => match choice {
                0 => {
                    let remove: Vec<(CommandId, String)> =
                        conflicts.iter().filter_map(|c| Some((c.command?, c.key.clone()))).collect();
                    self.write_keys(command, vec![sequence], remove, cx);
                }
                1 => self.write_keys(command, vec![sequence], Vec::new(), cx),
                _ => cx.notify(),
            },
            Some(Prompt::ResetAll { choice: 0 }) => self.reset_all_keys(cx),
            Some(Prompt::ResetAll { .. }) | None => cx.notify(),
        }
    }

    pub(super) fn answer_prompt_with(&mut self, choice: usize, cx: &mut Context<Self>) {
        match &mut self.keyboard.prompt {
            Some(Prompt::Conflict { choice: c, .. } | Prompt::ResetAll { choice: c }) => *c = choice,
            None => return,
        }
        self.keyboard_answer(cx);
    }

    /// Set `command`'s keys; `remove` takes keys off the commands they were taken from.
    fn write_keys(
        &mut self,
        command: CommandId,
        keys: Vec<String>,
        remove: Vec<(CommandId, String)>,
        cx: &mut Context<Self>,
    ) {
        let layer = self.keyboard.layer;
        let path = cx.global::<Ui>().config_dir.join("keys.toml");
        let rows = keymap::rows(layer, cx);
        let mut writes: Vec<(String, Vec<String>)> = Vec::new();
        for (other, key) in remove {
            if let Some(row) = rows.iter().find(|row| row.id == other) {
                let kept: Vec<String> =
                    row.keys.iter().filter(|k| !keymap_edit::sequences_conflict(k, &key)).cloned().collect();
                writes.push((other.key().to_owned(), kept));
            }
        }
        writes.push((command.key().to_owned(), keys));
        self.keyboard.notice = None;
        self.write_key_edits(
            command,
            // One file rewrite for the conflict removals and the new keys together.
            move |path| keymap_edit::set_many(path, layer, &writes),
            path,
            cx,
        );
    }

    pub(super) fn reset_keyboard_row(&mut self, command: CommandId, cx: &mut Context<Self>) {
        if cx.global::<Ui>().file_error("keys.toml").is_some() {
            return;
        }
        let layer = self.keyboard.layer;
        let path = cx.global::<Ui>().config_dir.join("keys.toml");
        self.write_key_edits(command, move |path| keymap_edit::reset_command(path, layer, command.key()), path, cx);
    }

    pub(super) fn ask_reset_all(&mut self, cx: &mut Context<Self>) {
        self.keyboard.prompt = Some(Prompt::ResetAll { choice: 1 });
        cx.notify();
    }

    fn reset_all_keys(&mut self, cx: &mut Context<Self>) {
        let layer = self.keyboard.layer;
        let path = cx.global::<Ui>().config_dir.join("keys.toml");
        let command = self.keyboard_rows(cx).first().map_or(CommandId::Quit, |row| row.id);
        self.write_key_edits(command, move |path| keymap_edit::reset_all(path, Some(layer)), path, cx);
    }

    /// Run the file edit off the main thread, then reload the keymap.
    fn write_key_edits(
        &mut self,
        command: CommandId,
        edit: impl FnOnce(&std::path::Path) -> Result<(), String> + Send + 'static,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        // Behind any earlier edit of keys.toml: rapid resets must not read the file
        // while the previous one is still writing it.
        let task = ui::queue_file_write("keys.toml", move || edit(&path), cx);
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.keyboard.notice = Some((command, error.into()));
                }
                ui::reload_keys(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn keyboard_activate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.keyboard.prompt.is_some() {
            self.keyboard_answer(cx);
            return;
        }
        if self.keyboard.capture.is_some() {
            return;
        }
        let rows = self.keyboard_rows(cx);
        if let Some(row) = rows.get(self.keyboard.cursor) {
            self.begin_capture(row.id, window, cx);
        }
    }

    pub(super) fn keyboard_reset_cursor_row(&mut self, cx: &mut Context<Self>) {
        if self.keyboard.capture.is_some() || self.keyboard.prompt.is_some() {
            return;
        }
        let rows = self.keyboard_rows(cx);
        if let Some(row) = rows.get(self.keyboard.cursor).filter(|row| row.changed) {
            self.reset_keyboard_row(row.id, cx);
        }
    }

    /// `fill`: the Keyboard page itself, where the list takes the height left
    /// under the controls. In search results the page scrolls, so the list is capped.
    pub(super) fn render_keyboard(
        &mut self,
        fill: bool,
        window: &Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let k = &self.keyboard;
        let rows = self.keyboard_rows(cx);
        let duplicates = self.keyboard_duplicates(cx);
        let list = ControlId::KeyboardList;
        let focused = self.is_focused(list, window);
        let muted = color(theme.role("fg-muted"));
        let broken = cx.global::<Ui>().file_error("keys.toml").is_some();
        let any_changed = keymap::rows(k.layer, cx).iter().any(|row| row.changed);
        let cursor = k.cursor.min(rows.len().saturating_sub(1));
        let total = rows.len();

        let layer_option = |layer: KeymapLayer, label: &'static str, cx: &mut Context<Self>| {
            let selected = k.layer == layer;
            div()
                .id(SharedString::from(format!("settings-keyboard-layer-{label}")))
                .debug_selector(move || format!("settings-keyboard-layer-{label}"))
                .role(Role::RadioButton)
                .aria_label(label)
                .aria_selected(selected)
                .px_3()
                .h(px(24.))
                .flex()
                .items_center()
                .rounded(px(6.))
                .cursor_pointer()
                .text_color(if selected { color(theme.role("fg")) } else { muted })
                .when(selected, |o| o.bg(color(theme.role("surface"))))
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| this.set_keyboard_layer(layer, cx)))
        };
        let switch = div()
            .id("settings-keyboard-layer")
            .role(Role::RadioGroup)
            .aria_label("Keymap layer")
            .flex()
            .gap_1()
            .child(layer_option(KeymapLayer::Finder, "Finder", cx))
            .child(layer_option(KeymapLayer::Vim, "Vim", cx));

        let search_focused = self.keyboard.search.read(cx).focus_handle(cx).is_focused(window);
        let search = div()
            .id("settings-keyboard-search-row")
            .debug_selector(|| "settings-keyboard-search-row".to_owned())
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(7.))
            .h(px(28.))
            .pl(px(9.))
            .pr(px(8.))
            .rounded(px(7.))
            .bg(color(theme.role("surface")))
            .border_1()
            .border_color(if search_focused { color(theme.role("fg-subtle")) } else { color(theme.role("surface")) })
            .child(glyph(Glyph::Search, 13., "fg-muted", theme))
            .child(div().flex_1().min_w_0().child(k.search.clone()));

        let reset_all = controls::link_button(
            ControlId::KeyboardResetAll.key(),
            "Reset all",
            any_changed && !broken,
            self.style(ControlId::KeyboardResetAll, window, theme),
            callback(cx, |this, _, cx| this.ask_reset_all(cx)),
        );

        let prompt_row = |text: String, options: Vec<(&'static str, usize)>, choice: usize, cx: &mut Context<Self>| {
            div()
                .id("settings-keyboard-prompt")
                .role(Role::Status)
                .aria_label(text.clone())
                .flex()
                .items_center()
                .flex_wrap()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(color(theme.role("surface")))
                .child(div().flex_1().min_w_0().text_color(color(theme.role("warning"))).child(text))
                .children(options.into_iter().map(|(label, index)| {
                    div()
                        .id(SharedString::from(format!("settings-keyboard-answer-{index}")))
                        .debug_selector(move || format!("settings-keyboard-answer-{index}"))
                        .role(Role::Button)
                        .aria_label(label)
                        .px_2()
                        .h(px(22.))
                        .flex()
                        .items_center()
                        .rounded(px(5.))
                        .cursor_pointer()
                        .border_1()
                        .border_color(color(theme.role(if choice == index { "accent" } else { "border" })))
                        .hover(|b| b.bg(color(theme.role("bg"))))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| this.answer_prompt_with(index, cx)))
                }))
        };

        let mut items: Vec<gpui::AnyElement> = Vec::new();
        let mut row_items = Vec::with_capacity(rows.len());
        let mut last_group = "";
        for (index, row) in rows.iter().enumerate() {
            if row.group != last_group {
                last_group = row.group;
                items.push(
                    div()
                        .id(SharedString::from(format!("settings-keyboard-group-{}", row.group)))
                        .role(Role::Heading)
                        .aria_level(4)
                        .px_2()
                        .pt_2()
                        .text_size(px(11.))
                        .text_color(muted)
                        .child(row.group)
                        .into_any_element(),
                );
            }
            row_items.push(items.len());
            let current = index == cursor;
            let capturing = k.capture.as_ref().is_some_and(|c| c.command == row.id);
            let asking = matches!(&k.prompt, Some(Prompt::Conflict { command, .. }) if *command == row.id);
            let duplicate = duplicates.contains(&row.id);
            let id = row.id;
            let spoken_keys = if row.keys.is_empty() {
                "no shortcut".to_owned()
            } else {
                row.keys.iter().map(|key| spoken_of(key, cx)).collect::<Vec<_>>().join(" or ")
            };
            let label = if capturing {
                format!("{}, press the new shortcut, Escape to cancel", row.label)
            } else {
                format!(
                    "{}, shortcut {}{}",
                    row.label,
                    spoken_keys,
                    if duplicate { ", also used elsewhere" } else { "" }
                )
            };
            let keys_el: gpui::AnyElement = if capturing {
                let steps = k.capture.as_ref().map(|c| c.steps.join(" ")).unwrap_or_default();
                let shown = if steps.is_empty() { "Type shortcut…".to_owned() } else { text_of(&steps, cx) };
                div()
                    .px_2()
                    .rounded(px(5.))
                    .shadow(cursor_ring(theme))
                    .text_color(color(theme.role("fg")))
                    .child(shown)
                    .into_any_element()
            } else if row.keys.is_empty() {
                div().text_color(muted).child("—").into_any_element()
            } else {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(row.keys.iter().map(|key| {
                        let text = caps_of(key, cx).concat();
                        crate::chrome::Keycap::text(text, crate::chrome::KeycapWeight::Outline)
                    }))
                    .into_any_element()
            };
            let reset: Option<gpui::AnyElement> = row.changed.then(|| {
                div()
                    .id(SharedString::from(format!("settings-keyboard-reset-{}", id.key())))
                    .debug_selector(move || format!("settings-keyboard-reset-{}", id.key()))
                    .role(Role::Button)
                    .aria_label(format!("Reset {} to its default shortcut", row.label))
                    .aria_disabled(broken)
                    .px_1()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child("Reset")
                    .when(!broken, |button| {
                        button.cursor_pointer().hover(|b| b.text_color(color(theme.role("fg")))).on_click(cx.listener(
                            move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.reset_keyboard_row(id, cx);
                            },
                        ))
                    })
                    .into_any_element()
            });
            let mut line = div()
                .id(SharedString::from(format!("settings-keyboard-row-{}", id.key())))
                .debug_selector(move || format!("settings-keyboard-row-{}", id.key()))
                .role(Role::ListBoxOption)
                .aria_label(label)
                .aria_description("Return to change, Delete to reset")
                .aria_selected(current)
                .aria_position_in_set(index + 1)
                .aria_size_of_set(total)
                .when(current && focused, |r| r.aria_active_descendant().shadow(cursor_ring(theme)))
                .rounded_md()
                .px_2()
                .py_1()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .gap_3()
                .cursor_pointer()
                .hover(|r| r.bg(color(theme.role("surface"))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.keyboard.cursor = index;
                    this.begin_capture(id, window, cx);
                }))
                .child(div().min_w_0().truncate().child(row.label))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(duplicate, |d| {
                            d.child(div().text_size(px(11.)).text_color(color(theme.role("warning"))).child("Conflict"))
                        })
                        .children(reset)
                        .child(keys_el),
                );
            if asking {
                line = line.bg(color(theme.role("surface")));
            }
            items.push(line.into_any_element());
            if let Some(Prompt::Conflict { command, sequence, conflicts, choice }) = &k.prompt
                && *command == row.id
            {
                let names = conflicts.iter().map(|c| c.label.clone()).collect::<Vec<_>>().join(", ");
                let text = format!("{} is also used by {names}.", text_of(sequence, cx));
                let mut options = Vec::new();
                if conflicts.iter().all(|c| c.command.is_some()) {
                    options.push(("Replace", 0));
                }
                options.push(("Keep both", 1));
                options.push(("Cancel", 2));
                items.push(prompt_row(text, options, *choice, cx).into_any_element());
            }
            if let Some((command, text)) = &k.notice
                && *command == row.id
            {
                items.push(
                    div()
                        .id("settings-keyboard-notice")
                        .role(Role::Status)
                        .aria_label(text.clone())
                        .px_2()
                        .text_size(px(12.))
                        .text_color(color(theme.role("warning")))
                        .child(text.clone())
                        .into_any_element(),
                );
            }
        }
        let empty = rows.is_empty();

        let keyboard = div()
            .when(fill, |d| d.size_full())
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .id("settings-keyboard-heading")
                            .role(Role::Heading)
                            .aria_level(3)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Keyboard shortcuts"),
                    )
                    .child(reset_all),
            )
            .child(div().flex().items_center().gap_2().child(search).child(switch))
            .when(broken, |d| {
                d.child(
                    div()
                        .text_color(color(theme.role("warning")))
                        .child("keys.toml has an error. Fix it to change shortcuts here."),
                )
            })
            .children(match &k.prompt {
                Some(Prompt::ResetAll { choice }) => Some(prompt_row(
                    format!(
                        "Reset every {} shortcut to its default?",
                        if k.layer == KeymapLayer::Vim { "Vim" } else { "Finder" }
                    ),
                    vec![("Reset all", 0), ("Cancel", 1)],
                    *choice,
                    cx,
                )),
                _ => None,
            })
            .child(
                div()
                    .id(list.key())
                    .debug_selector(move || list.key().to_owned())
                    .role(Role::ListBox)
                    .aria_label("Keyboard shortcuts")
                    .track_focus(&self.handle(list))
                    .flex()
                    .flex_col()
                    .gap_1()
                    .when(fill, |d| d.flex_1().min_h_0())
                    .when(!fill, |d| d.max_h(px(440.)))
                    .overflow_y_scroll()
                    .track_scroll(&k.scroll)
                    .children(items),
            )
            .when(empty, |d| {
                d.child(
                    div()
                        .id("settings-keyboard-empty")
                        .role(Role::Status)
                        .text_color(muted)
                        .child("No shortcuts match."),
                )
            })
            .into_any_element();
        self.keyboard.row_items = row_items;
        keyboard
    }
}
